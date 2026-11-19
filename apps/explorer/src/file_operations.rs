//! Native file operations. OperationCenter admits jobs and calls these adapters on workers.
use crate::{operation_center::JobWorker, settings_data::FileTransferEngine};
use anyhow::Context as _;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_io;
#[cfg(windows)]
pub use windows_io::{
    NativeFileDrag, clear_file_clipboard, read_file_clipboard, write_file_clipboard,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferKind {
    Copy,
    Move,
    /// ドロップ時の既定動作。各項目の実ボリュームをワーカーで判定する。
    Automatic,
}

impl TransferKind {
    pub fn from_drop_modifiers(control: bool, shift: bool) -> Self {
        if control {
            Self::Copy
        } else if shift {
            Self::Move
        } else {
            Self::Automatic
        }
    }
}

mod volume;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferOutcome {
    Completed,
    CompletedWithIssues,
    /// Some items may already have changed. Refresh every affected listing.
    Aborted,
}

pub type TransferResult = Result<TransferOutcome, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteKind {
    Recycle,
    /// Requires explicit confirmation before executing.
    Permanent,
}

pub fn supports_delete(kind: DeleteKind) -> bool {
    cfg!(windows) || (cfg!(target_os = "macos") && kind == DeleteKind::Recycle)
}

pub(crate) fn delete(
    paths: &[PathBuf],
    kind: DeleteKind,
    engine: FileTransferEngine,
    worker: &mut JobWorker,
) -> TransferResult {
    (|| {
        anyhow::ensure!(
            supports_delete(kind),
            "このOSでは指定した削除方法に対応していません"
        );
        let paths = prepare_deletion(paths)?;
        if worker.is_cancelled() {
            return Ok(TransferOutcome::Aborted);
        }
        #[cfg(windows)]
        {
            windows::delete(&paths, kind, engine, worker)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = engine;
            macos_trash(&paths, worker)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (engine, paths);
            anyhow::bail!("このOSの削除にはまだ対応していません")
        }
    })()
    .map_err(|error: anyhow::Error| format!("{error:#}"))
}

#[cfg(any(windows, test))]
pub(crate) fn confirm_permanent_delete(
    paths: &[PathBuf],
    reason: crate::operation_center::PermanentDeleteReason,
    worker: &mut JobWorker,
    delete: impl FnOnce(&[PathBuf], &mut JobWorker) -> anyhow::Result<TransferOutcome>,
) -> anyhow::Result<TransferOutcome> {
    use crate::operation_center::{PromptAction, PromptKind};
    let Some(representative) = paths.first().cloned() else {
        return Ok(TransferOutcome::Completed);
    };
    let known_bytes = (paths.len() <= 256)
        .then(|| {
            paths.iter().try_fold(0u64, |acc, path| {
                let meta = std::fs::symlink_metadata(path).ok()?;
                meta.is_file().then(|| acc.saturating_add(meta.len()))
            })
        })
        .flatten();
    let response = worker.prompt(PromptKind::ConfirmPermanentDelete {
        representative,
        items: paths.len() as u64,
        reason,
        known_bytes,
    });
    if worker.is_cancelled() {
        return Ok(TransferOutcome::Aborted);
    }
    let response = response.context("完全削除を確認できるウィンドウがありません")?;
    if response.action != PromptAction::Confirm {
        return Ok(TransferOutcome::Aborted);
    }
    delete(paths, worker)
}

pub(crate) fn validate_delete_arguments(paths: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "削除する項目がありません");
    for path in paths {
        anyhow::ensure!(path.is_absolute(), "削除には絶対パスが必要です");
        anyhow::ensure!(
            path.file_name().is_some() && path.parent().is_some(),
            "ドライブやファイルシステム全体は削除できません"
        );
        anyhow::ensure!(
            !path.as_os_str().as_encoded_bytes().contains(&0)
                && !path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
            "削除するパスに使用できない文字や親フォルダー参照があります"
        );
    }
    Ok(())
}

