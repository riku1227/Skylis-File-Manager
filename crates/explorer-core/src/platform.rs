//! OS連携。PoCではcore内に置くが、将来は platform / platform-win / platform-mac crate に分離する。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Drive,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub icon: PlaceIcon,
}

/// 呼び出したユーザーのクォータを反映した容量（バイト）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveSpace {
    pub total: u64,
    pub available: u64,
}

/// このディレクトリを含むドライブの容量を問い合わせる。
///
/// リムーバブル・ネットワークドライブ等で待たされる場合があるため、
/// 必ずワーカースレッドから呼び、UIの描画/イベント処理内で呼ばないこと。
/// WindowsとmacOSに対応。ほかのOSでは`Unsupported`を返す。
pub fn drive_space(path: &Path) -> std::io::Result<DriveSpace> {
    imp::drive_space(path)
}

/// サイドバーの「よく使う場所」
pub fn favorite_places() -> Vec<Place> {
    imp::favorite_places()
}

/// サイドバーの「ドライブ」
pub fn drives() -> Vec<Place> {
    imp::drives()
}

pub fn home_dir() -> PathBuf {
    imp::home_dir()
}

/// 既定のアプリでファイルを開く
pub fn open_with_default_app(path: &Path) -> std::io::Result<()> {
    imp::open_with_default_app(path)
}

#[cfg(windows)]
#[path = "platform/windows_photos.rs"]
mod windows_photos;

/// Use the Photos viewer activation when Microsoft Photos owns the association.
pub fn open_image_with_default_app(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if let Some(uri) = windows_photos::viewer_uri_if_default(path)
        && imp::shell_open(uri.as_ref()).is_ok()
    {
        return Ok(());
    }
    open_with_default_app(path)
}

