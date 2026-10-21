use std::time::Duration;

use dekan_core::phase::GamePhase;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use tracing::{debug, warn};

use crate::champ_select::ChampSelectSession;
use crate::error::LcuError;
use crate::live_selection::{CurrentSummoner, GameflowSession};
use crate::lockfile::Lockfile;

pub const DEFAULT_LCU_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct LcuClient {
    client: reqwest::Client,
    base_url: String,
}

impl std::fmt::Debug for LcuClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcuClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl LcuClient {
    pub async fn discover() -> Result<Self, LcuError> {
        let lockfile = tokio::task::spawn_blocking(|| Lockfile::discover(None))
            .await
            .map_err(|e| LcuError::Io(std::io::Error::other(e)))??;
        Self::new(&lockfile, DEFAULT_LCU_TIMEOUT)
    }

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

    pub(crate) async fn fetch(
        &self,
        path: &str,
    ) -> Result<(reqwest::Response, std::time::Instant), LcuError> {
        let url = format!("{}{path}", self.base_url);
        let started = std::time::Instant::now();
        match self.client.get(&url).send().await {
            Ok(resp) => Ok((resp, started)),
            Err(e) => {
                warn!(
                    path = path,
                    elapsed_ms = started.elapsed().as_millis(),
                    timeout = e.is_timeout(),
                    error = %e,
                    "LCU request failed before a response arrived"
                );
                Err(e.into())
            }
        }
    }

    pub(crate) fn log_refusal(
        path: &str,
        status: reqwest::StatusCode,
        started: std::time::Instant,
    ) {
        if status == reqwest::StatusCode::NOT_FOUND {
            debug!(
                path = path,
                elapsed_ms = started.elapsed().as_millis(),
                "LCU resource not found"
            );
        } else {
            warn!(
                path = path,
                status = status.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "LCU answered with a non-success status"
            );
        }
    }

    pub(crate) async fn get_json_opt<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, LcuError> {
        let (resp, started) = self.fetch(path).await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            Self::log_refusal(path, status, started);
            return Ok(None);
        }
        if !status.is_success() {
            Self::log_refusal(path, status, started);
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
                Ok(Some(value))
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

    pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, LcuError> {
        self.get_json_opt(path)
            .await?
            .ok_or_else(|| LcuError::Parse(format!("{path} unavailable (HTTP 404)")))
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.client
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn get_gameflow_phase(&self) -> Result<GamePhase, LcuError> {
        let raw_phase: String = self.get_json("/lol-gameflow/v1/gameflow-phase").await?;
        GamePhase::from_lcu(&raw_phase)
            .ok_or_else(|| LcuError::Parse(format!("unmapped gameflow phase '{raw_phase}'")))
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
            locale: String,
        }
        let value: RegionLocale = self.get_json("/riotclient/region-locale").await?;
        Ok(value.locale)
    }

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

    async fn patch_selected_skin(
        &self,
        path: &str,
        skin_id: u32,
    ) -> Result<reqwest::StatusCode, LcuError> {
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
        Ok(status)
    }

    pub(crate) async fn action_skin_status(
        &self,
        action_id: i64,
        skin_id: u32,
    ) -> Result<reqwest::StatusCode, LcuError> {
        self.patch_selected_skin(
            &format!("/lol-champ-select/v1/session/actions/{action_id}"),
            skin_id,
        )
        .await
    }

    pub(crate) async fn my_selection_skin_status(
        &self,
        skin_id: u32,
    ) -> Result<reqwest::StatusCode, LcuError> {
        self.patch_selected_skin("/lol-champ-select/v1/session/my-selection", skin_id)
            .await
    }

    pub async fn set_action_skin(&self, action_id: i64, skin_id: u32) -> Result<bool, LcuError> {
        Ok(self
            .action_skin_status(action_id, skin_id)
            .await?
            .is_success())
    }

    pub async fn set_my_selection_skin(&self, skin_id: u32) -> Result<bool, LcuError> {
        Ok(self.my_selection_skin_status(skin_id).await?.is_success())
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

        if total > 0 && owned.is_empty() {
            warn!(
                total,
                "The client inventory listed skins but none had a usable id; the skin schema may have changed"
            );
        } else if owned.len() != total {
            debug!(
                total,
                usable = owned.len(),
                "Some inventory items had no usable skin id"
            );
        }
        Ok(owned)
    }
}
