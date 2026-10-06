#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

pub const MAX_MEMBERS: usize = 5;

pub const MAX_MESSAGE_BYTES: usize = 8192;

pub fn is_valid_room_key(key: &str) -> bool {
    key.len() == 32
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedBlob {
    pub v: u32,
    pub n: String,
    pub c: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub summoner_id: u64,
    pub summoner_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<SealedBlob>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientInbound {
    Join {
        summoner_id: u64,
        #[serde(default)]
        summoner_name: String,
    },
    Skin {
        skin: Option<SealedBlob>,
    },
    Leave,
}

struct MemberSession {
    info: Option<MemberInfo>,
    sender: UnboundedSender<String>,
}

pub struct Room {
    members: HashMap<usize, MemberSession>,
    next_session_id: usize,
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    #[must_use]
    pub fn new() -> Self {
        Self {
            members: HashMap::new(),
            next_session_id: 1,
        }
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.members.len() >= MAX_MEMBERS
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn snapshot(&self) -> String {
        let members: Vec<&MemberInfo> = self
            .members
            .values()
            .filter_map(|m| m.info.as_ref())
            .collect();

        serde_json::to_string(&serde_json::json!({
            "type": "members",
            "members": members,
        }))
        .unwrap_or_else(|_| r#"{"type":"members","members":[]}"#.into())
    }

    pub fn broadcast(&self) {
        let snap = self.snapshot();
        for session in self.members.values() {
            let _ = session.sender.send(snap.clone()); // ignore-ok: closed client drops next
        }
    }

    pub fn add_member(&mut self, sender: UnboundedSender<String>) -> usize {
        let id = self.next_session_id;
        self.next_session_id += 1;
        self.members.insert(
            id,
            MemberSession {
                info: None,
                sender: sender.clone(),
            },
        );
        let _ = sender.send(self.snapshot()); // ignore-ok: initial snapshot
        id
    }

    pub fn update_join(&mut self, session_id: usize, summoner_id: u64) {
        if let Some(session) = self.members.get_mut(&session_id) {
            let prev_skin = session.info.as_mut().and_then(|i| i.skin.take());
            session.info = Some(MemberInfo {
                summoner_id,
                summoner_name: String::new(),
                skin: prev_skin,
            });
            self.broadcast();
        }
    }

    pub fn update_skin(&mut self, session_id: usize, skin: Option<SealedBlob>) {
        if let Some(session) = self.members.get_mut(&session_id) {
            if let Some(info) = session.info.as_mut() {
                info.skin = skin;
                self.broadcast();
            }
        }
    }

    pub fn remove_member(&mut self, session_id: usize) {
        if self.members.remove(&session_id).is_some() {
            self.broadcast();
        }
    }
}

pub type RoomsMap = Arc<RwLock<HashMap<String, Arc<RwLock<Room>>>>>;

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
                    let server = self.clone();
                    tokio::spawn(async move {
                        server.handle_connection(stream, peer_addr).await;
                    });
                }
            }
        }
    }

    async fn handle_connection(&self, mut stream: TcpStream, peer: std::net::SocketAddr) {
        let mut peek_buf = [0u8; 1024];
        let n = match stream.peek(&mut peek_buf).await {
            Ok(n) if n > 0 => n,
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

        let room_arc = {
            let mut rooms = self.rooms.write().await;
            rooms
                .entry(key.clone())
                .or_insert_with(|| Arc::new(RwLock::new(Room::new())))
                .clone()
        };

        {
            let room = room_arc.read().await;
            if room.is_full() {
                let mut discard = [0u8; 1024];
                let _ = stream.read(&mut discard).await; // ignore-ok: consume request before response
                let _ = stream.write_all(ROOM_FULL_RESPONSE).await; // ignore-ok: client disconnect
                let _ = stream.flush().await; // ignore-ok: client disconnect
                let _ = stream.shutdown().await; // ignore-ok: clean TCP FIN
                return;
            }
        }

        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                debug!(peer = %peer, error = %e, "WebSocket handshake failed");
                return;
            }
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
        let (tx, mut rx) = unbounded_channel::<String>();

        let session_id = {
            let mut room = room_arc.write().await;
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
mod tests {
    use super::*;

    #[test]
    fn test_valid_room_key() {
        assert!(is_valid_room_key("0123456789abcdef0123456789abcdef"));
        assert!(!is_valid_room_key("short"));
        assert!(!is_valid_room_key("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!is_valid_room_key("0123456789abcdef0123456789abcdefg"));
    }

    #[test]
    fn test_extract_room_key() {
        let req = "GET /room?key=0123456789abcdef0123456789abcdef HTTP/1.1\r\nHost: localhost\r\n";
        assert_eq!(
            RelayServer::extract_room_key(req),
            Some("0123456789abcdef0123456789abcdef".to_string())
        );

        let bad_req = "GET /room?key=badkey HTTP/1.1\r\n";
        assert_eq!(RelayServer::extract_room_key(bad_req), None);
    }
}
