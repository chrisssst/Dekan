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

pub fn report_game_build(
    state_dir: &std::path::Path,
    game_dir: &std::path::Path,
    overlay_dir: &std::path::Path,
) -> Option<u32> {
    use dekan_platform::game_version::{self, BuildCheck};

    if game_dir.as_os_str().is_empty() {
        debug!("Game build not checked: the game folder is not known yet");
        return None;
    }
    let stamp = match game_version::check(state_dir, game_dir) {
        Ok(BuildCheck::Unchanged { stamp }) => {
            debug!(
                time_date_stamp = stamp,
                "Game build unchanged since the last run"
            );
            stamp
        }
        Ok(BuildCheck::Changed { old, new }) => {
            info!(
                old,
                new, "Game build changed; invalidating overlay cache and locale (G2, G6)"
            );

            dekan_inject::overlay_cache::OverlayCache::invalidate(overlay_dir);
            dekan_app::catalog::invalidate_locale_cache();
            new
        }
        Ok(BuildCheck::FirstSeen { new }) => {
            info!(new, "Game build recorded for the first time");
            new
        }
        Err(e) => {
            warn!(
                game = %game_version::game_exe(game_dir).display(),
                error = %e,
                "Could not read or record the game build"
            );
            return None;
        }
    };
    Some(stamp)
}

pub fn report_ltk_dll_support(
    stamp: Option<u32>,
    installed: Option<dekan_app::ltk_release::InstalledDll>,
) {
    use dekan_inject::ltk_host::{DllSupport, dll_support};

    let Some(stamp) = stamp else { return };
    let Some(limit) = installed.and_then(|dll| dll.build_limit) else {
        warn!(
            game_build = stamp,
            "The patcher DLL's game build limit could not be read; Dekan cannot tell whether it accepts this build"
        );
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match dll_support(stamp, limit, now) {
        DllSupport::Supported => debug!(
            game_build = stamp,
            dll_limit = limit,
            "The patcher DLL accepts the installed game build"
        ),
        DllSupport::SupportedUntilNextPatch { days_left } => warn!(
            game_build = stamp,
            dll_limit = limit,
            days_left,
            "The patcher DLL accepts this game build, but refuses builds made after its limit: the next game patch needs a refreshed DLL"
        ),
        DllSupport::Refused => error!(
            game_build = stamp,
            dll_limit = limit,
            "The installed game build is newer than the patcher DLL accepts; no skin can load until a refreshed DLL is installed"
        ),
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
