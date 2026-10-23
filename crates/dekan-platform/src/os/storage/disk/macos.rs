use std::path::Path;

use super::DiskKind;

#[must_use]
pub fn disk_kind(_path: &Path) -> DiskKind {
    DiskKind::Unknown
}
