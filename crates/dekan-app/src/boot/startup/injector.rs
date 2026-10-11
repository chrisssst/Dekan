use tracing::{debug, info, warn};

use crate::trigger;

pub async fn compatible_ltk_version(state_dir: &std::path::Path) -> Option<String> {
    let online = dekan_app::update_check::is_enabled(
        std::env::var(dekan_core::env::UPDATE_CHECK).ok().as_deref(),
    );
    let cached = || {
        dekan_app::ltk_release::load_verdicts(state_dir)
            .status_from_cache()
            .and_then(|s| s.compatible)
    };
    if !online {
        return cached();
    }
    match tokio::time::timeout(
        STARTUP_LTK_LOOKUP,
        dekan_app::ltk_release::compatible_version(state_dir),
    )
    .await
    {
        Ok(found) => found,
        Err(_) => {
            debug!("LTK Manager lookup took too long at startup; using the last known result");
            cached()
        }
    }
}

pub const STARTUP_LTK_LOOKUP: std::time::Duration = std::time::Duration::from_secs(10);

pub fn install_injector_elevated(args: &[String]) -> i32 {
    let [staging, tools] = args else {
        return 2;
    };
    let tools = std::path::PathBuf::from(tools);
    let install_dir = dekan_platform::paths::install_dir().ok();
    let exe = std::env::current_exe().ok();
    if !dekan_app::injector_install::is_dekan_tools_folder(
        &tools,
        install_dir.as_deref(),
        exe.as_deref(),
    ) {
        return 3;
    }
    match dekan_app::injector_install::install(std::path::Path::new(staging), &tools) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

pub enum AutoInstall {
    Installed,
    Declined,
    Failed(String),
}

pub async fn install_injector_automatically(
    tools: &std::path::Path,
    state_dir: &std::path::Path,
    version: &str,
) -> AutoInstall {
    use dekan_app::injector_install::{InstallError, elevated_parameters, install, stage};
    use dekan_platform::elevation::{ElevatedRun, run_elevated};

    let staging = match stage(version, state_dir).await {
        Ok(staging) => staging,
        Err(e) => return AutoInstall::Failed(e.to_string()),
    };
    let outcome = match install(&staging, tools) {
        Ok(()) => AutoInstall::Installed,
        Err(InstallError::Denied) => {
            info!(tools = %tools.display(), "Asking Windows for permission to copy the injector into the tools folder");
            let parameters = elevated_parameters(&staging, tools);
            let elevated = match std::env::current_exe() {
                Ok(exe) => tokio::task::spawn_blocking(move || run_elevated(&exe, &parameters))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|run| run.map_err(|e| e.to_string())),
                Err(e) => Err(e.to_string()),
            };
            match elevated {
                Ok(ElevatedRun::Finished(0)) => AutoInstall::Installed,
                Ok(ElevatedRun::Finished(code)) => {
                    AutoInstall::Failed(format!("the elevated copy ended with code {code}"))
                }
                Ok(ElevatedRun::Declined) => AutoInstall::Declined,
                Err(e) => AutoInstall::Failed(e),
            }
        }
        Err(e) => AutoInstall::Failed(e.to_string()),
    };
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        debug!(staging = %staging.display(), error = %e, "Injector staging folder not removed");
    }
    outcome
}

