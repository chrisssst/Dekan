use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use serde_json::Value;

use super::characters::wad_stamp;

static BUILDING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
static FREED: Condvar = Condvar::new();

pub(crate) struct Claim(PathBuf);

impl Claim {
    pub(crate) fn wait_for(dir: &Path) -> Self {
        let mut building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
        while building.contains(dir) {
            building = FREED.wait(building).unwrap_or_else(PoisonError::into_inner);
        }
        building.insert(dir.to_path_buf());
        Self(dir.to_path_buf())
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        BUILDING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
        FREED.notify_all();
    }
}

pub(crate) fn generator_stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| {
        let exe = std::env::current_exe()
            .map(|path| wad_stamp(&path))
            .unwrap_or_default();
        format!("{}+{exe}", env!("CARGO_PKG_VERSION"))
    })
}

pub(crate) fn already_built(dir: &Path, key: &Value) -> bool {
    let Ok(bytes) = std::fs::read(dir.join("META").join("manifest.json")) else {
        return false;
    };
    let Ok(Value::Object(manifest)) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    dir.join("WAD").is_dir()
        && key
            .as_object()
            .is_some_and(|key| key.iter().all(|(k, v)| manifest.get(k) == Some(v)))
}
