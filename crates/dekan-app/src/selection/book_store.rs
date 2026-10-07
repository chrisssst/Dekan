use std::path::Path;

use dekan_platform::fs::atomic_write;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

const BOOK_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Persisted<T> {
    version: u32,
    book: T,
}

#[must_use]
pub(crate) fn load<T: DeserializeOwned + Default>(path: &Path, what: &str) -> T {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            warn!(file = %path.display(), what, error = %e, "Saved selections could not be read; starting empty");
            return T::default();
        }
    };

    match serde_json::from_slice::<Persisted<T>>(&bytes) {
        Ok(persisted) => {
            if persisted.version != BOOK_VERSION {
                warn!(
                    file = %path.display(),
                    what,
                    version = persisted.version,
                    expected = BOOK_VERSION,
                    "Saved selections written by another version; reading them as-is"
                );
            }
            persisted.book
        }
        Err(e) => {
            let aside = path.with_extension("json.unreadable");
            let moved = std::fs::rename(path, &aside);
            warn!(
                file = %path.display(),
                what,
                moved_to = %aside.display(),
                moved = moved.is_ok(),
                error = %e,
                "Saved selections file is not valid; it was set aside and starts empty"
            );
            T::default()
        }
    }
}

pub(crate) fn save<T: Serialize>(path: &Path, book: &T, what: &str) {
    let persisted = Persisted {
        version: BOOK_VERSION,
        book,
    };
    let bytes = match serde_json::to_vec_pretty(&persisted) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(what, error = %e, "Saved selections could not be serialized; they will not survive a restart");
            return;
        }
    };
    match atomic_write(path, &bytes, true) {
        Ok(()) => debug!(file = %path.display(), what, "Saved selections written"),
        Err(e) => {
            warn!(file = %path.display(), what, error = %e, "Saved selections could not be written")
        }
    }
}