/// Open the virtual recycle bin through the OS shell, never as a filesystem directory.
pub fn open_recycle_bin() -> std::io::Result<()> {
    #[cfg(windows)]
    return imp::open_with_default_app(Path::new("shell:RecycleBinFolder"));
    #[cfg(not(windows))]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "recycle bin is not implemented on this platform",
    ))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures,
        FOLDERID_Profile, SHGetKnownFolderPath, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows_sys::core::GUID;

    /// OneDriveへのリダイレクトなどを反映するため、固定パスではなくKnown Folder APIで取得する
    fn known_folder(id: &GUID) -> Option<PathBuf> {
        let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: 成功時はCoTaskMemAllocされたNUL終端文字列が返る。失敗時もrawの解放が必要。
        unsafe {
            let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw);
            let result = if hr >= 0 && !raw.is_null() {
                let mut len = 0;
                while *raw.add(len) != 0 {
                    len += 1;
                }
                let s = OsString::from_wide(std::slice::from_raw_parts(raw, len));
                Some(PathBuf::from(s))
            } else {
                None
            };
            CoTaskMemFree(raw as *const _);
            result
        }
    }

    pub fn home_dir() -> PathBuf {
        known_folder(&FOLDERID_Profile)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("C:\\"))
    }

    pub fn favorite_places() -> Vec<Place> {
        let items = [
            ("ホーム", FOLDERID_Profile, PlaceIcon::Home),
            ("デスクトップ", FOLDERID_Desktop, PlaceIcon::Desktop),
            ("ドキュメント", FOLDERID_Documents, PlaceIcon::Documents),
            ("ダウンロード", FOLDERID_Downloads, PlaceIcon::Downloads),
            ("ピクチャ", FOLDERID_Pictures, PlaceIcon::Pictures),
        ];
        items
            .iter()
            .filter_map(|(label, id, icon)| {
                known_folder(id).map(|path| Place {
                    label: (*label).into(),
                    path,
                    icon: *icon,
                })
            })
            .collect()
    }

    pub fn drives() -> Vec<Place> {
        // GetLogicalDrivesはディスクに触れないので、光学ドライブや切断されたネットワークドライブでも固まらない
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| {
                let letter = (b'A' + i) as char;
                Place {
                    label: format!("{letter}:"),
                    path: PathBuf::from(format!("{letter}:\\")),
                    icon: PlaceIcon::Drive,
                }
            })
            .collect()
    }

    pub fn drive_space(path: &Path) -> std::io::Result<DriveSpace> {
        let directory = wide(path.as_os_str());
        if directory[..directory.len() - 1].contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        let mut total = 0u64;
        let mut available = 0u64;
        // SAFETY: directoryはNUL終端UTF-16。出力ポインタは有効なu64を指す。
        // 第4引数は不要な全ユーザー向け空き容量なのでNULLにする。
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                directory.as_ptr(),
                &mut available,
                &mut total,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(DriveSpace { total, available })
        }
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn open_with_default_app(path: &Path) -> std::io::Result<()> {
        shell_open(path.as_os_str())
    }

    pub(super) fn shell_open(target: &std::ffi::OsStr) -> std::io::Result<()> {
        let file = wide(target);
        let verb = wide("open".as_ref());
        // SAFETY: 引数はすべてNUL終端のUTF-16
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // 32以下はエラー（ShellExecuteの仕様）
        if (r as isize) <= 32 {
            Err(std::io::Error::other(format!(
                "ShellExecuteW failed ({})",
                r as isize
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn home_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn favorite_places() -> Vec<Place> {
        let home = home_dir();
        [
            ("ホーム", None, PlaceIcon::Home),
            ("デスクトップ", Some("Desktop"), PlaceIcon::Desktop),
            ("書類", Some("Documents"), PlaceIcon::Documents),
            ("ダウンロード", Some("Downloads"), PlaceIcon::Downloads),
            ("ピクチャ", Some("Pictures"), PlaceIcon::Pictures),
        ]
        .into_iter()
        .map(|(label, sub, icon)| Place {
            label: label.into(),
            path: sub.map(|s| home.join(s)).unwrap_or_else(|| home.clone()),
            icon,
        })
        .collect()
    }

    pub fn drives() -> Vec<Place> {
        vec![Place {
            label: "/".into(),
            path: PathBuf::from("/"),
            icon: PlaceIcon::Drive,
        }]
    }

    #[cfg(target_os = "macos")]
    pub fn drive_space(path: &Path) -> std::io::Result<DriveSpace> {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL")
        })?;
        let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: valid C path and writable output; initialized only on success.
        if unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stats = unsafe { stats.assume_init() };
        let bytes = |blocks: u64| {
            blocks
                .checked_mul(u64::from(stats.f_bsize))
                .ok_or_else(|| std::io::Error::other("volume capacity overflows byte count"))
        };
        Ok(DriveSpace {
            total: bytes(stats.f_blocks)?,
            available: bytes(stats.f_bavail)?,
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn drive_space(_: &Path) -> std::io::Result<DriveSpace> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "drive space is not implemented on this platform",
        ))
    }

    pub fn open_with_default_app(path: &Path) -> std::io::Result<()> {
        let cmd = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(cmd)
            .arg(path)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn current_drive_capacity_is_consistent() {
        let space = drive_space(&std::env::temp_dir()).expect("query temporary directory drive");
        assert!(space.total > 0);
        assert!(space.available <= space.total);
        let used_ratio = (space.total - space.available) as f64 / space.total as f64;
        assert!((0.0..=1.0).contains(&used_ratio));
    }

    #[test]
    fn rejects_nul_instead_of_querying_truncated_path() {
        let path = PathBuf::from("C:\\\0not-a-directory");
        assert_eq!(
            drive_space(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_space_tests {
    use super::*;
    #[test]
    fn capacity_matches_native_statfs_for_root_and_home() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        for path in [PathBuf::from("/"), home_dir()] {
            let before = drive_space(&path).unwrap();
            let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut native = std::mem::MaybeUninit::<libc::statfs>::uninit();
            assert_eq!(
                unsafe { libc::statfs(cpath.as_ptr(), native.as_mut_ptr()) },
                0
            );
            let native = unsafe { native.assume_init() };
            let after = drive_space(&path).unwrap();
            assert!(before.total > 0);
            assert_eq!(before.total, native.f_blocks * u64::from(native.f_bsize));
            let reference = native.f_bavail * u64::from(native.f_bsize);
            // Concurrent disk writes can change available blocks between calls.
            assert!(before.available.abs_diff(reference) < 64 * 1024 * 1024);
            assert!(after.available.abs_diff(reference) < 64 * 1024 * 1024);
            assert!(before.available <= before.total);
        }
    }
    #[test]
    fn invalid_capacity_paths_are_errors() {
        assert_eq!(
            drive_space(Path::new("/\0truncated")).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            drive_space(Path::new("/nonexistent-skylis-capacity/path"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
