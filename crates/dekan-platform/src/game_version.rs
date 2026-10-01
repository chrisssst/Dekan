use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const GAME_PATCH_FILE: &str = "game_patch.json";

pub const GAME_EXE: &str = "League of Legends.exe";

const DOS_HEADER_LEN: usize = 64;

const E_LFANEW_OFFSET: usize = 0x3C;

const PE_HEADERS_LEN: usize = 4 + 20;

const TIME_DATE_STAMP_OFFSET: usize = 8;

const MAX_E_LFANEW: u32 = 64 * 1024;

#[derive(Debug, Error)]
pub enum GameVersionError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },

    #[error("file truncated before the {0}")]
    Truncated(&'static str),
    /// The file does not start with `MZ`.
    #[error("not a PE file: DOS signature is not MZ")]
    BadDosSignature,
    /// `e_lfanew` does not point at `PE\0\0`.
    #[error("not a PE file: no PE signature at offset {0:#x}")]
    BadPeSignature(u32),
    /// `e_lfanew` points outside any plausible header area.
    #[error("PE header offset {0:#x} is out of range")]
    HeaderOffsetOutOfRange(u32),
    /// The recorded build file exists but is not valid JSON of the expected shape.
    #[error("{path} is malformed: {source}")]
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// The recorded build could not be written.
    #[error("could not record the game build: {0}")]
    Save(#[from] crate::error::PlatformError),
}

/// A build of the game, as recorded in [`GAME_PATCH_FILE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameBuild {
    /// PE COFF `TimeDateStamp` of the game executable.
    pub time_date_stamp: u32,
}

/// What comparing the installed build with the recorded one found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildCheck {
    /// Nothing was recorded before; the current build is now.
    FirstSeen { new: u32 },
    /// Same build as last run.
    Unchanged { stamp: u32 },
    /// The game was updated (or rolled back) since the last run; the new build is now recorded.
    Changed { old: u32, new: u32 },
}

/// Read the COFF `TimeDateStamp` of the executable at `path`, reading only its headers.
pub fn read_time_date_stamp(path: &Path) -> Result<u32, GameVersionError> {
    let mut file = std::fs::File::open(path).map_err(|e| GameVersionError::Io {
        context: format!("could not open {}", path.display()),
        source: e,
    })?;
    time_date_stamp_from(&mut file)
}

/// Read the COFF `TimeDateStamp` from a PE image: the DOS header, then the PE signature and COFF
/// header it points at. Every offset is checked before it is used.
pub fn time_date_stamp_from<R: Read + Seek>(reader: &mut R) -> Result<u32, GameVersionError> {
    let mut dos = [0u8; DOS_HEADER_LEN];
    read_exact_or_truncated(reader, &mut dos, "DOS header")?;
    if &dos[..2] != b"MZ" {
        return Err(GameVersionError::BadDosSignature);
    }
    let e_lfanew =
        u32_le(&dos, E_LFANEW_OFFSET).ok_or(GameVersionError::Truncated("DOS header"))?;
    if e_lfanew < DOS_HEADER_LEN as u32 || e_lfanew > MAX_E_LFANEW {
        return Err(GameVersionError::HeaderOffsetOutOfRange(e_lfanew));
    }

    reader
        .seek(SeekFrom::Start(u64::from(e_lfanew)))
        .map_err(|e| GameVersionError::Io {
            context: format!("could not seek to the PE header at {e_lfanew:#x}"),
            source: e,
        })?;
    let mut pe = [0u8; PE_HEADERS_LEN];
    read_exact_or_truncated(reader, &mut pe, "COFF header")?;
    if &pe[..4] != b"PE\0\0" {
        return Err(GameVersionError::BadPeSignature(e_lfanew));
    }
    u32_le(&pe, TIME_DATE_STAMP_OFFSET).ok_or(GameVersionError::Truncated("COFF header"))
}

/// Load the build recorded by the last run. `None` when nothing was recorded yet.
pub fn load(state_dir: &Path) -> Result<Option<GameBuild>, GameVersionError> {
    let path = state_dir.join(GAME_PATCH_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(GameVersionError::Io {
                context: format!("could not read {}", path.display()),
                source: e,
            });
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| GameVersionError::Malformed { path, source })
}

/// Record `build` as the last one seen.
pub fn save(state_dir: &Path, build: GameBuild) -> Result<(), GameVersionError> {
    let json = serde_json::to_vec(&build).map_err(|source| GameVersionError::Malformed {
        path: state_dir.join(GAME_PATCH_FILE),
        source,
    })?;
    crate::fs::atomic_write(&state_dir.join(GAME_PATCH_FILE), &json, true)?;
    Ok(())
}

/// Compare the build in `game_dir` with the recorded one, and record it when it differs.
///
/// A malformed record is treated as no record: it is overwritten with the current build, because
/// refusing to record forever would hide every later patch.
pub fn check(state_dir: &Path, game_dir: &Path) -> Result<BuildCheck, GameVersionError> {
    let current = read_time_date_stamp(&game_dir.join(GAME_EXE))?;
    let previous = match load(state_dir) {
        Ok(previous) => previous,
        Err(GameVersionError::Malformed { .. }) => None,
        Err(e) => return Err(e),
    };
    let outcome = match previous {
        Some(GameBuild { time_date_stamp }) if time_date_stamp == current => {
            return Ok(BuildCheck::Unchanged { stamp: current });
        }
        Some(GameBuild { time_date_stamp }) => BuildCheck::Changed {
            old: time_date_stamp,
            new: current,
        },
        None => BuildCheck::FirstSeen { new: current },
    };
    save(
        state_dir,
        GameBuild {
            time_date_stamp: current,
        },
    )?;
    Ok(outcome)
}

