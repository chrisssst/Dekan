use std::path::Path;
use std::time::{Duration, Instant};

use dekan_platform::i18n;
use dekan_platform::star_gate::{GateEvent, GateView, StarGateDialog};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{debug, info, warn};

use super::github::{self, Github, Poll, Star};
use super::{Session, grant, missing_star_view, now_secs, record, unreachable_view};

const MIN_INTERVAL_SECS: u64 = 5;
const MAX_CODE_LIFETIME_SECS: u64 = 1800;
const GRANTED_PAUSE: Duration = Duration::from_millis(600);

enum SignIn {
    Done(Session),
    Failed(GateView),
    Quit,
}

fn notice(message: &str, busy: bool) -> GateView {
    GateView {
        message: message.to_string(),
        busy,
        ..GateView::default()
    }
}

fn failure(message: &str) -> GateView {
    GateView {
        message: message.to_string(),
        is_error: true,
        ..GateView::default()
    }
}

async fn sign_in(
    github: &Github,
    dialog: &StarGateDialog,
    events: &mut UnboundedReceiver<GateEvent>,
) -> SignIn {
    let text = i18n::text();
    dialog.show(notice(text.star_gate_waiting, true));
    let code = match github.request_code().await {
        Ok(code) => code,
        Err(e) => {
            warn!(error = %e, "StarGate could not start the GitHub sign-in");
            return SignIn::Failed(unreachable_view(&e, false));
        }
    };
    if let Err(e) = dekan_platform::clipboard::set_text(&code.user_code) {
        warn!(error = %e, "StarGate could not copy the GitHub code to the clipboard");
    }
    if let Err(e) = dekan_platform::shell::open_web_page(github::trusted_verification_uri(
        &code.verification_uri,
    )) {
        warn!(error = %e, "StarGate could not open the GitHub sign-in page");
    }
    dialog.show(GateView {
        code: code.user_code.clone(),
        ..notice(text.star_gate_waiting, true)
    });

    let deadline =
        Instant::now() + Duration::from_secs(code.expires_in.min(MAX_CODE_LIFETIME_SECS));
    let mut interval = code.interval.max(MIN_INTERVAL_SECS);
    let mut network_warned = false;
    let token = loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(GateEvent::Quit) | None => return SignIn::Quit,
                Some(other) => {
                    debug!(?other, "StarGate ignores this action while it waits for GitHub");
                    continue;
                }
            },
            () = tokio::time::sleep(Duration::from_secs(interval)) => {}
        }
        if Instant::now() >= deadline {
            info!("StarGate GitHub code expired before authorization");
            return SignIn::Failed(failure(text.star_gate_expired));
        }
        match github.poll(&code, interval).await {
            Ok(Poll::Token(token)) => break token,
            Ok(Poll::Pending) => {}
            Ok(Poll::SlowDown(next)) => interval = next.max(interval + MIN_INTERVAL_SECS),
            Ok(Poll::Expired) => return SignIn::Failed(failure(text.star_gate_expired)),
            Ok(Poll::Denied) => {
                info!("StarGate authorization was denied on GitHub");
                return SignIn::Failed(failure(text.star_gate_denied));
            }
            Err(github::GithubError::Network(e)) => {
                if network_warned {
                    debug!(error = %e, "StarGate is still waiting for GitHub to answer");
                } else {
                    warn!(error = %e, "StarGate lost GitHub while waiting for authorization; it keeps trying");
                    network_warned = true;
                }
            }
            Err(e) => {
                warn!(error = %e, "StarGate sign-in was refused by GitHub");
                return SignIn::Failed(unreachable_view(&e, false));
            }
        }
    };
    match github.account(&token).await {
        Ok(account) => SignIn::Done(Session {
            login: account.login,
            user_id: account.id,
            token,
        }),
        Err(e) => {
            warn!(error = %e, "StarGate could not read the GitHub account");
            SignIn::Failed(unreachable_view(&e, false))
        }
    }
}

pub(super) async fn run(
    github: &Github,
    state_dir: &Path,
    mut session: Option<Session>,
    first: GateView,
) -> anyhow::Result<bool> {
    let text = i18n::text();
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let dialog = StarGateDialog::open(events_tx)?;
    dialog.show(first);
    info!("StarGate is waiting for a GitHub sign-in");
    loop {
        match events.recv().await {
            Some(GateEvent::Quit) | None => {
                dialog.close();
                info!("StarGate closed without a confirmed star");
                return Ok(false);
            }
            Some(GateEvent::SignIn) => match sign_in(github, &dialog, &mut events).await {
                SignIn::Done(signed) => session = Some(signed),
                SignIn::Failed(view) => {
                    dialog.show(view);
                    continue;
                }
                SignIn::Quit => {
                    dialog.close();
                    info!("StarGate closed during the GitHub sign-in");
                    return Ok(false);
                }
            },
            Some(GateEvent::Recheck) => {}
        }
        let Some(current) = session.as_ref() else {
            continue;
        };
        dialog.show(notice(text.star_gate_checking, true));
        match github.star(&current.token).await {
            Ok(Star::Given) => {
                record(state_dir, current, now_secs());
                info!("StarGate confirmed the star");
                dialog.show(notice(text.star_gate_granted, true));
                tokio::time::sleep(GRANTED_PAUSE).await;
                dialog.close();
                return Ok(true);
            }
            Ok(Star::Missing) => {
                info!("StarGate found no star for the signed-in account");
                dialog.show(missing_star_view(&current.login));
            }
            Ok(Star::Revoked) => {
                info!("StarGate authorization was revoked on GitHub");
                session = None;
                grant::forget(state_dir);
                dialog.show(failure(text.star_gate_denied));
            }
            Err(e) => {
                warn!(error = %e, "StarGate could not check the star");
                dialog.show(unreachable_view(&e, true));
            }
        }
    }
}
