//! Atomic, no-overwrite name operations, executed only by a worker.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) enum NameOperation {
    Folder(PathBuf),
    Text(PathBuf),
    Rename(PathBuf),
}

pub(super) fn validate_name(name: &str) -> Result<(), String> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem
                .chars()
                .last()
                .is_some_and(|c| "123456789¹²³".contains(c));
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        || reserved
        || name.encode_utf16().count() > 255
    {
        return Err("名前に使用できない文字、予約名、末尾の空白・ピリオドが含まれています".into());
    }
    Ok(())
}

pub(super) fn execute(operation: &NameOperation, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let result = match operation {
        NameOperation::Folder(parent) => {
            let target = parent.join(name);
            std::fs::create_dir(&target).map(|_| target)
        }
        NameOperation::Text(parent) => {
            let target = parent.join(name);
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map(|_| target)
        }
        NameOperation::Rename(source) => {
            let target = source
                .parent()
                .ok_or("この項目の名前は変更できません")?
                .join(name);
            if source == &target {
                return Ok(target);
            }
            rename_exclusive(source, &target).map(|_| target)
        }
    };
    result.map_err(|error| format!("操作できませんでした: {error}"))
}

fn rename_exclusive(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // No REPLACE_EXISTING flag: collision detection and rename are one OS operation.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())?;
        let target = CString::new(target.as_os_str().as_bytes())?;
        if unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (source, target);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "このOSの名前変更は未対応です",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_traversal_and_windows_ambiguous_names() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "a:b",
            "NUL.txt",
            "con",
            "LPT9",
            "name.",
            "name ",
            "a\0b",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert!(validate_name("日本語の名前.txt").is_ok());
    }
    #[test]
    fn create_and_rename_never_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "skylis-names-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let first = execute(&NameOperation::Text(root.clone()), "first.txt").unwrap();
        std::fs::write(&first, "keep").unwrap();
        assert!(execute(&NameOperation::Text(root.clone()), "first.txt").is_err());
        let other = execute(&NameOperation::Text(root.clone()), "other.txt").unwrap();
        assert!(execute(&NameOperation::Rename(other.clone()), "first.txt").is_err());
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "keep");
        assert!(other.exists());
        let renamed = execute(&NameOperation::Rename(other), "new.txt").unwrap();
        assert!(renamed.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
