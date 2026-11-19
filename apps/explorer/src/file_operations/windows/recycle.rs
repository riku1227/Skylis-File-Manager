use std::{fs, path::Path};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    Storage::FileSystem::{GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
    },
};

const DRIVE_FIXED: u32 = 3;
const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Drive {
    Fixed,
    NonFixed,
    Unknown,
}

impl Drive {
    fn from_windows_type(drive_type: u32) -> Self {
        match drive_type {
            DRIVE_FIXED => Self::Fixed,
            2 | 5 | 6 => Self::NonFixed, // removable, optical, RAM disk
            // Includes DRIVE_REMOTE (4): Windows supports Recycle Bin on network-redirected
            // Known Folders, so UNC/network drives defer to Layer 2 (PreDeleteItem) & Layer 3.
            _ => Self::Unknown,
        }
    }
}

#[derive(Default)]
struct BitBucket {
    nuke_on_delete: Option<u32>,
    max_capacity: Option<u32>,
    policy_no_recycle: bool,
}

fn blocked(drive: Drive, settings: BitBucket, file_bytes: Option<u64>) -> bool {
    settings.policy_no_recycle
        || matches!(drive, Drive::NonFixed)
        || settings.nuke_on_delete.is_some_and(|value| value != 0)
        || settings.max_capacity.is_some_and(|size| {
            size == 0 || file_bytes.is_some_and(|bytes| bytes > u64::from(size) * MIB)
        })
}

fn policy_disables_recycle() -> bool {
    let policy_key = wide_c(r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer");
    read_dword(HKEY_CURRENT_USER, &policy_key, "NoRecycleFiles").is_some_and(|value| value != 0)
        || read_dword(HKEY_LOCAL_MACHINE, &policy_key, "NoRecycleFiles")
            .is_some_and(|value| value != 0)
}

fn volume_and_drive(parent: &Path) -> Option<(Drive, Option<String>)> {
    let wide = crate::native_shell::windows_path_wide(parent).ok()?;
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut volume = [0u16; 64];
    let has_volume = unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } != 0;
    // Prefer \\?\Volume{GUID}\ for GetDriveTypeW because its support is guaranteed by Windows docs;
    // fall back to root when no GUID exists (e.g. optical or virtual mounts).
    let drive_type = if has_volume {
        unsafe { GetDriveTypeW(volume.as_ptr()) }
    } else {
        unsafe { GetDriveTypeW(root.as_ptr()) }
    };
    let drive = Drive::from_windows_type(drive_type);
    let volume_name = if has_volume {
        let len = volume.iter().position(|&unit| unit == 0)?;
        String::from_utf16(&volume[..len]).ok()
    } else {
        None
    };
    Some((drive, volume_name))
}

#[cfg(test)]
fn drive_for_path(path: &Path) -> Drive {
    let Ok(canonical) = path.canonicalize() else {
        return Drive::Unknown;
    };
    volume_and_drive(&canonical).map_or(Drive::Unknown, |(drive, _)| drive)
}

pub(super) fn cannot_recycle(path: &Path) -> bool {
    fn inspect(path: &Path) -> Option<bool> {
        let policy_no_recycle = policy_disables_recycle();
        if policy_no_recycle {
            return Some(blocked(
                Drive::Unknown,
                BitBucket {
                    policy_no_recycle: true,
                    ..Default::default()
                },
                None,
            ));
        }
        // Deleting a link acts on the volume containing the link, regardless of its target.
        // If canonicalize fails (e.g. drive-less mount point), defer safely to Layer 2 (PreDeleteItem).
        let parent = path.parent()?.canonicalize().ok()?;
        let (drive, volume) = volume_and_drive(&parent)?;
        if !matches!(drive, Drive::Fixed) {
            return Some(blocked(drive, BitBucket::default(), None));
        }
        let volume = volume?;
        let guid = volume.strip_prefix(r"\\?\Volume")?.strip_suffix('\\')?;
        let key = wide_c(&format!(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}"
        ));
        let settings = BitBucket {
            nuke_on_delete: read_dword(HKEY_CURRENT_USER, &key, "NukeOnDelete"),
            max_capacity: read_dword(HKEY_CURRENT_USER, &key, "MaxCapacity"),
            policy_no_recycle,
        };
        let file_bytes = settings
            .max_capacity
            .filter(|&limit| limit > 0)
            .and_then(|_| {
                let metadata = fs::symlink_metadata(path).ok()?;
                (metadata.is_file()).then(|| metadata.len())
            });
        Some(blocked(drive, settings, file_bytes))
    }
    inspect(path).unwrap_or(false)
}

fn wide_c(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn read_dword(root_key: HKEY, key: &[u16], name: &str) -> Option<u32> {
    let name = wide_c(name);
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            root_key,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        )
    };
    (result == ERROR_SUCCESS && bytes as usize == size_of::<u32>()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_nonfixed_local_drives_are_rejected_by_drive_type() {
        for drive in [2, 5, 6] {
            assert!(blocked(
                Drive::from_windows_type(drive),
                BitBucket::default(),
                None
            ));
        }
        for drive in [0, 1, 3, 4] {
            assert!(!blocked(
                Drive::from_windows_type(drive),
                BitBucket::default(),
                None
            ));
        }
    }

    #[test]
    fn volume_and_policy_can_disable_recycle_for_all_items() {
        assert!(blocked(
            Drive::Fixed,
            BitBucket {
                nuke_on_delete: Some(1),
                max_capacity: None,
                policy_no_recycle: false,
            },
            None
        ));
        assert!(blocked(
            Drive::Fixed,
            BitBucket {
                nuke_on_delete: None,
                max_capacity: Some(0),
                policy_no_recycle: false,
            },
            None
        ));
        assert!(blocked(
            Drive::Fixed,
            BitBucket {
                nuke_on_delete: Some(0),
                max_capacity: Some(100),
                policy_no_recycle: true,
            },
            None
        ));
    }

    #[test]
    fn capacity_only_compares_a_single_known_file_in_mib() {
        let settings = || BitBucket {
            nuke_on_delete: Some(0),
            max_capacity: Some(2),
            policy_no_recycle: false,
        };
        assert!(!blocked(Drive::Fixed, settings(), Some(2_097_152)));
        assert!(blocked(Drive::Fixed, settings(), Some(2_097_153)));
        assert!(!blocked(Drive::Fixed, settings(), None));
        assert!(!blocked(Drive::Fixed, BitBucket::default(), Some(u64::MAX)));
    }

    #[test]
    fn resolves_temp_dir_drive_type_and_runs_precheck_cleanly() {
        let temp = tempfile::tempdir().unwrap();
        assert_ne!(drive_for_path(temp.path()), Drive::Unknown);
        let file = temp.path().join("recycle-check.txt");
        std::fs::write(&file, b"small").unwrap();
        let _ = cannot_recycle(&file);
    }
}
