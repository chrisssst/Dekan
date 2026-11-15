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
                "Native engine found and accepted"
            );
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
                    "Dekan stopped at startup: a native engine file is not signed by the configured engine publisher"
                );
            }
            (text.broken_tools_title, text.broken_tools_body)
        }
    };
    dekan_platform::shell::message_box_warning(title, body);
    true
}