/// Resolve only the parent: deleting a link must delete the link, including a dangling
/// link, rather than following it to its target. Reparse directories are not ancestors
/// for selection deduplication. The resolved identity is only for validation/deduplication;
/// Shell receives the original path, since Rust canonicalization produces a Windows
/// verbatim namespace path that SHCreateItemFromParsingName need not accept.
fn prepare_deletion(paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    validate_delete_arguments(paths)?;
    let mut sources = Vec::with_capacity(paths.len());
    let mut seen = std::collections::HashSet::with_capacity(paths.len());
    for path in paths {
        let parent = path
            .parent()
            .context("項目の親フォルダーを確認できません")?
            .canonicalize()
            .with_context(|| format!("親フォルダーを開けません: {}", path.display()))?;
        let source = parent.join(path.file_name().context("項目名を確認できません")?);
        let metadata = std::fs::symlink_metadata(&source)
            .with_context(|| format!("削除する項目を開けません: {}", path.display()))?;
        #[cfg(windows)]
        let is_link = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
        };
        #[cfg(not(windows))]
        let is_link = metadata.file_type().is_symlink();
        if seen.insert(source.clone()) {
            sources.push((path.clone(), source, metadata.is_dir() && !is_link));
        }
    }
    let directories: std::collections::HashSet<&Path> = sources
        .iter()
        .filter(|(_, _, is_directory)| *is_directory)
        .map(|(_, identity, _)| identity.as_path())
        .collect();
    Ok(sources
        .iter()
        .filter(|(_, identity, _)| {
            !identity
                .ancestors()
                .skip(1)
                .any(|ancestor| directories.contains(ancestor))
        })
        .map(|(path, _, _)| path.clone())
        .collect())
}

#[cfg(target_os = "macos")]
fn macos_trash(paths: &[PathBuf], worker: &JobWorker) -> anyhow::Result<TransferOutcome> {
    use crate::operation_center::JobIssue;
    use std::{
        ffi::{CStr, CString, c_char},
        os::unix::ffi::OsStrExt,
    };

    unsafe extern "C" {
        fn skylis_trash_item(path: *const c_char, error: *mut c_char, capacity: usize) -> bool;
    }

    for (index, path) in paths.iter().enumerate() {
        if worker.is_cancelled() {
            return Ok(TransferOutcome::Aborted);
        }
        report_delete_items(worker, paths.len(), index, Some(path));
        let path_bytes = CString::new(path.as_os_str().as_bytes())?;
        let mut error = [0 as c_char; 1024];
        if !unsafe { skylis_trash_item(path_bytes.as_ptr(), error.as_mut_ptr(), error.len()) } {
            let details = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
            worker.issue(JobIssue {
                item: Some(path.to_string_lossy().into_owned()),
                message: if details.is_empty() {
                    "ゴミ箱へ移動できませんでした".into()
                } else {
                    details.into_owned()
                },
            });
        }
        report_delete_items(worker, paths.len(), index + 1, Some(path));
    }
    Ok(TransferOutcome::Completed)
}

#[cfg(any(windows, target_os = "macos"))]
fn report_delete_items(worker: &JobWorker, total: usize, done: usize, current: Option<&Path>) {
    use crate::operation_center::{JobProgress, ProgressMeasure, TotalState};
    worker.report(JobProgress {
        measure: ProgressMeasure::Measured {
            bytes_done: 0,
            bytes_total: TotalState::Unknown,
            items_done: done as u64,
            items_total: TotalState::Known(total as u64),
            bytes_per_sec: None,
            eta_seconds: None,
        },
        current_item: current.map(|path| path.to_string_lossy().into_owned()),
        ..JobProgress::default()
    });
}

pub(crate) fn transfer(
    paths: &[PathBuf],
    destination: &Path,
    kind: TransferKind,
    engine: FileTransferEngine,
    worker: &mut JobWorker,
) -> TransferResult {
    (|| {
        let prepared = prepare(paths, destination)?;
        if worker.is_cancelled() {
            return Ok(TransferOutcome::Aborted);
        }
        #[cfg(windows)]
        {
            windows::transfer(&prepared, kind, engine, worker)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = engine;
            if kind != TransferKind::Automatic {
                return macos_transfer(&prepared, kind);
            }

            let kinds: Vec<_> = prepared
                .paths
                .iter()
                .map(|path| volume::resolve(kind, path, &prepared.destination))
                .collect();
            worker.report_transfer_kind(kinds.iter().copied());
            for (path, resolved) in prepared.paths.iter().zip(kinds) {
                let single = Prepared {
                    paths: vec![path.clone()],
                    destination: prepared.destination.clone(),
                };
                if macos_transfer(&single, resolved)? == TransferOutcome::Aborted {
                    return Ok(TransferOutcome::Aborted);
                }
            }
            Ok(TransferOutcome::Completed)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (prepared, kind, engine);
            anyhow::bail!("このOSのファイル操作にはまだ対応していません")
        }
    })()
    .map_err(|error: anyhow::Error| format!("{error:#}"))
}

struct Prepared {
    paths: Vec<PathBuf>,
    destination: PathBuf,
}

pub(crate) fn validate_arguments(paths: &[PathBuf], destination: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "コピー・移動する項目がありません");
    for path in paths.iter().map(PathBuf::as_path).chain(Some(destination)) {
        anyhow::ensure!(path.is_absolute(), "コピー・移動には絶対パスが必要です");
        anyhow::ensure!(
            !path.as_os_str().as_encoded_bytes().contains(&0),
            "パスに使用できない文字があります"
        );
    }
    Ok(())
}

