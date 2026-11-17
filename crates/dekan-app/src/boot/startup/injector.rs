use tracing::{info, warn};

use crate::trigger;

pub fn injector_unusable(paths: &trigger::ResolvedPaths) -> bool {
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
            warn!("Mod Manager is not installed on this PC");
            dekan_platform::shell::message_box_warning(
                text.manager_missing_title,
                text.manager_missing_body,
            );
        }
    }
}
