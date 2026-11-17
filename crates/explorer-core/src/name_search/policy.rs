//! Metadata-only, no-follow platform boundary. Never call from the UI thread.
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locality {
    Local,
    NeedsStart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub volume: u64,
    file: u64,
}

pub fn classify(path: &Path) -> Locality {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
        // Resolve intermediate junctions before classifying: a drive-letter path
        // can reach a UNC share through an ancestor. This runs in the explicit lane.
        let Some(resolved) = resolved_root(path) else {
            return Locality::NeedsStart;
        };
        let wide = wide(&resolved);
        let mut volume = vec![0u16; 32768];
        unsafe {
            if GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) != 0 {
                // Fixed and RAM disks only. Removable, remote and unknown need consent per query.
                if matches!(GetDriveTypeW(volume.as_ptr()), 3 | 6) {
                    return Locality::Local;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        if let Ok(path) = CString::new(path.as_os_str().as_bytes()) {
            let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
            unsafe {
                if libc::statfs(path.as_ptr(), stat.as_mut_ptr()) == 0
                    && stat.assume_init().f_flags & libc::MNT_LOCAL as u32 != 0
                {
                    return Locality::Local;
                }
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = path;
    Locality::NeedsStart
}
pub(super) fn path_bytes(path: &Path) -> usize {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().count() * 2
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().len()
    }
}
#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    // std's directory enumeration supports long paths. Keep the explicit Win32
    // metadata adapter equally capable, including non-Unicode names.
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let native: Vec<u16> = absolute
        .as_os_str()
        .encode_wide()
        .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
        .collect();
    let verbatim: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let device: Vec<u16> = "\\\\.\\".encode_utf16().collect();
    let mut output = if native.starts_with(&verbatim) || native.starts_with(&device) {
        native
    } else if native.starts_with(&[92, 92]) {
        "\\\\?\\UNC\\"
            .encode_utf16()
            .chain(native.into_iter().skip(2))
            .collect()
    } else {
        verbatim.into_iter().chain(native).collect()
    };
    output.push(0);
    output
}
#[cfg(windows)]
fn resolved_root(path: &Path) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::*,
    };
    unsafe {
        let name = wide(path);
        let handle = CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut output = vec![0u16; 32768];
        let len = GetFinalPathNameByHandleW(handle, output.as_mut_ptr(), output.len() as u32, 0);
        CloseHandle(handle);
        if len == 0 || len as usize >= output.len() {
            return None;
        }
        let resolved = std::ffi::OsString::from_wide(&output[..len as usize]);
        Some(resolved.into())
    }
}
#[cfg(windows)]
fn allowed_reparse(attributes: u32, tag: u32) -> bool {
    const OFFLINE_OR_RECALL: u32 = 0x1000 | 0x40000 | 0x400000;
    if attributes & OFFLINE_OR_RECALL != 0 {
        return false;
    }
    if attributes & 0x400 == 0 {
        return true;
    }
    // IO_REPARSE_TAG_CLOUD and CLOUD_1..F are not name surrogates.
    tag & 0x20000000 == 0 && tag & 0xffff0fff == 0x9000001a
}
#[cfg(windows)]
pub(super) fn directory(path: &Path) -> io::Result<Option<Identity>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::*,
    };
    let path = wide(path);
    unsafe {
        let handle = CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        struct Handle(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for Handle {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let _handle = Handle(handle);
        let mut info = std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>();
        if GetFileInformationByHandle(handle, &mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Ok(None);
        }
        let mut tag = FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: info.dwFileAttributes,
            ReparseTag: 0,
        };
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                &mut tag as *mut _ as *mut _,
                std::mem::size_of_val(&tag) as u32,
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !allowed_reparse(tag.FileAttributes, tag.ReparseTag) {
            return Ok(None);
        }
        Ok(Some(Identity {
            volume: info.dwVolumeSerialNumber as u64,
            file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        }))
    }
}
#[cfg(unix)]
pub(super) fn directory(path: &Path) -> io::Result<Option<Identity>> {
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::MetadataExt as MacMetadataExt;
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    #[cfg(target_os = "macos")]
    if meta.st_flags() & 0x40000000 != 0 {
        return Ok(None);
    } // SF_DATALESS
    Ok(
        (meta.is_dir() && !meta.file_type().is_symlink()).then(|| Identity {
            volume: meta.dev(),
            file: meta.ino(),
        }),
    )
}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn long_native_paths_preserve_unc_and_non_unicode_names() {
        use std::{
            os::windows::ffi::{OsStrExt, OsStringExt},
            path::Path,
        };
        let unc = super::wide(Path::new("\\\\server\\share\\folder"));
        assert_eq!(
            String::from_utf16(&unc[..unc.len() - 1]).unwrap(),
            "\\\\?\\UNC\\server\\share\\folder"
        );
        let mut native: Vec<u16> = "C:\\folder\\".encode_utf16().collect();
        native.push(0xd800);
        let path = std::ffi::OsString::from_wide(&native);
        let encoded = super::wide(Path::new(&path));
        assert_eq!(
            &encoded[4..encoded.len() - 1],
            path.encode_wide().collect::<Vec<_>>()
        );
        assert_eq!(
            super::wide(Path::new("C:/folder/../target")),
            "\\\\?\\C:\\target\0".encode_utf16().collect::<Vec<_>>()
        );
    }
    #[cfg(windows)]
    #[test]
    fn local_cloud_allowed_surrogates_unknown_and_recall_rejected() {
        use super::allowed_reparse as a;
        assert!(a(0x400, 0x9000001a));
        assert!(a(0x400, 0x9000f01a));
        assert!(!a(0x400, 0xa0000003));
        assert!(!a(0x400, 0xa000000c));
        assert!(!a(0x400, 0x80000042));
        assert!(!a(0x400 | 0x40000, 0x9000001a));
        assert!(!a(0x400 | 0x400000, 0x9000001a));
        assert!(!a(0x1000, 0));
    }
}
