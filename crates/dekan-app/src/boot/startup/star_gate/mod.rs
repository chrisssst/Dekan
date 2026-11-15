use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use dekan_platform::i18n;
use dekan_platform::star_gate::GateView;
use tracing::{info, warn};

mod dialog;
mod github;
mod grant;

use github::{Github, GithubError, Star};
use grant::{Freshness, Grant};

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
    let freshness = saved.freshness(now);
    if freshness == Freshness::FromTheFuture {
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
        Err(e) if freshness == Freshness::Fresh => {
            warn!(error = %e, "GitHub is unreachable; StarGate uses the verification from the last 7 days");
            Start::Granted
        }
        Err(e) => {
            warn!(error = %e, "GitHub is unreachable and the saved verification is older than 7 days");
            let view = unreachable_view(&e, true);
            Start::Ask(Some(session), view)
        }
    }
}

pub async fn enforce_star_gate(state_dir: &Path) -> anyhow::Result<bool> {
    let Some(client_id) = github::CLIENT_ID.filter(|id| !id.trim().is_empty()) else {
        warn!("This build has no DEKAN_GITHUB_CLIENT_ID; StarGate refuses to start Dekan");
        let text = i18n::text();
        dekan_platform::shell::message_box_warning(
            text.star_gate_title,
            text.star_gate_unconfigured,
        );
        return Ok(false);
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

#[cfg(test)]
mod tests;
