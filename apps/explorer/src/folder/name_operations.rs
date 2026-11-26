//! Atomic, no-overwrite name operations, executed only by a worker.
use std::path::{Path, PathBuf};

/// An item created with a default name and then renamed inline, as in Explorer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NewItem {
    Folder,
    Text,
}

impl NewItem {
    /// Default name for the `n`-th attempt: `新しいフォルダー`, `新しいフォルダー (2)`, ...
    fn candidate(self, n: u32) -> String {
        let (stem, extension) = match self {
            Self::Folder => ("新しいフォルダー", ""),
            Self::Text => ("新しいテキスト ドキュメント", ".txt"),
        };
        if n <= 1 {
            format!("{stem}{extension}")
        } else {
            format!("{stem} ({n}){extension}")
        }
    }
}

/// Upper bound on default-name attempts so a hostile or huge folder cannot spin the worker.
const MAX_NEW_ITEM_ATTEMPTS: u32 = 1000;

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

/// Creates `kind` under `parent` with the first free default name. Each attempt is an
/// exclusive create, so an item that appears concurrently is skipped, never overwritten.
pub(super) fn create_new_item(parent: &Path, kind: NewItem) -> Result<PathBuf, String> {
    for n in 1..=MAX_NEW_ITEM_ATTEMPTS {
        let target = parent.join(kind.candidate(n));
        let result = match kind {
            NewItem::Folder => std::fs::create_dir(&target),
            NewItem::Text => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map(drop),
        };
        match result {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("作成できませんでした: {error}")),
        }
    }
    Err("作成できませんでした: 既定の名前がすべて使用されています".into())
}

pub(super) fn rename(source: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let target = source
        .parent()
        .ok_or("この項目の名前は変更できません")?
        .join(name);
    if source == target {
        // The item may have disappeared while its inline editor stayed open.
        // Validate on this worker even for a no-op, without following links.
        return std::fs::symlink_metadata(source)
            .map(|_| target)
            .map_err(|error| format!("元の項目を確認できませんでした: {error}"));
    }
    rename_exclusive(source, &target)
        .map(|_| target)
        .map_err(|error| format!("操作できませんでした: {error}"))
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
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.txt");
        std::fs::write(&first, "keep").unwrap();
        let other = root.join("other.txt");
        std::fs::write(&other, "").unwrap();
        assert!(rename(&other, "first.txt").is_err());
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "keep");
        assert!(other.exists());
        let renamed = rename(&other, "new.txt").unwrap();
        assert!(renamed.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_items_take_the_next_free_default_name_without_overwriting() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skylis-new-{}-{unique}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let folder = create_new_item(&root, NewItem::Folder).unwrap();
        assert_eq!(folder, root.join("新しいフォルダー"));
        assert!(folder.is_dir());
        std::fs::write(root.join("新しいテキスト ドキュメント.txt"), "keep").unwrap();
        let text = create_new_item(&root, NewItem::Text).unwrap();
        assert_eq!(text, root.join("新しいテキスト ドキュメント (2).txt"));
        assert!(text.is_file());
        assert_eq!(
            std::fs::read_to_string(root.join("新しいテキスト ドキュメント.txt")).unwrap(),
            "keep"
        );
        let second = create_new_item(&root, NewItem::Folder).unwrap();
        assert_eq!(second, root.join("新しいフォルダー (2)"));
        assert!(create_new_item(&root.join("missing"), NewItem::Folder).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_only_rename_changes_spelling_without_replacing_content() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("skylis-case-{}-{unique}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("CaseStudy.txt");
        std::fs::write(&source, "preserve case-only rename content").unwrap();
        let target = rename(&source, "casestudy.TXT").unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "preserve case-only rename content"
        );
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("casestudy.TXT")]);
        assert!(root.is_absolute() && root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_name_does_not_report_success_for_a_missing_source() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let missing = std::env::temp_dir().join(format!(
            "skylis-missing-{}-{unique}.txt",
            std::process::id()
        ));
        let name = missing.file_name().unwrap().to_str().unwrap();
        assert!(rename(&missing, name).is_err());
    }
}
