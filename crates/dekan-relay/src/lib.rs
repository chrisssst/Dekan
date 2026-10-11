#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{Sender, channel};
use tokio::sync::{RwLock, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

pub const MAX_MEMBERS: usize = 5;

pub const MAX_MESSAGE_BYTES: usize = 8192;

pub const MAX_CONNECTIONS: usize = 1024;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const MEMBER_QUEUE: usize = 8;

mod room;

pub use room::{ClientInbound, MemberInfo, Room, RoomsMap, SealedBlob, is_valid_room_key};

const HEALTH_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"status\":\"ok\",\"service\":\"dekan-party-relay\"}";

const BAD_REQUEST_RESPONSE: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid room key";

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
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                res = listener.accept() => {
                    let (stream, peer_addr) = match res {
                        Ok(pair) => pair,
                        Err(e) => {
                            warn!(error = %e, "Accept error");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        debug!(peer = %peer_addr, "Connection refused: the relay is at its connection limit");
                        continue;
                    };
                    let server = self.clone();
                    tokio::spawn(async move {
                        let _slot = slot;
                        if tokio::time::timeout(HANDSHAKE_TIMEOUT * 6, server.handle_connection(stream, peer_addr))
                            .await
                            .is_err()
                        {
                            debug!(peer = %peer_addr, "Connection closed: no complete session in time");
                        }
                    });
                }
            }
        }
    }

    async fn handle_connection(&self, mut stream: TcpStream, peer: std::net::SocketAddr) {
        let mut peek_buf = [0u8; 1024];
        let n = match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut peek_buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return,
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
            let mut discard = [0u8; 1024];
            let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
            let _ = stream.write_all(UPGRADE_REQUIRED_RESPONSE).await; // ignore-ok: client disconnect
            let _ = stream.flush().await; // ignore-ok: client disconnect
            let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
            return;
        }

        let room_key = Self::extract_room_key(&request_str);
        let Some(key) = room_key else {
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
                debug!(peer = %peer, error = %e, "WebSocket handshake failed");
                return;
            }
            Err(_) => {
                debug!(peer = %peer, "WebSocket handshake timed out");
                return;
            }
        };

        let room_arc = {
            let mut rooms = self.rooms.write().await;
            rooms
                .entry(key.clone())
                .or_insert_with(|| Arc::new(RwLock::new(Room::new())))
                .clone()
        };

        debug!(peer = %peer, room = %key, "Client joined room");

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
                let _ = write.close().await; // ignore-ok: the room filled up during the handshake
                return;
            }
            room.add_member(tx)
        };

        loop {
            tokio::select! {
                outgoing = rx.recv() => {
                    let Some(payload) = outgoing else { break };
                    if write.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                incoming = read.next() => {
                    let Some(msg_res) = incoming else { break };
                    let msg = match msg_res {
                        Ok(m) => m,
                        Err(_) => break,
                    };

                    match msg {
                        Message::Text(text) => {
                            if text.len() > MAX_MESSAGE_BYTES {
                                warn!(room = %room_key, "Oversized message dropped");
                                continue;
                            }
                            if text == "ping" {
                                let _ = write.send(Message::Text("pong".into())).await; // ignore-ok: pong response
                                continue;
                            }
                            let Ok(inbound) = serde_json::from_str::<ClientInbound>(&text) else {
                                continue;
                            };
                            match inbound {
                                ClientInbound::Join { summoner_id, .. } => {
                                    if summoner_id > 0 {
                                        let mut room = room_arc.write().await;
                                        room.update_join(session_id, summoner_id);
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
                    debug!(room = %room_key, "Cleaned up empty room");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
