#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr = format!("{host}:{port}");

    let listener = TcpListener::bind(&addr).await?;
    info!("Dekan Party Relay listening on http://{addr} (ws://{addr}/room?key=...)");

    let server = Arc::new(dekan_relay::RelayServer::new());
    let cancel = CancellationToken::new();

    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await; // ignore-ok: shutdown signal
        info!("Shutdown signal received, closing relay...");
        cancel_for_signal.cancel();
    });

    server.run(listener, cancel).await;
    info!("Relay server stopped cleanly.");
    Ok(())
}
