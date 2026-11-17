use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Response;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error, http};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info};

mod dns;

type Connected = (WebSocketStream<MaybeTlsStream<TcpStream>>, Response);

const SYSTEM_TIMEOUT: Duration = Duration::from_secs(10);
const ADDRESS_TIMEOUT: Duration = Duration::from_secs(4);

pub(super) async fn connect(url: &str, config: WebSocketConfig) -> Result<Connected, Error> {
    let direct = tokio_tungstenite::connect_async_with_config(url, Some(config), false);
    let failed = match tokio::time::timeout(SYSTEM_TIMEOUT, direct).await {
        Ok(Ok(connected)) => return Ok(connected),
        Ok(Err(error)) => error,
        Err(_) => timed_out(),
    };
    if !should_recover(url, &failed) {
        return Err(failed);
    }
    recover(url, config, failed).await
}

fn should_recover(url: &str, error: &Error) -> bool {
    let Ok(request) = url.into_client_request() else {
        return false;
    };
    let Ok(default) = super::config::DEFAULT_RELAY_URL.parse::<http::Uri>() else {
        return false;
    };
    request.uri().scheme_str() == Some("wss")
        && request.uri().host() == default.host()
        && request.uri().port_u16().unwrap_or(443) == 443
        && matches!(error, Error::Io(_))
}

async fn recover(url: &str, config: WebSocketConfig, original: Error) -> Result<Connected, Error> {
    let request = url.into_client_request()?;
    let Some(host) = request.uri().host() else {
        return Err(original);
    };
    debug!(
        relay_host = host,
        "Party: trying HTTPS DNS after a network connection failure"
    );
    let addresses = dns::addresses(host).await;
    if addresses.is_empty() {
        return Err(original);
    }
    connect_addresses(url, config, &addresses).await
}

async fn connect_addresses(
    url: &str,
    config: WebSocketConfig,
    addresses: &[SocketAddr],
) -> Result<Connected, Error> {
    let mut last = Error::Io(io::Error::new(
        io::ErrorKind::NotFound,
        "no relay addresses",
    ));
    for address in addresses {
        let attempt = async {
            let socket = TcpStream::connect(address).await.map_err(Error::Io)?;
            tokio_tungstenite::client_async_tls_with_config(url, socket, Some(config), None).await
        };
        match tokio::time::timeout(ADDRESS_TIMEOUT, attempt).await {
            Ok(Ok(connected)) => {
                info!(relay_address = %address, "Party: connected using an alternate DNS address");
                return Ok(connected);
            }
            Ok(Err(Error::Io(error))) => {
                debug!(relay_address = %address, error = %error, "Party: alternate DNS address unreachable");
                last = Error::Io(error);
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => last = timed_out(),
        }
    }
    Err(last)
}

fn timed_out() -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::TimedOut,
        "relay connection timed out",
    ))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
