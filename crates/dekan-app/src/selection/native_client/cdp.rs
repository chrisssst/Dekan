use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const LIMIT: usize = 2 * 1024 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(700);

pub(super) struct Connection {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next: u64,
}

pub(super) fn debug_port(command: &str) -> Option<u16> {
    let mut words = command.split_whitespace().map(|s| s.trim_matches('"'));
    while let Some(word) = words.next() {
        let value = if word == "--remote-debugging-port" {
            words.next()
        } else {
            word.strip_prefix("--remote-debugging-port=")
        };
        if let Some(port) = value
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
        {
            return Some(port);
        }
    }
    None
}

fn endpoint(targets: &Value, port: u16) -> Result<String> {
    let pages = targets
        .as_array()
        .context("CDP target list is not an array")?;
    for page in pages {
        if page["type"] != "page"
            || !page["url"]
                .as_str()
                .unwrap_or_default()
                .contains("index.html")
        {
            continue;
        }
        let Some(raw) = page["webSocketDebuggerUrl"].as_str() else {
            continue;
        };
        let url = reqwest::Url::parse(raw)?;
        if url.scheme() == "ws"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path().starts_with("/devtools/page/")
        {
            return Ok(raw.to_owned());
        }
    }
    bail!("No loopback League client page target")
}

impl Connection {
    pub(super) async fn discover(port: u16) -> Result<Self> {
        tokio::time::timeout(TIMEOUT, async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let mut response = client
                .get(format!("http://127.0.0.1:{port}/json"))
                .send()
                .await?
                .error_for_status()?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len().saturating_add(chunk.len()) > LIMIT {
                    bail!("CDP target list too large")
                }
                bytes.extend_from_slice(&chunk);
            }
            let url = endpoint(&serde_json::from_slice(&bytes)?, port)?;
            let config = WebSocketConfig::default()
                .max_message_size(Some(LIMIT))
                .max_frame_size(Some(LIMIT));
            let (socket, _) =
                tokio_tungstenite::connect_async_with_config(url, Some(config), true).await?;
            Ok(Self { socket, next: 0 })
        })
        .await
        .context("CDP discovery timeout")?
    }

    pub(super) async fn evaluate(&mut self, expression: &str) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        tokio::time::timeout(TIMEOUT, async {
            self.socket
                .send(Message::Text(
                    json!({
                        "id": id, "method": "Runtime.evaluate", "params": {
                            "expression": expression, "returnByValue": true, "awaitPromise": true,
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await?;
            for _ in 0..128 {
                let message = self.socket.next().await.context("CDP socket closed")??;
                if let Message::Text(text) = message {
                    let response: Value = serde_json::from_str(&text)?;
                    if response["id"].as_u64() != Some(id) {
                        continue;
                    }
                    if !response["error"].is_null()
                        || !response["result"]["exceptionDetails"].is_null()
                    {
                        bail!("League client rejected the Dekan script")
                    }
                    return response["result"]["result"]
                        .get("value")
                        .cloned()
                        .context("CDP evaluation returned no value");
                }
            }
            bail!("CDP response did not arrive within the event limit")
        })
        .await
        .context("CDP evaluation timeout")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_port_is_exact_and_nonzero() {
        assert_eq!(
            debug_port(r#""LeagueClientUx.exe" --remote-debugging-port=46359 --foo"#),
            Some(46359)
        );
        assert_eq!(debug_port("app --remote-debugging-port 12345"), Some(12345));
        for invalid in [
            "app",
            "--remote-debugging-port=0",
            "--remote-debugging-port=70000",
            "--fake-remote-debugging-port=42",
        ] {
            assert_eq!(debug_port(invalid), None);
        }
    }

    #[test]
    fn only_the_expected_local_page_socket_is_accepted() {
        let target = |url: &str| json!([{"type":"page","url":"https://riot/index.html","webSocketDebuggerUrl":url}]);
        assert!(endpoint(&target("ws://127.0.0.1:1234/devtools/page/1"), 1234).is_ok());
        for bad in [
            "ws://evil.test:1234/devtools/page/1",
            "ws://127.0.0.1:1235/devtools/page/1",
            "ws://u@127.0.0.1:1234/devtools/page/1",
            "ws://127.0.0.1:1234/devtools/browser/1",
        ] {
            assert!(endpoint(&target(bad), 1234).is_err());
        }
    }
}
