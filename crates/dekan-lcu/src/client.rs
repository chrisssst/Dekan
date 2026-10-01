use std::time::Duration;

use dekan_core::phase::GamePhase;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use tracing::{debug, warn};

use crate::champ_select::ChampSelectSession;
use crate::error::LcuError;
use crate::live_selection::{CurrentSummoner, GameflowSession};
use crate::lockfile::Lockfile;

pub const DEFAULT_LCU_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct LcuClient {
    client: reqwest::Client,
    base_url: String,
}

impl LcuClient {
    pub fn new(lockfile: &Lockfile, timeout: Duration) -> Result<Self, LcuError> {
        let mut headers = HeaderMap::new();
        let auth_val = HeaderValue::from_str(&lockfile.basic_auth_header())
            .map_err(|e| LcuError::Parse(e.to_string()))?;
        headers.insert(AUTHORIZATION, auth_val);

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .default_headers(headers)
            .timeout(timeout)
            .build()?;

        debug!(
            base_url = %lockfile.base_url(),
            timeout_ms = timeout.as_millis(),
            "LCU REST client built"
        );

        Ok(Self {
            client,
            base_url: lockfile.base_url(),
        })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, LcuError> {
        let url = format!("{}{path}", self.base_url);
        let started = std::time::Instant::now();

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,

            Err(e) => {
                warn!(
                    path = path,
                    elapsed_ms = started.elapsed().as_millis(),
                    timeout = e.is_timeout(),
                    error = %e,
                    "LCU request failed before a response arrived"
                );
                return Err(e.into());
            }
        };

        let status = resp.status();
        if !status.is_success() {
            warn!(
                path = path,
                status = status.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "LCU answered with a non-success status"
            );
            return Err(LcuError::Parse(format!(
                "{path} unavailable (HTTP {})",
                status.as_u16()
            )));
        }

        match resp.json::<T>().await {
            Ok(value) => {
                debug!(
                    path = path,
                    status = status.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "LCU request succeeded"
                );
                Ok(value)
            }

            Err(e) => {
                warn!(
                    path = path,
                    status = status.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %e,
                    "LCU response could not be deserialized"
                );
                Err(e.into())
            }
        }
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.client
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn get_gameflow_phase(&self) -> Result<GamePhase, LcuError> {
        let raw_phase: String = match self.get_json("/lol-gameflow/v1/gameflow-phase").await {
            Ok(phase) => phase,

            Err(e) => {
                debug!(error = %e, "Gameflow phase unavailable; treating it as None");
                return Ok(GamePhase::None);
            }
        };

        Ok(GamePhase::from_lcu(&raw_phase).unwrap_or_else(|| {
            warn!(raw = %raw_phase, "Unmapped LCU gameflow phase; treating it as None");
            GamePhase::None
        }))
    }

    pub async fn get_champ_select_session(&self) -> Result<ChampSelectSession, LcuError> {
        self.get_json("/lol-champ-select/v1/session").await
    }

    pub async fn get_gameflow_session(&self) -> Result<GameflowSession, LcuError> {
        self.get_json("/lol-gameflow/v1/session").await
    }

    pub async fn get_current_summoner(&self) -> Result<CurrentSummoner, LcuError> {
        self.get_json("/lol-summoner/v1/current-summoner").await
    }

    pub async fn get_region_locale(&self) -> Result<String, LcuError> {
        #[derive(serde::Deserialize)]
        struct RegionLocale {
            #[serde(default)]
            locale: String,
        }
        let value: RegionLocale = self.get_json("/riotclient/region-locale").await?;
        Ok(value.locale)
    }

    /// Accept the match found. `Ok(false)` means the client refused it (e.g. the ready check
    /// already ended), which is not an error for the caller.
    pub async fn accept_ready_check(&self) -> Result<bool, LcuError> {
        let path = "/lol-matchmaking/v1/ready-check/accept";
        let url = format!("{}{path}", self.base_url);
        let resp = self.client.post(&url).send().await?;
        let status = resp.status();
        debug!(
            path,
            status = status.as_u16(),
            "LCU ready-check accept answered"
        );
        Ok(status.is_success())
    }

    async fn patch_selected_skin(&self, path: &str, skin_id: u32) -> Result<bool, LcuError> {
        let url = format!("{}{path}", self.base_url);
        let started = std::time::Instant::now();
        let payload = serde_json::json!({ "selectedSkinId": skin_id });

        let resp = match self.client.patch(&url).json(&payload).send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(
                    path,
                    skin_id,
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %e,
                    "LCU skin PATCH failed before a response arrived"
                );
                return Err(e.into());
            }
        };

        let status = resp.status();
        debug!(
            path,
            skin_id,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "LCU skin PATCH answered"
        );
        Ok(status.is_success())
    }

    pub async fn set_action_skin(&self, action_id: i64, skin_id: u32) -> Result<bool, LcuError> {
        self.patch_selected_skin(
            &format!("/lol-champ-select/v1/session/actions/{action_id}"),
            skin_id,
        )
        .await
    }

    pub async fn set_my_selection_skin(&self, skin_id: u32) -> Result<bool, LcuError> {
        self.patch_selected_skin("/lol-champ-select/v1/session/my-selection", skin_id)
            .await
    }

    pub async fn get_owned_skin_ids(&self) -> Result<std::collections::HashSet<u32>, LcuError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct InventoryItem {
            #[serde(default)]
            item_id: Option<i64>,
        }

        let items: Vec<InventoryItem> = self
            .get_json("/lol-inventory/v2/inventory/CHAMPION_SKIN")
            .await?;
        let total = items.len();
        let owned: std::collections::HashSet<u32> = items
            .into_iter()
            .filter_map(|item| item.item_id.and_then(|id| u32::try_from(id).ok()))
            .filter(|id| *id > 0)
            .collect();

        if owned.len() != total {
            debug!(
                total,
                usable = owned.len(),
                "Some inventory items had no usable skin id"
            );
        }
        Ok(owned)
    }
}