/// Filesystem resolution is intentionally performed only on the transfer worker.
fn prepare(paths: &[PathBuf], destination: &Path) -> anyhow::Result<Prepared> {
    validate_arguments(paths, destination)?;
    let target = destination
        .canonicalize()
        .with_context(|| format!("保存先を開けません: {}", destination.display()))?;
    anyhow::ensure!(target.is_dir(), "保存先はフォルダーを指定してください");
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("項目を開けません: {}", path.display()))?;
        let resolved = path
            .canonicalize()
            .with_context(|| format!("項目を確認できません: {}", path.display()))?;
        let link = metadata.file_type().is_symlink();
        let is_dir = resolved.is_dir();
        let name = path
            .file_name()
            .context("ドライブやファイルシステム全体はコピー・移動できません")?;
        anyhow::ensure!(
            target != resolved && !(is_dir && target.starts_with(&resolved)),
            "フォルダー自身やその内部へコピー・移動できません: {}",
            path.display()
        );
        if let Ok(existing) = target.join(name).canonicalize() {
            anyhow::ensure!(
                existing != resolved,
                "同じ場所へコピー・移動することはできません: {}",
                path.display()
            );
        }
        if !sources
            .iter()
            .any(|(original, _, _, _): &(PathBuf, PathBuf, bool, bool)| original == path)
        {
            sources.push((path.clone(), resolved, is_dir, link));
        }
    }
    // A folder plus one of its children is one operation, not two. Keep symbolic links as
    // distinct entries even if they resolve to the same target as another selected item.
    let paths = sources
        .iter()
        .filter(|(path, resolved, _, link)| {
            !sources.iter().any(|(other, parent, is_dir, parent_link)| {
                other != path
                    && *is_dir
                    && !*parent_link
                    && !*link
                    && resolved != parent
                    && resolved.starts_with(parent)
            })
        })
        .map(|(path, _, _, _)| path.clone())
        .collect();
    Ok(Prepared {
        paths,
        destination: destination.to_path_buf(),
    })
}

