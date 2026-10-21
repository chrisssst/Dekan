use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

const MAX_BODY: usize = 16 * 1024;
const MAX_ADDRESSES: usize = 4;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
struct DnsResponse {
    #[serde(rename = "Status")]
    status: u32,
    #[serde(default, rename = "TC")]
    truncated: bool,
    #[serde(default, rename = "CD")]
    checking_disabled: bool,
    #[serde(default, rename = "Answer")]
    answers: Vec<Answer>,
}

#[derive(Deserialize)]
struct Answer {
    name: String,
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

pub(super) async fn addresses(host: &str) -> Vec<SocketAddr> {
    let client = match reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(QUERY_TIMEOUT)
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            debug!(%error, "Party: could not create the HTTPS DNS client");
            return Vec::new();
        }
    };
    let (google, cloudflare) = tokio::join!(
        query(&client, "https://dns.google/resolve", host),
        query(&client, "https://cloudflare-dns.com/dns-query", host),
    );
    let mut addresses = Vec::new();
    for result in [google, cloudflare] {
        match result {
            Ok(bytes) => match parse(&bytes, host) {
                Ok(found) => {
                    for address in found {
                        if !addresses.contains(&address) && addresses.len() < MAX_ADDRESSES {
                            addresses.push(address);
                        }
                    }
                }
                Err(error) => debug!(%error, "Party: HTTPS DNS answer was unusable"),
            },
            Err(error) => debug!(%error, "Party: HTTPS DNS request failed"),
        }
    }
    addresses
}

async fn query(client: &reqwest::Client, endpoint: &str, host: &str) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT, "application/dns-json")
        .query(&[
            ("name", host),
            ("type", "A"),
            ("edns_client_subnet", "0.0.0.0/0"),
        ])
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("{error:?}"))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("{error:?}"))?
    {
        if chunk.len() > MAX_BODY.saturating_sub(bytes.len()) {
            return Err("HTTPS DNS response exceeds the size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse(bytes: &[u8], host: &str) -> Result<Vec<SocketAddr>, String> {
    if bytes.len() > MAX_BODY {
        return Err("HTTPS DNS response exceeds the size limit".into());
    }
    let response: DnsResponse = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if response.status != 0 || response.truncated || response.checking_disabled {
        return Err("HTTPS DNS response is unsuccessful, truncated or unchecked".into());
    }
    let mut addresses = Vec::new();
    for answer in response.answers {
        if answer.kind != 1 || !answer.name.trim_end_matches('.').eq_ignore_ascii_case(host) {
            continue;
        }
        let Ok(ip) = answer.data.parse::<Ipv4Addr>() else {
            continue;
        };
        let address = SocketAddr::from((ip, 443));
        if public_ipv4(ip) && !addresses.contains(&address) {
            addresses.push(address);
            if addresses.len() == MAX_ADDRESSES {
                break;
            }
        }
    }
    Ok(addresses)
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..=19).contains(&b)))
}

#[cfg(test)]
#[path = "dns_tests.rs"]
mod tests;
