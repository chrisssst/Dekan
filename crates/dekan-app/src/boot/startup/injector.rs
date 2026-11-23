use tracing::{info, warn};

use crate::trigger;

#[cfg_attr(target_os = "macos", allow(unused_variables))]
pub fn injector_unusable(paths: &trigger::ResolvedPaths) -> bool {
    #[cfg(target_os = "macos")]
    {
        info!(
            dekan_version = %dekan_platform::version::display_version(),
            "Built-in macOS Mach-O engine ready"
        );
        return false;
    }
    #[cfg(not(target_os = "macos"))]
    {
        use dekan_app::startup::{InjectorRefusal, injector_refusal};

        let text = dekan_platform::i18n::text();
        let (title, body) = match injector_refusal(&paths.native_host_exe, &paths.native_dll_path) {
            None => {
                info!(
                    host = %paths.native_host_exe.display(),
                    dll = %paths.native_dll_path.display(),
                    dekan_version = %dekan_platform::version::display_version(),
                    "Native engine found and accepted"
                );
                warn_if_manager_outdated(text);
                return false;
            }
            Some(InjectorRefusal::Missing) => {
                warn!(
                    tools = %paths.tools_dir.display(),
                    "Dekan stopped at startup: the native engine is missing from the tools folder"
                );
                (text.missing_tools_title, text.missing_tools_body)
            }
            Some(InjectorRefusal::NotTrusted(files)) => {
                for (file, error) in &files {
                    warn!(
                        file = %file.display(),
                        error = %error,
                        "Dekan stopped at startup: a native engine file is not signed by Dekan's publisher"
                    );
                }
                (text.broken_tools_title, text.broken_tools_body)
            }
        };
        dekan_platform::shell::message_box_warning(title, body);
        true
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn warn_if_manager_outdated(text: &dekan_platform::i18n::Text) {
    use dekan_inject::ltf::{MIN_MANAGER_VERSION, ManagerInstallState, check_manager_install};
    use dekan_platform::i18n::fill;
    use dekan_platform::version::{display_version, format_version};

    match check_manager_install() {
        ManagerInstallState::Ready { path, version } => {
            let ver = version.map_or_else(|| "unknown".into(), format_version);
            info!(
                manager = %path.display(),
                manager_version = %ver,
                dekan_version = %display_version(),
                "Mod Manager install verified"
            );
        }
        ManagerInstallState::Outdated { path, found } => {
            let found_str = format_version(found);
            let req_str = format_version(MIN_MANAGER_VERSION);
            let dekan_ver = display_version();
            warn!(
                manager = %path.display(),
                found = %found_str,
                required = %req_str,
                dekan = %dekan_ver,
                "Mod Manager is outdated"
            );
            let body = fill(
                &fill(
                    &fill(text.manager_outdated_body, "found", &found_str),
                    "dekan",
                    dekan_ver,
                ),
                "required",
                &req_str,
            );
            dekan_platform::shell::message_box_warning(text.manager_outdated_title, &body);
        }
        ManagerInstallState::Missing => {
            let desktop_roaming = dekan_platform::user_profile::local_app_data()
                .ok()
                .and_then(|p| p.parent().map(|a| a.join("Roaming")));
            let appdata_exists = desktop_roaming
                .into_iter()
                .chain(std::env::var("APPDATA").ok().map(std::path::PathBuf::from))
                .any(|dir| dir.join("dev.leaguetoolkit.manager").is_dir());
            let env_override = std::env::var("DEKAN_LTK_MANAGER")
                .or_else(|_| std::env::var("LTK_MANAGER_EXE"))
                .ok();
            warn!(
                appdata_profile_exists = appdata_exists,
                env_override_set = env_override.is_some(),
                "Mod Manager (ltk-manager.exe) was not found on this PC"
            );
            dekan_platform::shell::message_box_warning(
                text.manager_missing_title,
                text.manager_missing_body,
            );
        }
    }
}
