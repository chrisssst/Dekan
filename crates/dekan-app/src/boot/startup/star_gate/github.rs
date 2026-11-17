use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;

const DEFAULT_CLIENT_ID: &str = "Ov23liUlSxiglkxMrJLA";

pub(super) const CLIENT_ID: Option<&str> = match option_env!("DEKAN_GITHUB_CLIENT_ID") {
    Some(id) if !id.is_empty() => Some(id),
    _ => Some(DEFAULT_CLIENT_ID),
};

const REPOSITORY: &str = "chrisssst/Dekan";
const API: &str = "https://api.github.com";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const TIMEOUT: Duration = Duration::from_secs(10);
const SLOW_DOWN_STEP: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(super) struct Account {
    pub login: String,
    pub id: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Poll {
    Token(String),
    Pending,
    SlowDown(u64),
    Expired,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Star {
    Given,
    Missing,
    Revoked,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum GithubError {
    #[error("network error: {0}")]
    Network(String),
    #[error("GitHub rate limit reached")]
    RateLimited,
    #[error("GitHub answered HTTP {0}")]
    Status(u16),
    #[error("unexpected GitHub answer: {0}")]
    Protocol(String),
}

fn network(e: reqwest::Error) -> GithubError {
    GithubError::Network(e.without_url().to_string())
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

pub(super) fn parse_device_code(status: u16, body: &str) -> Result<DeviceCode, GithubError> {
    if !(200..300).contains(&status) {
        return Err(GithubError::Status(status));
    }
    if let Ok(TokenAnswer {
        error: Some(error),
        error_description,
        ..
    }) = serde_json::from_str::<TokenAnswer>(body)
    {
        return Err(GithubError::Protocol(describe(&error, error_description)));
    }
    let code: DeviceCode = serde_json::from_str(body)
        .map_err(|e| GithubError::Protocol(format!("device code: {e}")))?;
    if code.device_code.is_empty() || code.user_code.is_empty() {
        return Err(GithubError::Protocol("device code without codes".into()));
    }
    Ok(code)
}

pub(super) fn parse_poll(status: u16, body: &str, interval: u64) -> Result<Poll, GithubError> {
    if !(200..300).contains(&status) {
        return Err(GithubError::Status(status));
    }
    let answer: TokenAnswer = serde_json::from_str(body)
        .map_err(|e| GithubError::Protocol(format!("token answer: {e}")))?;
    if let Some(token) = answer.access_token.filter(|t| !t.is_empty()) {
        return Ok(Poll::Token(token));
    }
    match answer.error.as_deref() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown(
            answer.interval.unwrap_or(interval + SLOW_DOWN_STEP),
        )),
        Some("expired_token") => Ok(Poll::Expired),
        Some("access_denied") => Ok(Poll::Denied),
        Some(other) => Err(GithubError::Protocol(describe(
            other,
            answer.error_description,
        ))),
        None => Err(GithubError::Protocol(
            "token answer without token or error".into(),
        )),
    }
}

fn describe(error: &str, description: Option<String>) -> String {
    match description {
        Some(text) if !text.is_empty() => format!("{error}: {text}"),
        _ => error.to_string(),
    }
}

pub(super) fn classify_star(
    status: u16,
    rate_remaining: Option<&str>,
) -> Result<Star, GithubError> {
    match status {
        204 => Ok(Star::Given),
        404 => Ok(Star::Missing),
        401 => Ok(Star::Revoked),
        429 => Err(GithubError::RateLimited),
        403 if rate_remaining == Some("0") => Err(GithubError::RateLimited),
        other => Err(GithubError::Status(other)),
    }
}

pub(super) fn trusted_verification_uri(uri: &str) -> &str {
    if uri.starts_with("https://github.com/") {
        uri
    } else {
        "https://github.com/login/device"
    }
}

pub(super) struct Github {
    http: reqwest::Client,
    client_id: &'static str,
}

impl Github {
    pub(super) fn new(client_id: &'static str) -> Result<Self, GithubError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Dekan/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .build()
            .map_err(network)?;
        Ok(Self { http, client_id })
    }

    pub(super) async fn request_code(&self) -> Result<DeviceCode, GithubError> {
        let response = self
            .http
            .post(DEVICE_CODE_URL)
            .header(ACCEPT, "application/json")
            .form(&[("client_id", self.client_id)])
            .send()
            .await
            .map_err(network)?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(network)?;
        parse_device_code(status, &body)
    }

    pub(super) async fn poll(&self, code: &DeviceCode, interval: u64) -> Result<Poll, GithubError> {
        let response = self
            .http
            .post(TOKEN_URL)
            .header(ACCEPT, "application/json")
            .form(&[
                ("client_id", self.client_id),
                ("device_code", code.device_code.as_str()),
                ("grant_type", DEVICE_GRANT),
            ])
            .send()
            .await
            .map_err(network)?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(network)?;
        parse_poll(status, &body, interval)
    }

    fn api(&self, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{API}{path}"))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(AUTHORIZATION, format!("Bearer {token}"))
    }

    pub(super) async fn account(&self, token: &str) -> Result<Account, GithubError> {
        let response = self.api("/user", token).send().await.map_err(network)?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(GithubError::Status(status));
        }
        response
            .json::<Account>()
            .await
            .map_err(|e| GithubError::Protocol(format!("account: {}", e.without_url())))
    }

    pub(super) async fn star(&self, token: &str) -> Result<Star, GithubError> {
        let response = self
            .api(&format!("/user/starred/{REPOSITORY}"), token)
            .send()
            .await
            .map_err(network)?;
        let remaining = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        classify_star(response.status().as_u16(), remaining.as_deref())
    }
}
