use std::path::PathBuf;
use std::sync::Arc;

use dekan_core::phase::GamePhase;
use dekan_core::state::{StateSender, clear_for_new_game, set_lcu_connected, set_phase};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::champ_select::apply_session_to_state;
use crate::client::{DEFAULT_LCU_TIMEOUT, LcuClient};
use crate::error::LcuError;
use crate::lockfile::Lockfile;
use crate::websocket::{
    BackoffManager, LcuEvent, TOPIC_CHAMP_SELECT, TOPIC_GAMEFLOW, dispatch_event_to_state,
    make_subscribe_frame,
};

#[derive(Debug)]
struct DangerSkipServerCertVerifier;

impl rustls::client::danger::ServerCertVerifier for DangerSkipServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn make_lcu_tls_connector() -> Connector {
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(DangerSkipServerCertVerifier))
        .with_no_client_auth();

    config.enable_sni = false;

    Connector::Rustls(Arc::new(config))
}

const IDLE_REPORT_EVERY: u32 = 10;

pub struct LcuObserver {
    explicit_lockfile: Option<PathBuf>,
    state_tx: StateSender,
}

impl LcuObserver {
    pub fn new(state_tx: StateSender, explicit_lockfile: Option<PathBuf>) -> Self {
        Self {
            explicit_lockfile,
            state_tx,
        }
    }

    pub async fn run(&self, token: CancellationToken) {
        let mut backoff = BackoffManager::default();

        let mut waiting_since: Option<std::time::Instant> = None;
        let mut idle_reports: u32 = 0;

        while !token.is_cancelled() {
            let explicit = self.explicit_lockfile.clone();
            let discovery = tokio::task::spawn_blocking(move || {
                Lockfile::discover(explicit.as_deref()).map_err(|e| e.to_string())
            });

            let lock_res = tokio::select! {
                () = token.cancelled() => {
                    info!("LCU observer cancelled during lockfile discovery");
                    break;
                }
                joined = discovery => match joined {
                    Ok(res) => res,
                    Err(e) => {
                        warn!(error = %e, "Lockfile discovery task failed to join");
                        Err(LcuError::LockfileNotFound.to_string())
                    }
                },
            };

            let connected = match lock_res {
                Ok(lockfile) => {
                    info!(
                        pid = lockfile.pid,
                        port = lockfile.port,
                        "League Client lockfile detected. Initiating connection."
                    );

                    let outcome = self.handle_connected_session(&lockfile, &token).await;

                    set_lcu_connected(&self.state_tx, false);
                    clear_for_new_game(&self.state_tx);
                    set_phase(&self.state_tx, GamePhase::None);

                    match outcome {
                        Ok(session_established) => session_established,
                        Err(e) => {
                            warn!(error = %e, pid = lockfile.pid, port = lockfile.port, "LCU session ended or encountered error");
                            false
                        }
                    }
                }
                Err(_) => false,
            };

            if connected {
                backoff.reset();
                waiting_since = None;
                idle_reports = 0;
                continue;
            }

            let delay = backoff.next_delay();

            if waiting_since.is_none() {
                waiting_since = Some(std::time::Instant::now());
                idle_reports = 0;
                info!("League Client not running; waiting for it");
            } else if idle_reports % IDLE_REPORT_EVERY == IDLE_REPORT_EVERY - 1 {
                let waited = waiting_since.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                debug!(
                    waited_s = waited,
                    delay_ms = delay.as_millis(),
                    "Still waiting for the League Client"
                );
            }
            idle_reports = idle_reports.saturating_add(1);

            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = token.cancelled() => {
                    info!("LCU observer loop cancelled during discovery backoff");
                    break;
                }
            }
        }