pub async fn injector_unusable(
    paths: &trigger::ResolvedPaths,
    state_dir: &std::path::Path,
) -> bool {
    use dekan_app::startup::{InjectorRefusal, injector_refusal};

    let text = dekan_platform::i18n::text();
    let (title, body) = match injector_refusal(&paths.ltk_host_exe, &paths.ltk_dll_path) {
        None => return false,
        Some(InjectorRefusal::Missing) => {
            warn!(
                tools = %paths.tools_dir.display(),
                "Dekan stopped at startup: the injector files are missing from the tools folder"
            );
            (text.missing_tools_title, text.missing_tools_body)
        }
        Some(InjectorRefusal::NotTrusted(files)) => {
            for (file, error) in &files {
                warn!(
                    file = %file.display(),
                    error = %error,
                    "Dekan stopped at startup: an injector file is not signed by its publisher"
                );
            }
            (text.broken_tools_title, text.broken_tools_body)
        }
    };
    let compatible = compatible_ltk_version(state_dir).await;
    info!(
        compatible = compatible.as_deref().unwrap_or("unknown"),
        "Pointing the user at the newest LTK Manager release with a signed injector"
    );
    let mut failure = None;
    if let Some(version) = compatible.as_deref() {
        let offer = dekan_platform::i18n::fill(text.injector_auto_body, "version", version);
        if dekan_platform::shell::message_box_question(text.injector_auto_title, &offer) {
            match install_injector_automatically(&paths.tools_dir, state_dir, version).await {
                AutoInstall::Installed => {
                    let still_refused = injector_refusal(&paths.ltk_host_exe, &paths.ltk_dll_path);
                    if still_refused.is_none() {
                        info!(version, tools = %paths.tools_dir.display(), "Injector installed from the LTK Manager release on GitHub");
                        return false;
                    }
                    warn!(refusal = ?still_refused, "The installed injector is still refused");
                }
                AutoInstall::Declined => {
                    info!("The user declined the permission to copy the injector")
                }
                AutoInstall::Failed(e) => {
                    warn!(error = %e, "The injector could not be installed automatically");
                    failure = Some(e);
                }
            }
        }
    }
    let version = compatible
        .clone()
        .unwrap_or_else(|| text.ltk_version_unknown.to_owned());
    let body = dekan_platform::i18n::fill(body, "version", &version);
    let body = match failure {
        Some(e) => format!(
            "{}\n\n{body}",
            dekan_platform::i18n::fill(text.injector_auto_failed, "error", &e)
        ),
        None => body,
    };
    let page = dekan_app::ltk_release::release_page(compatible.as_deref());
    if let Err(e) = dekan_platform::shell::open_web_page(&page) {
        warn!(error = %e, page = %page, "Could not open the LTK Manager release page");
    }
    if let Err(e) = std::fs::create_dir_all(&paths.tools_dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            dekan_platform::shell::open_folder(&paths.tools_dir).map_err(|e| e.to_string())
        })
    {
        warn!(tools = %paths.tools_dir.display(), error = %e, "The tools folder could not be opened for the user");
    }
    dekan_platform::shell::message_box_warning(title, &body);
    true
}

pub async fn install_from_panel(
    notice: &dekan_app::ltk_release::LtkNotice,
    tools: &std::path::Path,
    state_dir: &std::path::Path,
    tray: &dekan_platform::tray::TrayController,
) {
    let text = dekan_platform::i18n::text();
    let Some(version) = notice.status().and_then(|s| s.compatible) else {
        warn!(
            "Injector install requested, but no LTK Manager release with a signed injector is known yet"
        );
        return;
    };
    info!(version = %version, "Installing the injector requested from the control panel");
    match install_injector_automatically(tools, state_dir, &version).await {
        AutoInstall::Installed => {
            info!(version = %version, tools = %tools.display(), "Injector installed from the LTK Manager release on GitHub");
            tray.notify(
                &dekan_platform::i18n::fill(text.injector_installed_title, "version", &version),
                text.injector_installed_body,
            );
        }
        AutoInstall::Declined => {
            info!("The user declined the permission to copy the injector");
            tray.notify(text.injector_auto_title, text.injector_install_declined);
        }
        AutoInstall::Failed(e) => {
            warn!(error = %e, version = %version, "The injector could not be installed from the control panel");
            tray.notify(
                text.injector_auto_title,
                &dekan_platform::i18n::fill(text.injector_auto_failed, "error", &e),
            );
        }
    }
}
