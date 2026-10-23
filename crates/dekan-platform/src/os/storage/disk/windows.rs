use std::path::Path;

use tracing::debug;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
};
use windows::core::{HSTRING, PCWSTR};

use super::DiskKind;

#[must_use]
pub fn disk_kind(path: &Path) -> DiskKind {
    volume_device(path)
        .and_then(|device| seek_penalty(&device))
        .map_or(DiskKind::Unknown, |penalty| {
            if penalty {
                DiskKind::Rotational
            } else {
                DiskKind::SolidState
            }
        })
}

fn volume_device(path: &Path) -> Option<String> {
    let mut mount_point = [0u16; 261];
    let mut volume = [0u16; 64];
    unsafe {
        GetVolumePathNameW(&HSTRING::from(path.as_os_str()), &mut mount_point)
            .inspect_err(|e| debug!(error = %e, "Volume of the path unavailable"))
            .ok()?;
        GetVolumeNameForVolumeMountPointW(PCWSTR(mount_point.as_ptr()), &mut volume)
            .inspect_err(|e| debug!(error = %e, "Volume device name unavailable"))
            .ok()?;
    }
    let end = volume.iter().position(|&c| c == 0)?;
    let name = String::from_utf16(&volume[..end]).ok()?;
    Some(name.trim_end_matches('\\').to_owned())
}

fn seek_penalty(device: &str) -> Option<bool> {
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(device),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .inspect_err(|e| debug!(error = %e, "Volume device not openable for the seek-penalty query"))
    .ok()?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
    let mut returned = 0u32;
    let queried = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(std::ptr::from_ref(&query).cast()),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(std::ptr::from_mut(&mut descriptor).cast()),
            size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            Some(&mut returned),
            None,
        )
    };
    let _ = unsafe { CloseHandle(handle) }; // ignore-ok: the query already ran; a handle that fails to close changes nothing
    queried
        .inspect_err(|e| debug!(error = %e, "Seek-penalty query refused by the volume"))
        .ok()?;
    (returned as usize >= size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>())
        .then_some(descriptor.IncursSeekPenalty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absent_drive() -> Option<std::path::PathBuf> {
        ('D'..='Z')
            .rev()
            .map(|letter| std::path::PathBuf::from(format!("{letter}:\\")))
            .find(|root| !root.exists())
    }

    #[test]
    fn a_local_folder_resolves_to_its_volume_device_without_the_trailing_separator() {
        let dir = std::env::temp_dir();
        let device = volume_device(&dir).expect("temp folder is on a mounted volume");
        assert!(device.starts_with(r"\\?\Volume{"), "{device}");
        assert!(device.ends_with('}'), "{device}");

        let nested = dir.join(format!("dekan_disk_kind_{}", std::process::id()));
        std::fs::write(&nested, b"probe").expect("probe file");
        let found = volume_device(&nested);
        std::fs::remove_file(&nested).ok();
        assert_eq!(found.as_deref(), Some(device.as_str()));
    }

    #[test]
    fn every_path_on_one_volume_gets_the_same_answer() {
        let dir = std::env::temp_dir();
        let kind = disk_kind(&dir);
        assert_eq!(disk_kind(&dir.join("a").join("b")), kind);
        let root = dir.ancestors().last().expect("root");
        assert_eq!(disk_kind(root), kind);
    }

    #[test]
    fn a_drive_that_is_not_mounted_is_unknown_not_a_guess() {
        let Some(root) = absent_drive() else {
            return;
        };
        assert_eq!(volume_device(&root), None);
        assert_eq!(disk_kind(&root.join("Riot Games")), DiskKind::Unknown);
    }

    #[test]
    fn a_device_that_cannot_be_opened_reports_no_seek_penalty() {
        assert_eq!(
            seek_penalty(r"\\?\Volume{00000000-0000-0000-0000-000000000000}"),
            None
        );
    }

    #[test]
    fn the_log_values_match_the_documented_ones() {
        assert_eq!(
            [
                DiskKind::SolidState.as_str(),
                DiskKind::Rotational.as_str(),
                DiskKind::Unknown.as_str(),
            ],
            ["ssd", "hdd", "unknown"]
        );
    }
}
