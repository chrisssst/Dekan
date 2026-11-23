use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dekan_core::phase::GamePhase;
use dekan_platform::i18n;
use dekan_platform::star_gate::GateView;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

mod dialog;
mod github;
mod grant;

use github::{Github, GithubError, Star};
use grant::Grant;

pub(super) struct Session {
    pub login: String,
    pub user_id: u64,
    pub token: String,
}

impl Session {
    fn from_grant(grant: Grant) -> Self {
        Self {
            login: grant.login,
            user_id: grant.user_id,
            token: grant.token,
        }
    }
}

enum Start {
    Granted,
    Ask(Option<Session>, GateView),
}

fn now_secs() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs(),
        Err(e) => {
            warn!(error = %e, "The system clock is before 1970; StarGate treats it as zero");
            0
        }
    }
}

fn missing_star_view(login: &str) -> GateView {
    GateView {
        message: i18n::fill(i18n::text().star_gate_missing_star, "user", login),
        is_error: true,
        can_recheck: true,
        ..GateView::default()
    }
}

fn unreachable_view(error: &GithubError, can_recheck: bool) -> GateView {
    GateView {
        message: i18n::fill(
            i18n::text().star_gate_unreachable,
            "error",
            &error.to_string(),
        ),
        is_error: true,
        can_recheck,
        ..GateView::default()
    }
}

fn record(state_dir: &Path, session: &Session, now: u64) {
    let grant = Grant {
        login: session.login.clone(),
        user_id: session.user_id,
        token: session.token.clone(),
        verified_at: now,
    };
    if let Err(e) = grant::save(state_dir, &grant) {
        warn!(error = %e, "StarGate could not save the verification; the next start checks GitHub again");
    }
}

async fn revalidate(github: &Github, state_dir: &Path, saved: Grant, now: u64) -> Start {
    if saved.is_from_the_future(now) {
        warn!("StarGate verification is dated in the future; it is discarded");
        grant::forget(state_dir);
        return Start::Ask(None, GateView::default());
    }
    let session = Session::from_grant(saved);
    match github.star(&session.token).await {
        Ok(Star::Given) => {
            record(state_dir, &session, now);
            info!("StarGate confirmed the star online");
            Start::Granted
        }
        Ok(Star::Missing) => {
            info!("StarGate found no star for the saved account");
            let view = missing_star_view(&session.login);
            Start::Ask(Some(session), view)
        }
        Ok(Star::Revoked) => {
            info!("StarGate authorization was revoked on GitHub");
            grant::forget(state_dir);
            Start::Ask(None, GateView::default())
        }
        Err(e) => {
            warn!(error = %e, "GitHub is unreachable; StarGate needs an online check to start Dekan");
            let view = unreachable_view(&e, true);
            Start::Ask(Some(session), view)
        }
    }
}

pub async fn enforce_star_gate(state_dir: &Path) -> anyhow::Result<bool> {
    let Some(client_id) = github::CLIENT_ID.filter(|id| !id.trim().is_empty()) else {
        info!(
            "This build has no DEKAN_GITHUB_CLIENT_ID; StarGate is off and Dekan starts normally"
        );
        return Ok(true);
    };
    let github = Github::new(client_id)?;
    let start = match grant::load(state_dir) {
        Some(saved) => revalidate(&github, state_dir, saved, now_secs()).await,
        None => Start::Ask(None, GateView::default()),
    };
    match start {
        Start::Granted => Ok(true),
        Start::Ask(session, view) => dialog::run(&github, state_dir, session, view).await,
    }
}

async fn star_lost(state_dir: &Path) -> Option<String> {
    let client_id = github::CLIENT_ID.filter(|id| !id.trim().is_empty())?;
    let Some(saved) = grant::load(state_dir) else {
        warn!("StarGate verification disappeared while Dekan was running");
        return Some(String::new());
    };
    let github = match Github::new(client_id) {
        Ok(github) => github,
        Err(e) => {
            warn!(error = %e, "StarGate could not build its GitHub client for the recheck");
            return None;
        }
    };
    match github.star(&saved.token).await {
        Ok(Star::Given) => {
            info!("StarGate confirmed the star again");
            None
        }
        Ok(Star::Missing) => {
            info!("StarGate found the star removed while Dekan was running");
            Some(saved.login)
        }
        Ok(Star::Revoked) => {
            info!("StarGate authorization was revoked while Dekan was running");
            grant::forget(state_dir);
            Some(saved.login)
        }
        Err(e) => {
            warn!(error = %e, "StarGate could not recheck the star; the check repeats when the next match is searched");
            None
        }
    }
}

const STAR_RECHECK_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

pub async fn watch_star(
    state_dir: PathBuf,
    mut state: dekan_core::state::StateReceiver,
    shutdown: CancellationToken,
    token: CancellationToken,
) {
    let mut last = GamePhase::None;
    let mut last_confirmed: Option<std::time::Instant> = None;
    loop {
        tokio::select! {
            () = token.cancelled() => return,
            changed = state.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
        let phase = state.borrow_and_update().phase;
        let entered =
            phase != last && matches!(phase, GamePhase::Matchmaking | GamePhase::ChampSelect);
        last = phase;
        if !entered || last_confirmed.is_some_and(|t| t.elapsed() < STAR_RECHECK_COOLDOWN) {
            continue;
        }
        if let Some(login) = star_lost(&state_dir).await {
            warn!("StarGate closes Dekan: the repository star is required");
            let text = i18n::text();
            dekan_platform::shell::message_box_warning(
                text.star_gate_title,
                &i18n::fill(text.star_gate_missing_star, "user", &login),
            );
            shutdown.cancel();
            return;
        }
        last_confirmed = Some(std::time::Instant::now());
    }
}

#[cfg(test)]
mod tests;
