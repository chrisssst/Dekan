use tracing::{debug, error, info, warn};

pub fn log_profile_resolution() {
    match dekan_platform::user_profile::resolution() {
        dekan_platform::user_profile::Resolution::SameUser { path } => {
            debug!(local_app_data = %path.display(), "Running as the desktop user");
        }
        dekan_platform::user_profile::Resolution::DesktopUser { path, own } => info!(
            local_app_data = %path.display(),
            own_profile = ?own,
            "Running as another account than the desktop user; data goes to the desktop user's profile"
        ),
        dekan_platform::user_profile::Resolution::OwnProfile { path, reason } => warn!(
            local_app_data = %path.display(),
            reason = %reason,
            "Desktop user could not be determined; using this process's own profile"
        ),
        dekan_platform::user_profile::Resolution::Unresolved { reason } => {
            error!(reason = %reason, "No LocalAppData could be resolved");
        }
    }
}

pub fn report_dir_fallback<E: std::fmt::Display>(
    what: &str,
    resolved: &Result<std::path::PathBuf, E>,
    used: &std::path::Path,
) {
    if let Err(e) = resolved {
        warn!(
            folder = what,
            error = %e,
            fallback = %used.display(),
            "Folder could not be resolved; using a temporary one"
        );
    }
}

pub fn report_game_build(
    state_dir: &std::path::Path,
    game_dir: &std::path::Path,
    overlay_dir: &std::path::Path,
) {
    use dekan_platform::game_version::{self, BuildCheck};

    if game_dir.as_os_str().is_empty() {
        debug!("Game build not checked: the game folder is not known yet");
        return;
    }
    match game_version::check(state_dir, game_dir) {
        Ok(BuildCheck::Unchanged { stamp }) => {
            debug!(
                time_date_stamp = stamp,
                "Game build unchanged since the last run"
            );
        }
        Ok(BuildCheck::Changed { old, new }) => {
            info!(
                old,
                new, "Game build changed; invalidating overlay cache and locale (G2, G6)"
            );

            dekan_inject::overlay_cache::OverlayCache::invalidate(overlay_dir);
            dekan_app::catalog::invalidate_locale_cache();
        }
        Ok(BuildCheck::FirstSeen { new }) => {
            info!(new, "Game build recorded for the first time");
        }
        Err(e) => {
            warn!(
                game = %game_version::game_exe(game_dir).display(),
                error = %e,
                "Could not read or record the game build"
            );
        }
    }
}

pub fn report_injector_files(files: &[&std::path::Path]) {
    for file in files {
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(file = %file.display(), error = %e, "Injector file could not be read for the report");
                continue;
            }
        };
        let modified_unix = std::fs::metadata(file)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| age.as_secs());
        info!(
            file = %file.display(),
            bytes = bytes.len(),
            modified_unix = ?modified_unix,
            sha256 = %dekan_inject::dll_validator::compute_sha256(&bytes),
            "Injector file found"
        );
    }
}

pub fn remove_retired_files(state_dir: &std::path::Path) {
    for name in ["bridge.port", "bridge.token", "suspend.lock"] {
        let path = state_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => info!(file = %path.display(), "Removed a file of a retired feature"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                file = %path.display(),
                error = %e,
                "Could not remove a file of a retired feature"
            ),
        }
    }
}

pub fn report_active_injector_files(paths: &crate::trigger::ResolvedPaths) {
    report_injector_files(&[&paths.native_host_exe, &paths.native_dll_path]);
}