fn read_exact_or_truncated<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    what: &'static str,
) -> Result<(), GameVersionError> {
    reader.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            GameVersionError::Truncated(what)
        } else {
            GameVersionError::Io {
                context: format!("could not read the {what}"),
                source: e,
            }
        }
    })
}

fn u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    bytes
        .get(offset..end)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const STAMP: u32 = 0x68D1_2A3B;
    const LFANEW: u32 = 0x80;

    fn synthetic_pe(stamp: u32) -> Vec<u8> {
        let mut image = vec![0u8; LFANEW as usize + PE_HEADERS_LEN];
        image[..2].copy_from_slice(b"MZ");
        image[E_LFANEW_OFFSET..E_LFANEW_OFFSET + 4].copy_from_slice(&LFANEW.to_le_bytes());
        let pe = LFANEW as usize;
        image[pe..pe + 4].copy_from_slice(b"PE\0\0");
        image[pe + 4..pe + 6].copy_from_slice(&0x8664u16.to_le_bytes());
        image[pe + 8..pe + 12].copy_from_slice(&stamp.to_le_bytes());
        image
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dekan_game_version_{tag}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: the fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir
    }

    #[test]
    fn test_a_valid_image_yields_its_time_date_stamp() {
        let stamp = time_date_stamp_from(&mut Cursor::new(synthetic_pe(STAMP))).expect("valid PE");
        assert_eq!(stamp, STAMP);
    }

    #[test]
    fn test_a_truncated_dos_header_is_a_typed_error() {
        let image = synthetic_pe(STAMP);
        let err = time_date_stamp_from(&mut Cursor::new(image[..40].to_vec()))
            .expect_err("truncated DOS header");
        assert!(matches!(err, GameVersionError::Truncated("DOS header")));
    }

    #[test]
    fn test_a_truncated_coff_header_is_a_typed_error() {
        let image = synthetic_pe(STAMP);
        let cut = LFANEW as usize + 6;
        let err = time_date_stamp_from(&mut Cursor::new(image[..cut].to_vec()))
            .expect_err("truncated COFF header");
        assert!(matches!(err, GameVersionError::Truncated("COFF header")));
    }

    #[test]
    fn test_a_bad_dos_signature_is_refused() {
        let mut image = synthetic_pe(STAMP);
        image[..2].copy_from_slice(b"ZM");
        let err = time_date_stamp_from(&mut Cursor::new(image)).expect_err("bad MZ");
        assert!(matches!(err, GameVersionError::BadDosSignature));
    }

    #[test]
    fn test_a_bad_pe_signature_is_refused() {
        let mut image = synthetic_pe(STAMP);
        image[LFANEW as usize..LFANEW as usize + 4].copy_from_slice(b"NE\0\0");
        let err = time_date_stamp_from(&mut Cursor::new(image)).expect_err("bad PE signature");
        assert!(matches!(err, GameVersionError::BadPeSignature(LFANEW)));
    }

    #[test]
    fn test_an_out_of_range_header_offset_is_refused_without_seeking() {
        let mut image = synthetic_pe(STAMP);
        image[E_LFANEW_OFFSET..E_LFANEW_OFFSET + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = time_date_stamp_from(&mut Cursor::new(image.clone())).expect_err("huge offset");
        assert!(matches!(
            err,
            GameVersionError::HeaderOffsetOutOfRange(u32::MAX)
        ));

        image[E_LFANEW_OFFSET..E_LFANEW_OFFSET + 4].copy_from_slice(&4u32.to_le_bytes());
        let err = time_date_stamp_from(&mut Cursor::new(image)).expect_err("offset inside DOS");
        assert!(matches!(err, GameVersionError::HeaderOffsetOutOfRange(4)));
    }

    #[test]
    fn test_the_build_is_recorded_then_compared() {
        let state = temp_dir("state");
        let game = temp_dir("game");
        std::fs::write(game.join(GAME_EXE), synthetic_pe(STAMP)).expect("game exe");

        assert_eq!(
            check(&state, &game).expect("first"),
            BuildCheck::FirstSeen { new: STAMP }
        );
        assert_eq!(
            check(&state, &game).expect("second"),
            BuildCheck::Unchanged { stamp: STAMP }
        );

        std::fs::write(game.join(GAME_EXE), synthetic_pe(STAMP + 1)).expect("patched exe");
        assert_eq!(
            check(&state, &game).expect("after patch"),
            BuildCheck::Changed {
                old: STAMP,
                new: STAMP + 1
            }
        );
        assert_eq!(
            load(&state).expect("load"),
            Some(GameBuild {
                time_date_stamp: STAMP + 1
            })
        );

        let _ = std::fs::remove_dir_all(&state); // ignore-ok: test temp dir teardown
        let _ = std::fs::remove_dir_all(&game); // ignore-ok: test temp dir teardown
    }

    #[test]
    fn test_a_malformed_record_is_replaced_not_fatal() {
        let state = temp_dir("malformed");
        let game = temp_dir("malformed_game");
        std::fs::write(state.join(GAME_PATCH_FILE), b"{not json").expect("record");
        std::fs::write(game.join(GAME_EXE), synthetic_pe(STAMP)).expect("game exe");

        assert!(matches!(
            load(&state),
            Err(GameVersionError::Malformed { .. })
        ));
        assert_eq!(
            check(&state, &game).expect("check"),
            BuildCheck::FirstSeen { new: STAMP }
        );

        let _ = std::fs::remove_dir_all(&state); // ignore-ok: test temp dir teardown
        let _ = std::fs::remove_dir_all(&game); // ignore-ok: test temp dir teardown
    }

    #[test]
    fn test_the_record_format_is_stable() {
        let json = serde_json::to_string(&GameBuild {
            time_date_stamp: 42,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"time_date_stamp":42}"#);
    }
}