#[cfg(target_os = "macos")]
fn macos_transfer(prepared: &Prepared, kind: TransferKind) -> anyhow::Result<TransferOutcome> {
    // A fixed script receives paths only as argv: quoting and shell syntax in filenames cannot
    // become executable code. Finder preserves macOS metadata. Replacement stays disabled.
    // macOS may ask for Automation permission; denial is reported to the application.
    const SCRIPT: &str = r#"on run argv
        set destinationFolder to (POSIX file (item 1 of argv)) as alias
        set operationKind to item 2 of argv
        with timeout of 86400 seconds
        try
            repeat with sourcePath in (items 3 thru -1 of argv)
                set sourceItem to POSIX file (contents of sourcePath)
                tell application "Finder"
                    if operationKind is "copy" then
                        duplicate sourceItem to destinationFolder without replacing
                    else
                        move sourceItem to destinationFolder without replacing
                    end if
                end tell
            end repeat
            return "completed"
        on error errorText number errorNumber
            if errorNumber is -128 then return "aborted"
            error errorText number errorNumber
        end try
        end timeout
    end run"#;
    let output = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(SCRIPT)
        .arg("--")
        .arg(&prepared.destination)
        .arg(match kind {
            TransferKind::Copy => "copy",
            TransferKind::Move => "move",
            TransferKind::Automatic => unreachable!(),
        })
        .args(&prepared.paths)
        .output()
        .context("Finderのファイル操作を開始できません")?;
    anyhow::ensure!(
        output.status.success(),
        "Finderのファイル操作に失敗しました: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    match String::from_utf8_lossy(&output.stdout).trim() {
        "completed" => {
            // Do not claim Move succeeded if a volume/provider retained the original.
            // Never try to fix that by recursively deleting the source ourselves.
            if kind == TransferKind::Move {
                anyhow::ensure!(
                    prepared.paths.iter().all(|path| {
                        std::fs::symlink_metadata(path)
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                    }),
                    "Finderの移動後も元の場所に項目が残っています。元と保存先を確認してください"
                );
            }
            Ok(TransferOutcome::Completed)
        }
        "aborted" => Ok(TransferOutcome::Aborted),
        _ => anyhow::bail!("Finderの操作結果を確認できません"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "skylis-transfer-{}-{timestamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn transfer_rejects_invalid_arguments() {
        let fixture = Fixture::new();
        assert!(validate_arguments(&[], &fixture.0).is_err());
        assert!(validate_arguments(&[PathBuf::from("relative")], &fixture.0).is_err());
        assert!(validate_arguments(&[fixture.0.join("bad\0name")], &fixture.0).is_err());
    }

    #[test]
    fn transfer_rejects_self_and_descendant_destinations_before_mutation() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let nested = source.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let file = source.join("file.txt");
        std::fs::write(&file, "original").unwrap();
        assert!(prepare(std::slice::from_ref(&source), &source).is_err());
        assert!(prepare(std::slice::from_ref(&source), &nested).is_err());
        assert!(prepare(std::slice::from_ref(&file), &source).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }

    #[test]
    fn transfer_validates_all_sources_before_accepting_a_batch() {
        let fixture = Fixture::new();
        let destination = fixture.0.join("destination");
        std::fs::create_dir(&destination).unwrap();
        let file = fixture.0.join("file.txt");
        std::fs::write(&file, "original").unwrap();
        assert!(prepare(&[file.clone(), fixture.0.join("missing")], &destination).is_err());
        assert!(!destination.join("file.txt").exists());
        let prepared = prepare(std::slice::from_ref(&file), &destination).unwrap();
        assert_eq!(prepared.paths, [file]);
    }

    #[test]
    fn transfer_deduplicates_nested_selection_and_allows_native_conflict_handling() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let destination = fixture.0.join("destination");
        std::fs::create_dir_all(destination.join("source")).unwrap();
        std::fs::create_dir(&source).unwrap();
        let file = source.join("file.txt");
        std::fs::write(&file, "original").unwrap();
        let prepared = prepare(&[file, source.clone(), source.clone()], &destination).unwrap();
        assert_eq!(prepared.paths, [source]);
    }

    #[test]
    fn deletion_rejects_roots_relative_paths_and_traversal_before_mutation() {
        let fixture = Fixture::new();
        assert!(validate_delete_arguments(&[]).is_err());
        assert!(validate_delete_arguments(&[PathBuf::from("relative")]).is_err());
        assert!(validate_delete_arguments(&[fixture.0.join("bad\0name")]).is_err());
        assert!(validate_delete_arguments(&[fixture.0.join("..").join("outside")]).is_err());
        let root = fixture.0.ancestors().last().unwrap().to_path_buf();
        assert!(validate_delete_arguments(&[root]).is_err());
    }

    #[test]
    fn deletion_validates_every_item_and_deduplicates_parent_child_selection() {
        let fixture = Fixture::new();
        let parent = fixture.0.join("日本語のフォルダー");
        std::fs::create_dir(&parent).unwrap();
        let child = parent.join("keep.txt");
        std::fs::write(&child, "unchanged").unwrap();
        let sibling = fixture.0.join("日本語のフォルダー-extra.txt");
        std::fs::write(&sibling, "sibling").unwrap();
        assert!(prepare_deletion(&[child.clone(), fixture.0.join("missing")]).is_err());
        let prepared = prepare_deletion(&[
            child.clone(),
            parent.clone(),
            parent.clone(),
            sibling.clone(),
        ])
        .unwrap();
        assert_eq!(prepared, [parent, sibling.clone()]);
        assert_eq!(std::fs::read_to_string(&child).unwrap(), "unchanged");
        assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "sibling");
    }

    #[cfg(windows)]
    #[test]
    fn deletion_keeps_shell_paths_while_deduplicating_canonical_identities() {
        let fixture = Fixture::new();
        let path = fixture.0.join("日本語.txt");
        std::fs::write(&path, "unchanged").unwrap();
        let canonical = path.canonicalize().unwrap();
        assert_ne!(path, canonical);
        let prepared = prepare_deletion(&[path.clone(), canonical]).unwrap();
        assert_eq!(prepared, [path.clone()]);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "unchanged");
    }

    /// Read-only Shell integration: construction succeeds, but no deletion is queued.
    #[cfg(windows)]
    #[test]
    fn prepared_deletion_sources_can_be_opened_by_windows_shell() {
        use ::windows::{
            Win32::{
                System::Ole::{OleInitialize, OleUninitialize},
                UI::Shell::{IShellItem, SHCreateItemFromParsingName},
            },
            core::PCWSTR,
        };
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    OleUninitialize();
                }
            }
        }
        unsafe {
            OleInitialize(None).unwrap();
        }
        let _apartment = Apartment;
        let fixture = Fixture::new();
        let fixture_file = fixture.0.join("削除準備の確認.txt");
        std::fs::write(&fixture_file, "preserve this file").unwrap();
        let input = std::env::var_os("SKYLIS_DELETE_SHELL_SMOKE_PATH")
            .map(PathBuf::from)
            .unwrap_or(fixture_file.clone());
        for path in prepare_deletion(&[input]).unwrap() {
            let wide = crate::native_shell::windows_path_wide(&path).unwrap();
            let _: IShellItem =
                unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(fixture_file).unwrap(),
            "preserve this file"
        );
    }
}
