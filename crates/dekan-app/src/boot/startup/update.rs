use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use dekan_app::self_update::{self, Offer, PendingUpdate, StagedSetup};
use dekan_app::update_check;
use dekan_core::phase::GamePhase;
use dekan_core::state::StateReceiver;
use dekan_platform::i18n;
use dekan_platform::single_instance::SingleInstanceGuard;
use dekan_platform::tray::TrayController;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const DISMISS_RETRY: Duration = Duration::from_millis(200);
const DISMISS_ATTEMPTS: u32 = 25;

#[derive(Clone)]
pub struct UpdateOffer {
    pub tray: TrayController,
    pub state_dir: PathBuf,
    pub state_rx: StateReceiver,
    pub pending: PendingUpdate,
    pub shutdown: CancellationToken,
}

pub struct ReadyInstall {
    setup: StagedSetup,
    _held: std::fs::File,
}

pub async fn offer(ctx: UpdateOffer, offer: Offer) {
    let text = i18n::text();
    let version = offer.version.to_string();
    let installed = std::env::current_exe().is_ok_and(|exe| self_update::is_installed_copy(&exe));
    if !installed {
        info!(
            version,
            "This copy was not installed by the setup; only the download page is offered"
        );
        ctx.tray.notify(
            &i18n::fill(text.update_available_title, "version", &version),
            text.update_available_body,
        );
        update_check::remember_notified(&ctx.state_dir, offer.version);
        return;
    }
    match ask(&ctx, &version).await {
        Some(true) => {}
        Some(false) => {
            info!(version, "The user declined the update");
            update_check::remember_notified(&ctx.state_dir, offer.version);
            return;
        }
        None => {
            info!(
                version,
                "The update question closed unanswered as a match started or Dekan exited; it is asked again later"
            );
            return;
        }
    }
    info!(version, "The user accepted the update");
    if !wait_until(&ctx, self_update::can_install).await {
        return;
    }
    let downloaded = tokio::select! {
        _ = ctx.shutdown.cancelled() => return,
        downloaded = self_update::download(&offer, &ctx.state_dir) => downloaded,
    };
    let setup = match downloaded {
        Ok(setup) => setup,
        Err(e) => {
            warn!(version, error = %e, "The update was not downloaded or failed verification; opening the download page");
            ctx.tray
                .notify(text.update_failed_title, text.update_failed_body);
            if let Err(e) = dekan_platform::shell::open_web_page(&update_check::release_page()) {
                warn!(error = %e, "The download page could not be opened");
            }
            return;
        }
    };
    if !wait_until(&ctx, self_update::can_install).await {
        return;
    }
    info!(version, setup = %setup.path.display(), "Update verified; closing Dekan to install it");
    update_check::remember_notified(&ctx.state_dir, offer.version);
    ctx.pending.set(setup);
    ctx.shutdown.cancel();
}

async fn ask(ctx: &UpdateOffer, version: &str) -> Option<bool> {
    let text = i18n::text();
    let title = i18n::fill(text.update_offer_title, "version", version);
    let body = i18n::fill(text.update_offer_body, "version", version);
    let dialog_title = title.clone();
    let mut question = tokio::task::spawn_blocking(move || {
        dekan_platform::shell::message_box_question(&title, &body)
    });
    let mut state_rx = ctx.state_rx.clone();
    loop {
        tokio::select! {
            answer = &mut question => return Some(answer.unwrap_or(false)),
            _ = ctx.shutdown.cancelled() => break,
            changed = state_rx.changed() => {
                if changed.is_err() || update_check::is_busy(state_rx.borrow().phase) {
                    break;
                }
            }
        }
    }
    for _ in 0..DISMISS_ATTEMPTS {
        if question.is_finished() || dekan_platform::shell::dismiss_question(&dialog_title) {
            break;
        }
        tokio::time::sleep(DISMISS_RETRY).await;
    }
    if tokio::time::timeout(DISMISS_RETRY * DISMISS_ATTEMPTS, question)
        .await
        .is_err()
    {
        warn!("The update question did not close; it stays open until answered");
    }
    None
}

async fn wait_until(ctx: &UpdateOffer, ready: fn(GamePhase) -> bool) -> bool {
    let mut state_rx = ctx.state_rx.clone();
    while !ready(state_rx.borrow_and_update().phase) {
        tokio::select! {
            _ = ctx.shutdown.cancelled() => return false,
            changed = state_rx.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
    true
}

pub fn prepare_install(pending: &PendingUpdate) -> Option<ReadyInstall> {
    let setup = pending.take()?;
    match self_update::open_verified(&setup) {
        Ok(held) => {
            info!(setup = %setup.path.display(), "Starting the installer; it closes this process and opens the new version");
            Some(ReadyInstall { setup, _held: held })
        }
        Err(e) => {
            error!(error = %e, "The downloaded installer no longer matches the signed release; it is not run");
            None
        }
    }
}

pub fn run_installer(install: Option<ReadyInstall>, instance_guard: Option<SingleInstanceGuard>) {
    let Some(install) = install else {
        return;
    };
    drop(instance_guard);
    let mut command = std::process::Command::new(&install.setup.path);
    command.args(self_update::SILENT_INSTALL);
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        command.raw_arg(format!("/DIR=\"{}\"", dir.display()));
    }
    match command.spawn() {
        Ok(_) => {
            info!(
                "Update installer started; Dekan is exiting so the installed files can be replaced"
            );
        }
        Err(e) => {
            error!(error = %e, "Could not start the update installer");
            relaunch();
        }
    }
}

fn relaunch() {
    let _ = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn()); // ignore-ok: the log is already closed; the user reopens Dekan from the Start menu
}
