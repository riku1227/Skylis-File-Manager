//! Host the operating system's menu without translating extension-owned items into GPUI.
//!
//! Windows Shell extensions execute on a dedicated STA thread. Dropping the result receiver
//! cancels a pending or displayed menu; an already executing third-party command cannot be
//! forcibly interrupted safely. The application may refresh its listing after `Invoked`.
use std::path::{Path, PathBuf};

mod properties;
#[cfg(windows)]
mod windows;
pub use properties::active as properties_active;
pub use properties::show as properties;

/// Shell parsing accepts Windows separators, unlike Rust filesystem APIs that also
/// accept `/`. Keep the exact Unicode/UTF-16 path and do no I/O or symlink resolution.
/// Verbatim prefixes are preserved so normalization cannot change a file's identity.
#[cfg(windows)]
pub(crate) fn windows_path_wide(path: &Path) -> anyhow::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    validate_path(path)?;
    Ok(path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .chain(Some(0))
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuOutcome {
    Cancelled,
    Invoked,
}

pub type MenuResult = Result<MenuOutcome, String>;

#[derive(Debug)]
enum Target {
    Selection(Vec<PathBuf>),
    Background(PathBuf),
}

/// `position` is in GPUI client-area logical pixels, including on mixed-DPI displays.
/// Keep the receiver alive while the menu is wanted. Only one native popup may be active.
pub fn show_context_menu(
    paths: Vec<PathBuf>,
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> anyhow::Result<async_channel::Receiver<MenuResult>> {
    validate_selection(&paths)?;
    show(Target::Selection(paths), window, position)
}

/// The directory's background menu (for example, Paste / New), rather than its item menu.
pub fn show_background_menu(
    folder: PathBuf,
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> anyhow::Result<async_channel::Receiver<MenuResult>> {
    validate_path(&folder)?;
    show(Target::Background(folder), window, position)
}

fn show(
    target: Target,
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> anyhow::Result<async_channel::Receiver<MenuResult>> {
    #[cfg(windows)]
    {
        windows::show(target, window, position)
    }
    #[cfg(not(windows))]
    {
        let _ = (target, window, position);
        anyhow::bail!("このOSではWindows Shellメニューを利用できません")
    }
}

fn validate_selection(paths: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "メニューを表示する項目がありません");
    for path in paths {
        validate_path(path)?;
    }
    // BHID_SFUIObject supports a flat selection only. Do not silently act on just one
    // of a mixed-folder selection. No filesystem resolution is performed on the UI thread.
    let parent = paths[0].parent();
    anyhow::ensure!(
        paths.iter().all(|path| path.parent() == parent),
        "Windowsメニューには同じフォルダーの項目を選択してください"
    );
    Ok(())
}

fn validate_path(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "Shellメニューには絶対パスが必要です");
    anyhow::ensure!(
        !path.as_os_str().as_encoded_bytes().contains(&0),
        "パスに使用できない文字があります"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\")
        } else {
            PathBuf::from("/")
        }
    }

    #[test]
    fn shell_selection_rejects_empty_relative_and_nul_paths() {
        assert!(validate_selection(&[]).is_err());
        assert!(validate_selection(&[PathBuf::from("relative.txt")]).is_err());
        assert!(validate_selection(&[root().join("bad\0name")]).is_err());
    }

    #[test]
    fn shell_selection_requires_a_single_parent_without_touching_disk() {
        assert!(validate_selection(&[root().join("missing/a"), root().join("missing/b")]).is_ok());
        assert!(validate_selection(&[root().join("one/a"), root().join("two/b")]).is_err());
        assert!(validate_selection(&[root()]).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn shell_paths_normalize_separators_without_resolving_links_or_losing_utf16() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        for (input, expected) in [
            ("C:/資料/a.txt", r"C:\資料\a.txt"),
            ("//server/share/a.txt", r"\\server\share\a.txt"),
            (r"\\?\C:\folder\file", r"\\?\C:\folder\file"),
        ] {
            assert_eq!(
                String::from_utf16(&windows_path_wide(Path::new(input)).unwrap()).unwrap(),
                format!("{expected}\0")
            );
        }
        let raw = [b'C' as u16, b':' as u16, b'/' as u16, 0xd800];
        let path = PathBuf::from(OsString::from_wide(&raw));
        assert_eq!(
            windows_path_wide(&path).unwrap(),
            [b'C' as u16, b':' as u16, b'\\' as u16, 0xd800, 0]
        );
    }
}
