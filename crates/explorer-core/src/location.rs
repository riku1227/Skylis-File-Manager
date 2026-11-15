use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Home,
    Folder(PathBuf),
    /// An archive member is never a filesystem destination. `prefix` is a
    /// validated, slash-separated member path; an empty prefix is the root.
    Archive {
        archive: PathBuf,
        prefix: String,
    },
}

impl Location {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Home | Self::Archive { .. } => None,
            Self::Folder(path) => Some(path),
        }
    }
}

/// Parse an address only; filesystem validation belongs on a worker thread.
pub fn resolve_address(text: &str, base: &Path, home: &Path) -> Result<PathBuf, String> {
    let text = text.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(text);
    if text.is_empty() || text.contains(['\0', '\r', '\n']) {
        return Err("フォルダーのパスを入力してください".into());
    }
    if text.contains("://") {
        return Err("URLではなくフォルダーのパスを入力してください".into());
    }
    if text == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return Ok(home.join(rest));
    }
    #[cfg(windows)]
    if text.as_bytes().get(1) == Some(&b':') && text.as_bytes()[0].is_ascii_alphabetic() {
        if text.len() == 2 {
            return Ok(PathBuf::from(format!("{text}\\")));
        }
        if !matches!(text.as_bytes()[2], b'/' | b'\\') {
            return Err("ドライブ文字の後に \\ を付けてください（例: D:\\）".into());
        }
    }
    Ok(base.join(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_members_never_expose_a_filesystem_destination() {
        let location = Location::Archive {
            archive: PathBuf::from("C:/downloads/example.zip"),
            prefix: "inside/folder".into(),
        };
        assert!(location.path().is_none());
    }

    #[test]
    fn home_and_folders_share_history_without_a_fake_filesystem_path() {
        use crate::History;
        let mut history = History::new(Location::Home);
        let folder = Location::Folder(PathBuf::from("/work"));
        history.push(folder.clone(), None);
        assert_eq!(
            history.go_back(Some("資料".into())),
            Some((Location::Home, None))
        );
        assert_eq!(
            history.go_forward(None),
            Some((folder.clone(), Some("資料".into())))
        );
        history.go_back(None);
        history.push(Location::Folder(PathBuf::from("/other")), None);
        assert!(!history.can_go_forward());
    }

    #[test]
    fn address_accepts_quoted_unicode_and_relative_paths() {
        let home = Path::new("/users/aoi");
        let base = Path::new("/work");
        assert_eq!(
            resolve_address("  \"資料/企画\"  ", base, home).unwrap(),
            base.join("資料/企画")
        );
        assert_eq!(
            resolve_address("~/Documents", base, home).unwrap(),
            home.join("Documents")
        );
        assert_eq!(resolve_address("~", base, home).unwrap(), home);
        assert!(resolve_address("  ", base, home).is_err());
        assert!(resolve_address("a\0b", base, home).is_err());
        assert!(resolve_address("https://example.com", base, home).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_addresses_keep_unc_and_root_bare_drive_letters() {
        let base = Path::new(r"C:\work");
        let home = Path::new(r"C:\Users\aoi");
        assert_eq!(
            resolve_address("D:", base, home).unwrap(),
            PathBuf::from(r"D:\")
        );
        assert_eq!(
            resolve_address(r"\\server\share\資料", base, home).unwrap(),
            PathBuf::from(r"\\server\share\資料")
        );
        assert_eq!(
            resolve_address(r"\Windows", base, home).unwrap(),
            PathBuf::from(r"C:\Windows")
        );
        assert!(resolve_address("D:relative", base, home).is_err());
    }
}