        info!("LCU observer task finished cleanly");
    }

    async fn handle_connected_session(
        &self,
        lockfile: &Lockfile,
        token: &CancellationToken,
    ) -> Result<bool, LcuError> {
        let rest_client = LcuClient::new(lockfile, DEFAULT_LCU_TIMEOUT)?;
        let initial_phase = tokio::select! {
            () = token.cancelled() => {
                info!("LCU observer cancelled during initial REST sync");
                return Ok(false);
            }
            res = rest_client.get_gameflow_phase() => res,
        };
        match initial_phase {
            Ok(initial_phase) => {
                info!(phase = ?initial_phase, "Initial LCU gameflow phase synced");
                set_phase(&self.state_tx, initial_phase);

                if initial_phase == GamePhase::ChampSelect {
                    if let Ok(session) = rest_client.get_champ_select_session().await {
                        info!("Initial champ select session populated");
                        apply_session_to_state(&self.state_tx, &session);
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Could not fetch initial gameflow phase via REST");
            }
        }

        if let Ok(summoner) = rest_client.get_current_summoner().await {
            if !summoner.puuid.is_empty() {
                debug!(puuid = %summoner.puuid, "Current summoner PUUID initialized on LCU connect");
                let current_team = self.state_tx.borrow().team.clone();
                dekan_core::state::set_team(&self.state_tx, Some(summoner.puuid), current_team);
            }
        }

        let mut req = lockfile
            .ws_url()
            .into_client_request()
            .map_err(|e| LcuError::Parse(e.to_string()))?;

        req.headers_mut().insert(
            "Authorization",
            lockfile
                .basic_auth_header()
                .parse()
                .map_err(|e: reqwest::header::InvalidHeaderValue| LcuError::Parse(e.to_string()))?,
        );

        let connector = make_lcu_tls_connector();
        let connect =
            tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(connector));
        let (mut ws_stream, _) = tokio::select! {
            () = token.cancelled() => {
                info!("LCU observer cancelled during WebSocket connect");
                return Ok(false);
            }
            res = connect => res.map_err(|e| LcuError::WebSocket(e.to_string()))?,
        };

        info!("LCU WebSocket connected successfully");

        let sub_gameflow = make_subscribe_frame(TOPIC_GAMEFLOW);
        let sub_champ_select = make_subscribe_frame(TOPIC_CHAMP_SELECT);

        ws_stream
            .send(Message::Text(sub_gameflow.into()))
            .await
            .map_err(|e| LcuError::WebSocket(e.to_string()))?;

        ws_stream
            .send(Message::Text(sub_champ_select.into()))
            .await
            .map_err(|e| LcuError::WebSocket(e.to_string()))?;

        info!("Subscribed to gameflow and champ-select LCU event streams");
        set_lcu_connected(&self.state_tx, true);

        loop {
            tokio::select! {
                () = token.cancelled() => {
                    info!("LCU session received cancellation signal; closing stream");
                    let _ = ws_stream.close(None).await; // ignore-ok: closing a socket we are done with
                    return Ok(true);
                }
                msg_opt = ws_stream.next() => {
                    match msg_opt {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(Some(event)) = LcuEvent::parse(&text) {
                                dispatch_event_to_state(&self.state_tx, &event);
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = ws_stream.send(Message::Pong(payload)).await; // ignore-ok: a pong that cannot be sent means the socket is gone; the read loop reports it
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("LCU WebSocket received close frame from client");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!(error = %e, "LCU WebSocket read error");
                            break;
                        }
                        None => {
                            info!("LCU WebSocket stream reached EOF (client terminated)");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::state::new_state_channel;
    use std::time::Duration;

    #[tokio::test]
    async fn test_observer_cancellation_during_discovery() {
        let (tx, _rx) = new_state_channel();
        let observer = LcuObserver::new(tx, Some(PathBuf::from("non_existent_lockfile_123.tmp")));

        let token = CancellationToken::new();
        let child_token = token.clone();

        let handle = tokio::spawn(async move {
            observer.run(child_token).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();

        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            res.is_ok(),
            "observer should terminate cleanly on cancellation"
        );
    }
}
