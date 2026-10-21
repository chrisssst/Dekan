#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Sender, channel};
use tokio::sync::{RwLock, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub const MAX_MEMBERS: usize = 5;

pub const MAX_MESSAGE_BYTES: usize = 8192;

pub const MAX_CONNECTIONS: usize = 1024;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const MEMBER_QUEUE: usize = 8;

pub const CLOSE_REPLACED: u16 = 4000;

pub const CLOSE_ROOM_FULL: u16 = 4009;

mod room;

pub use room::{ClientInbound, MemberInfo, Room, RoomsMap, SealedBlob, is_valid_room_key};

const HEALTH_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"status\":\"ok\",\"service\":\"dekan-party-relay\"}";

const BAD_REQUEST_RESPONSE: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid room key";

const ROOM_FULL_WARN_EVERY_SECS: u64 = 60;
const ROOM_FULL_RESPONSE: &[u8] = b"HTTP/1.1 409 Conflict\r\nConnection: close\r\n\r\nRoom is full";

const UPGRADE_REQUIRED_RESPONSE: &[u8] = b"HTTP/1.1 426 Upgrade Required\r\nConnection: close\r\n\r\nDekan party relay: WebSocket upgrade required at /room?key=";

pub struct RelayServer {
    rooms: RoomsMap,
}

impl Default for RelayServer {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayServer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn run(self: Arc<Self>, listener: TcpListener, cancel: CancellationToken) {
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let mut accept_failing = false;
        let mut saturated = false;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                res = listener.accept() => {
                    let stream = match res {
                        Ok((stream, _)) => {
                            accept_failing = false;
                            stream
                        }
                        Err(e) => {
                            if !accept_failing {
                                warn!(error = %e, "Accept error; retrying until it clears");
                                accept_failing = true;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        if !saturated {
                            warn!(limit = MAX_CONNECTIONS, "Connection refused: the relay is at its connection limit");
                            saturated = true;
                        }
                        continue;
                    };
                    saturated = false;
                    let server = self.clone();
                    tokio::spawn(async move {
                        let _slot = slot;
                        server.handle_connection(stream).await;
                    });
                }
            }
        }
    }

    async fn handle_connection(&self, mut stream: TcpStream) {
        let mut peek_buf = [0u8; 1024];
        let n = match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut peek_buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            Ok(Ok(_)) => return,
            Ok(Err(e)) => {
                debug!(error = %e, "Connection dropped before sending a request");
                return;
            }
            Err(_) => {
                debug!("Connection sent no request in time");
                return;
            }
        };

        let request_str = String::from_utf8_lossy(&peek_buf[..n]);

        if request_str.starts_with("GET / HTTP/") || request_str.starts_with("GET / ") {
            let mut discard = [0u8; 1024];
            let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
            let _ = stream.write_all(HEALTH_RESPONSE).await; // ignore-ok: client disconnect
            let _ = stream.flush().await; // ignore-ok: client disconnect
            let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
            return;
        }

        let is_websocket = request_str
            .to_ascii_lowercase()
            .contains("upgrade: websocket");
        if !is_websocket {
            debug!("Request refused: WebSocket upgrade required");
            let mut discard = [0u8; 1024];
            let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
            let _ = stream.write_all(UPGRADE_REQUIRED_RESPONSE).await; // ignore-ok: client disconnect
            let _ = stream.flush().await; // ignore-ok: client disconnect
            let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
            return;
        }

        let room_key = Self::extract_room_key(&request_str);
        let Some(key) = room_key else {
            debug!("Request refused: invalid room key");
            let mut discard = [0u8; 1024];
            let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
            let _ = stream.write_all(BAD_REQUEST_RESPONSE).await; // ignore-ok: client disconnect
            let _ = stream.flush().await; // ignore-ok: client disconnect
            let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
            return;
        };

        let existing = self.rooms.read().await.get(&key).cloned();
        if let Some(room) = existing {
            if room.read().await.is_full() {
                note_room_full("before the handshake");
                let mut discard = [0u8; 1024];
                let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
                let _ = stream.write_all(ROOM_FULL_RESPONSE).await; // ignore-ok: client disconnect
                let _ = stream.flush().await; // ignore-ok: client disconnect
                let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
                return;
            }
        }

        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let handshake = tokio_tungstenite::accept_async_with_config(stream, Some(config));
        let ws_stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
            Ok(Ok(ws)) => ws,
            Ok(Err(e)) => {
                debug!(error = %e, "WebSocket handshake failed");
                return;
            }
            Err(_) => {
                debug!("WebSocket handshake timed out");
                return;
            }
        };

        let room_arc = {
            let mut rooms = self.rooms.write().await;
            let opened = !rooms.contains_key(&key);
            let room = rooms
                .entry(key.clone())
                .or_insert_with(|| Arc::new(RwLock::new(Room::new())))
                .clone();
            if opened {
                info!(rooms = rooms.len(), "Relay: room opened");
            }
            room
        };

        debug!(room = %key, "Client joined room");

        self.run_client_session(ws_stream, room_arc, key).await;
    }

    fn extract_room_key(request: &str) -> Option<String> {
        let first_line = request.lines().next()?;
        let path = first_line.split_whitespace().nth(1)?;
        let query = path.split_once('?')?;
        for pair in query.1.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "key" && is_valid_room_key(v) {
                    return Some(v.to_string());
                }
            }
        }
        None
    }

    async fn run_client_session(
        &self,
        ws: tokio_tungstenite::WebSocketStream<TcpStream>,
        room_arc: Arc<RwLock<Room>>,
        room_key: String,
    ) {
        let (mut write, mut read) = ws.split();
        let (tx, mut rx) = channel::<String>(MEMBER_QUEUE);

        let session_id = {
            let mut room = room_arc.write().await;
            if room.is_full() {
                drop(room);
                note_room_full("during the handshake");
                let frame = CloseFrame {
                    code: CloseCode::Library(CLOSE_ROOM_FULL),
                    reason: "room is full".into(),
                };
                let _ = write.send(Message::Close(Some(frame))).await; // ignore-ok: the client is told best effort and retries on its own otherwise
                return;
            }
            room.add_member(tx)
        };

        loop {
            tokio::select! {
                outgoing = rx.recv() => {
                    let Some(payload) = outgoing else {
                        let frame = CloseFrame {
                            code: CloseCode::Library(CLOSE_REPLACED),
                            reason: "replaced by a newer connection".into(),
                        };
                        let _ = write.send(Message::Close(Some(frame))).await; // ignore-ok: the member was already dropped from the room
                        break;
                    };
                    if write.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                incoming = read.next() => {
                    let Some(msg_res) = incoming else { break };
                    let msg = match msg_res {
                        Ok(m) => m,
                        Err(e) => {
                            if matches!(e, tokio_tungstenite::tungstenite::Error::Capacity(_)) {
                                warn!(room = %room_key, error = %e, "Session closed: message over the size limit");
                            } else {
                                debug!(room = %room_key, error = %e, "Session ended with a read error");
                            }
                            break;
                        }
                    };

                    match msg {
                        Message::Text(text) => {
                            if text == "ping" {
                                let _ = write.send(Message::Text("pong".into())).await; // ignore-ok: pong response
                                continue;
                            }
                            let inbound = match serde_json::from_str::<ClientInbound>(&text) {
                                Ok(inbound) => inbound,
                                Err(e) => {
                                    debug!(room = %room_key, error = %e, "Message not understood; ignored");
                                    continue;
                                }
                            };
                            match inbound {
                                ClientInbound::Join { summoner_id, .. } => {
                                    if summoner_id == 0 {
                                        debug!(room = %room_key, "Join ignored: member id is zero");
                                        continue;
                                    }
                                    let replaced = room_arc.write().await.update_join(session_id, summoner_id);
                                    if replaced > 0 {
                                        info!(replaced, "Relay: an older connection of the same member was replaced");
                                    }
                                }
                                ClientInbound::Skin { skin } => {
                                    let mut room = room_arc.write().await;
                                    room.update_skin(session_id, skin);
                                }
                                ClientInbound::Leave => {
                                    let _ = write.close().await; // ignore-ok: close on leave
                                    break;
                                }
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = write.send(Message::Pong(payload)).await; // ignore-ok: pong
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        }

        let is_empty = {
            let mut room = room_arc.write().await;
            room.remove_member(session_id);
            room.is_empty()
        };

        if is_empty {
            let mut rooms = self.rooms.write().await;
            if let Some(r) = rooms.get(&room_key) {
                if r.read().await.is_empty() {
                    rooms.remove(&room_key);
                    info!(rooms = rooms.len(), "Relay: room closed");
                }
            }
        }
    }
}

fn note_room_full(stage: &'static str) {
    static LAST_WARN_SECS: AtomicU64 = AtomicU64::new(0);
    static SUPPRESSED: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let last = LAST_WARN_SECS.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= ROOM_FULL_WARN_EVERY_SECS
        && LAST_WARN_SECS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let suppressed = SUPPRESSED.swap(0, Ordering::Relaxed);
        warn!(
            max_members = MAX_MEMBERS,
            stage, suppressed, "Join refused: the room is full"
        );
    } else {
        SUPPRESSED.fetch_add(1, Ordering::Relaxed);
        debug!(stage, "Join refused: the room is full");
    }
}

#[cfg(test)]
mod tests;
