use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::PlatformError;

pub struct Preference {
    file: &'static str,
    label: &'static str,
    default: bool,
    value: AtomicBool,
}

pub static AUTO_ACCEPT: Preference =
    Preference::new("auto_accept.json", "automatic match accept", false);

pub static RANDOM_SKIN: Preference =
    Preference::new("random_skin.json", "random skin when none is chosen", true);

pub static LIGHT_LOADING: Preference =
    Preference::new("light_loading.json", "light game loading", true);

#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    enabled: bool,
}

impl Preference {
    const fn new(file: &'static str, label: &'static str, default: bool) -> Self {
        Self {
            file,
            label,
            default,
            value: AtomicBool::new(default),
        }
    }

    pub fn load(&self) -> bool {
        match crate::paths::state_dir() {
            Ok(dir) => self.load_in(&dir),
            Err(e) => {
                warn!(
                    error = %e,
                    preference = self.label,
                    default = self.default,
                    "State folder unavailable; the preference keeps its default"
                );
                self.value.store(self.default, Ordering::Relaxed);
                self.default
            }
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.value.load(Ordering::Relaxed)
    }

    pub fn toggle(&self) -> Result<bool, PlatformError> {
        self.toggle_in(&crate::paths::state_dir()?)
    }

    fn load_in(&self, state_dir: &Path) -> bool {
        let enabled = self.read_from(&state_dir.join(self.file));
        self.value.store(enabled, Ordering::Relaxed);
        enabled
    }

    fn toggle_in(&self, state_dir: &Path) -> Result<bool, PlatformError> {
        let next = !self.is_enabled();
        write_to(&state_dir.join(self.file), next, self.label)?;
        self.value.store(next, Ordering::Relaxed);
        Ok(next)
    }

    fn read_from(&self, path: &Path) -> bool {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(stored) => stored.enabled,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        preference = self.label,
                        default = self.default,
                        "Preference file unreadable; the default applies"
                    );
                    self.default
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.default,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    preference = self.label,
                    default = self.default,
                    "Preference file could not be read; the default applies"
                );
                self.default
            }
        }
    }
}

fn write_to(path: &Path, enabled: bool, label: &str) -> Result<(), PlatformError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PlatformError::Io {
            context: format!("failed to create {}", parent.display()),
            source: e,
        })?;
    }
    let json = serde_json::to_vec(&Stored { enabled })
        .map_err(|e| PlatformError::Path(format!("{label}: {e}")))?;
    crate::fs::atomic_write(path, &json, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("dekan_preference_{name}_{}", std::process::id()))
            .join("preference.json")
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir); // ignore-ok: fixture cleanup
        }
    }

    #[test]
    fn test_a_missing_file_means_the_default() {
        let off = Preference::new("x.json", "off by default", false);
        let on = Preference::new("x.json", "on by default", true);
        assert!(!off.read_from(&temp_file("missing_off")));
        assert!(on.read_from(&temp_file("missing_on")));
    }

    #[test]
    fn test_the_setting_round_trips() {
        let preference = Preference::new("x.json", "round trip", true);
        let path = temp_file("roundtrip");
        write_to(&path, false, "round trip").expect("write off");
        assert!(!preference.read_from(&path));
        write_to(&path, true, "round trip").expect("write on");
        assert!(preference.read_from(&path));
        cleanup(&path);
    }

    #[test]
    fn test_garbage_means_the_default() {
        let path = temp_file("garbage");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("fixture dir");
        }
        std::fs::write(&path, b"not json").expect("write garbage");
        assert!(!Preference::new("x.json", "off", false).read_from(&path));
        assert!(Preference::new("x.json", "on", true).read_from(&path));
        cleanup(&path);
    }

    #[test]
    fn test_a_toggle_persists_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("dekan_pref_toggle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        let running = Preference::new("pref.json", "toggle", true);
        assert!(running.load_in(&dir), "nothing stored yet: the default");
        assert!(!running.toggle_in(&dir).expect("toggle off"));
        assert!(!running.is_enabled());

        let restarted = Preference::new("pref.json", "toggle", true);
        assert!(
            !restarted.load_in(&dir),
            "the stored choice wins over the default"
        );
        assert!(restarted.toggle_in(&dir).expect("toggle on"));
        assert!(Preference::new("pref.json", "toggle", false).load_in(&dir));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    fn test_defaults_of_the_shipped_preferences() {
        assert!(
            !AUTO_ACCEPT.default,
            "accepting a match for the user is opt-in"
        );
        assert!(
            RANDOM_SKIN.default,
            "no match starts without a skin by accident"
        );
    }
}
