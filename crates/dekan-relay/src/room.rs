use super::*;

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
    sender: Sender<String>,
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
            let _ = session.sender.try_send(snap.clone()); // ignore-ok: a full queue skips this snapshot; the next one carries the whole room
        }
    }

    pub fn add_member(&mut self, sender: Sender<String>) -> usize {
        let id = self.next_session_id;
        self.next_session_id += 1;
        self.members.insert(
            id,
            MemberSession {
                info: None,
                sender: sender.clone(),
            },
        );
        let _ = sender.try_send(self.snapshot()); // ignore-ok: initial snapshot into an empty queue
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
