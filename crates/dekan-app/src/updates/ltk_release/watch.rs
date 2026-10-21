use super::*;

#[derive(Debug, Clone, Default)]
pub struct LtkNotice(Arc<Mutex<Option<LtkStatus>>>);

impl LtkNotice {
    #[must_use]
    pub fn status(&self) -> Option<LtkStatus> {
        match self.0.lock() {
            Ok(slot) => slot.clone(),
            Err(e) => {
                debug!(error = %e, "Injector notice lock poisoned; no status is shown");
                None
            }
        }
    }

    fn set(&self, status: LtkStatus) {
        match self.0.lock() {
            Ok(mut slot) => *slot = Some(status),
            Err(e) => warn!(error = %e, "Injector notice lock poisoned; the status is not shown"),
        }
    }
}

pub struct LtkCheck {
    pub state_dir: PathBuf,
    pub installed_dll: PathBuf,
    pub installed: InstalledDllCache,
    pub notice: LtkNotice,
    pub notify: Box<dyn Fn(&str) + Send>,
}

async fn pause(token: &CancellationToken, duration: Duration) -> bool {
    tokio::select! {
        _ = token.cancelled() => false,
        () = tokio::time::sleep(duration) => true,
    }
}

pub async fn run(check: LtkCheck, state_rx: StateReceiver, token: CancellationToken) {
    let client = match http_client() {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, "Injector check off: the HTTP client could not be created");
            return;
        }
    };
    let mut verdicts = load_verdicts(&check.state_dir);
    if let Some(status) = verdicts.status_from_cache() {
        check.notice.set(status);
    }
    if !pause(&token, FIRST_CHECK_DELAY).await {
        return;
    }
    let mut failure_warned: Option<String> = None;
    loop {
        match refresh(&client, &mut verdicts).await {
            Ok(Some(status)) => {
                save_verdicts(&check.state_dir, &verdicts);
                if check.notice.status().as_ref() != Some(&status) {
                    info!(
                        latest = %status.latest,
                        latest_trusted = status.latest_trusted,
                        compatible = status.compatible.as_deref().unwrap_or("none"),
                        "LTK Manager releases checked against the publisher's signature"
                    );
                    check.notice.set(status.clone());
                }
                let installed = check
                    .installed
                    .read(&check.installed_dll)
                    .map(|dll| dll.sha256);
                if installed.is_none() && check.installed_dll.is_file() {
                    warn!(file = %check.installed_dll.display(), "The installed patcher DLL could not be read; an injector update is offered without comparing it");
                }
                if let Some(version) = status.offers_update_over(installed.as_deref()) {
                    if !already_notified(&check.state_dir, version) {
                        while is_busy(state_rx.borrow().phase) {
                            if !pause(&token, BUSY_RETRY).await {
                                return;
                            }
                        }
                        (check.notify)(version);
                        remember_notified(&check.state_dir, version);
                    }
                }
            }
            Ok(None) => {
                save_verdicts(&check.state_dir, &verdicts);
                debug!("No published LTK Manager release could be classified");
            }
            Err(e) => {
                save_verdicts(&check.state_dir, &verdicts);
                debug!(error = %e, "Injector check failed; trying again later");
                if failure_warned.as_deref() != Some(e.as_str()) {
                    warn!(error = %e, "Injector check cannot classify the LTK Manager releases; the injector will not be updated until this changes");
                    failure_warned = Some(e);
                }
            }
        }
        if !pause(&token, CHECK_INTERVAL).await {
            return;
        }
    }
}
