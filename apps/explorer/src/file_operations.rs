//! Native file mutations. File operations outlive navigation; the OS progress dialog owns
//! cancellation. Unlike a preview or folder-size scan, dropping the receiver must not stop
//! a user-requested move halfway through. There is at most one mutation worker per process.
pub use crate::file_operation_progress::{
    OperationProgress, progress_snapshot, subscribe_progress,
};
use anyhow::Context as _;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

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
    /// Some items may already have changed. Refresh every affected listing.
    Aborted,
}

pub type TransferResult = Result<TransferOutcome, String>;
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn has_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteKind {
    Recycle,
    /// Windows presents its explicit permanent-deletion warning before executing.
    Permanent,
}

pub fn supports_delete(kind: DeleteKind) -> bool {
    cfg!(windows) || (cfg!(target_os = "macos") && kind == DeleteKind::Recycle)
}

/// Queues one selection snapshot. Filesystem validation and all native calls run on the
/// worker. A failed/rejected batch is never retried as a different deletion kind.
pub fn delete(
    paths: Vec<PathBuf>,
    kind: DeleteKind,
    window: &gpui::Window,
) -> anyhow::Result<async_channel::Receiver<TransferResult>> {
    validate_delete_arguments(&paths)?;
    anyhow::ensure!(
        supports_delete(kind),
        "このOSでは指定した削除方法に対応していません"
    );
    let owner = native_owner(window)?;
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        anyhow::bail!("ファイル操作が進行中です。完了またはキャンセル後に操作してください");
    }
    let (sender, receiver) = async_channel::bounded(1);
    if let Err(error) = std::thread::Builder::new()
        .name("file-delete".into())
        .spawn(move || {
            let _guard = ActiveGuard;
            let _progress = crate::file_operation_progress::begin(match kind {
                DeleteKind::Recycle => "ゴミ箱へ移動",
                DeleteKind::Permanent => "削除",
            });
            let result = prepare_deletion(&paths)
                .and_then(|paths| execute_deletion(&paths, kind, owner))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.try_send(result);
        })
    {
        ACTIVE.store(false, Ordering::Release);
        return Err(error).context("削除を開始できません");
    }
    Ok(receiver)
}

fn native_owner(window: &gpui::Window) -> anyhow::Result<isize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("ウィンドウを取得できません: {error}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("Windowsウィンドウが必要です");
        };
        Ok(raw.hwnd.get())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Ok(0)
    }
}

fn validate_delete_arguments(paths: &[PathBuf]) -> anyhow::Result<()> {
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

fn execute_deletion(
    paths: &[PathBuf],
    kind: DeleteKind,
    owner: isize,
) -> anyhow::Result<TransferOutcome> {
    #[cfg(windows)]
    {
        windows_delete(paths, kind, owner)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = owner;
        anyhow::ensure!(
            kind == DeleteKind::Recycle,
            "macOSの完全削除にはまだ対応していません"
        );
        macos_trash(paths)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (paths, kind, owner);
        anyhow::bail!("このOSの削除にはまだ対応していません")
    }
}

#[cfg(windows)]
fn windows_delete_flags(kind: DeleteKind) -> windows::Win32::UI::Shell::FILEOPERATION_FLAGS {
    use windows::Win32::UI::Shell::*;
    // WANTNUKEWARNING asks before destruction if recycling is unavailable, even when
    // Explorer's normal recycle confirmation is disabled. Do not answer Yes on its behalf.
    match kind {
        DeleteKind::Recycle => {
            FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOF_WANTNUKEWARNING
        }
        DeleteKind::Permanent => FOF_WANTNUKEWARNING,
    }
}

#[cfg(windows)]
fn windows_delete(
    paths: &[PathBuf],
    kind: DeleteKind,
    owner: isize,
) -> anyhow::Result<TransferOutcome> {
    use windows::{
        Win32::{
            Foundation::HWND,
            System::{
                Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
                Ole::{OleInitialize, OleUninitialize},
            },
            UI::{Shell::*, WindowsAndMessaging::IsWindow},
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
        OleInitialize(None)?;
    }
    let _apartment = Apartment;
    let owner = HWND(owner as *mut std::ffi::c_void);
    anyhow::ensure!(
        unsafe { IsWindow(Some(owner)).as_bool() },
        "操作元のウィンドウが閉じられました"
    );
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(owner)?;
        operation.SetOperationFlags(windows_delete_flags(kind))?;
        // Progress observation is optional; failure must never change native file behavior.
        let _progress_sink = crate::file_operation_progress::Subscription::new(&operation).ok();
        // Build every Shell item before submitting work, so a malformed later item does
        // not cause an earlier item to be deleted before validation has completed.
        let items: Vec<IShellItem> = paths
            .iter()
            .map(|path| {
                let wide = crate::native_shell::windows_path_wide(path)?;
                SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                    .with_context(|| format!("Shellが項目を開けません: {}", path.display()))
            })
            .collect::<anyhow::Result<_>>()?;
        for item in &items {
            operation.DeleteItem(item, None)?;
        }
        let result = operation.PerformOperations();
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Ok(TransferOutcome::Aborted);
        }
        result.context("削除が完了しませんでした。一部の項目は処理済みの可能性があります")?;
    }
    Ok(TransferOutcome::Completed)
}

#[cfg(target_os = "macos")]
fn macos_trash(paths: &[PathBuf]) -> anyhow::Result<TransferOutcome> {
    // Pass paths as argv, never interpolate them into AppleScript or shell source.
    // Finder's delete command moves items to Trash; there is no permanent-delete retry.
    const SCRIPT: &str = r#"on run argv
        with timeout of 86400 seconds
        try
            repeat with sourcePath in argv
                set sourceItem to (POSIX file (contents of sourcePath))
                tell application "Finder" to delete sourceItem
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
        .args(paths)
        .output()
        .context("Finderの削除を開始できません")?;
    anyhow::ensure!(
        output.status.success(),
        "Finderの削除に失敗しました: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    match String::from_utf8_lossy(&output.stdout).trim() {
        "completed" => Ok(TransferOutcome::Completed),
        "aborted" => Ok(TransferOutcome::Aborted),
        _ => anyhow::bail!("Finderの削除結果を確認できません"),
    }
}

pub fn transfer(
    paths: Vec<PathBuf>,
    destination: PathBuf,
    kind: TransferKind,
    window: &gpui::Window,
) -> anyhow::Result<async_channel::Receiver<TransferResult>> {
    validate_arguments(&paths, &destination)?;
    #[cfg(windows)]
    let owner = {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("ウィンドウを取得できません: {error}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("Windowsウィンドウが必要です");
        };
        raw.hwnd.get()
    };
    #[cfg(not(windows))]
    let owner = {
        let _ = window;
        0_isize
    };
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        anyhow::bail!("ファイル操作が進行中です。完了またはキャンセル後に操作してください");
    }
    let (sender, receiver) = async_channel::bounded(1);
    if let Err(error) = std::thread::Builder::new()
        .name("file-transfer".into())
        .spawn(move || {
            let _guard = ActiveGuard;
            let _progress = crate::file_operation_progress::begin(match kind {
                TransferKind::Copy => "コピー",
                TransferKind::Move => "移動",
                TransferKind::Automatic => "ファイル操作",
            });
            let result = prepare(&paths, &destination)
                .and_then(|prepared| execute(&prepared, kind, owner))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.try_send(result);
        })
    {
        ACTIVE.store(false, Ordering::Release);
        return Err(error).context("ファイル操作を開始できません");
    }
    Ok(receiver)
}

struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

struct Prepared {
    paths: Vec<PathBuf>,
    destination: PathBuf,
}

fn validate_arguments(paths: &[PathBuf], destination: &Path) -> anyhow::Result<()> {
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

fn execute(
    prepared: &Prepared,
    kind: TransferKind,
    owner: isize,
) -> anyhow::Result<TransferOutcome> {
    #[cfg(windows)]
    {
        windows_transfer(prepared, kind, owner)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = owner;
        macos_transfer(prepared, kind)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (prepared, kind, owner);
        anyhow::bail!("このOSのファイル操作にはまだ対応していません")
    }
}

#[cfg(windows)]
fn windows_transfer(
    prepared: &Prepared,
    kind: TransferKind,
    owner: isize,
) -> anyhow::Result<TransferOutcome> {
    use std::ffi::c_void;
    use windows::{
        Win32::{
            Foundation::HWND,
            System::{
                Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
                Ole::{OleInitialize, OleUninitialize},
            },
            UI::{Shell::*, WindowsAndMessaging::IsWindow},
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
        OleInitialize(None)?;
    }
    let _apartment = Apartment;
    let owner = HWND(owner as *mut c_void);
    anyhow::ensure!(
        unsafe { IsWindow(Some(owner)).as_bool() },
        "操作元のウィンドウが閉じられました"
    );
    let shell_item = |path: &Path| -> anyhow::Result<IShellItem> {
        let wide = crate::native_shell::windows_path_wide(path)?;
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
            .with_context(|| format!("Shellが項目を開けません: {}", path.display()))
    };
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(owner)?;
        // Keep native conflict/progress/error UI. Never set FOF_NOCONFIRMATION, which would
        // silently answer Yes to replacement prompts. Windows manages metadata and records
        // native undo information; Skylis does not yet expose an application Undo command.
        operation.SetOperationFlags(FOFX_ADDUNDORECORD | FOF_NOCONFIRMMKDIR)?;
        let _progress_sink = crate::file_operation_progress::Subscription::new(&operation).ok();
        let destination = shell_item(&prepared.destination)?;
        for path in &prepared.paths {
            let source = shell_item(path)?;
            match volume::resolve(kind, path, &prepared.destination) {
                TransferKind::Copy => {
                    operation.CopyItem(&source, &destination, PCWSTR::null(), None)?
                }
                TransferKind::Move => {
                    operation.MoveItem(&source, &destination, PCWSTR::null(), None)?
                }
                TransferKind::Automatic => unreachable!("volume policy resolves automatic intent"),
            }
        }
        let result = operation.PerformOperations();
        // PerformOperations may return S_OK for a cancelled operation. Query either way.
        let aborted = operation.GetAnyOperationsAborted()?;
        if aborted.as_bool() {
            return Ok(TransferOutcome::Aborted);
        }
        result
            .context("ファイル操作が完了しませんでした。一部の項目は処理済みの可能性があります")?;
    }
    Ok(TransferOutcome::Completed)
}

#[cfg(target_os = "macos")]
fn macos_transfer(prepared: &Prepared, kind: TransferKind) -> anyhow::Result<TransferOutcome> {
    if kind == TransferKind::Automatic {
        // 同じバッチ内の別ボリュームも、項目ごとに既定動作を選ぶ。
        for path in &prepared.paths {
            let resolved = volume::resolve(kind, path, &prepared.destination);
            let single = Prepared {
                paths: vec![path.clone()],
                destination: prepared.destination.clone(),
            };
            if macos_transfer(&single, resolved)? == TransferOutcome::Aborted {
                return Ok(TransferOutcome::Aborted);
            }
        }
        return Ok(TransferOutcome::Completed);
    }
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
    use std::sync::atomic::AtomicU64;
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
        use windows::{
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

    #[cfg(windows)]
    #[test]
    fn recycle_and_permanent_delete_keep_explicit_destruction_warnings() {
        use windows::Win32::UI::Shell::*;
        let recycle = windows_delete_flags(DeleteKind::Recycle);
        let permanent = windows_delete_flags(DeleteKind::Permanent);
        assert_ne!(recycle.0 & FOFX_RECYCLEONDELETE.0, 0);
        assert_ne!(recycle.0 & FOF_ALLOWUNDO.0, 0);
        assert_eq!(
            permanent.0 & (FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD).0,
            0
        );
        for flags in [recycle, permanent] {
            assert_ne!(flags.0 & FOF_WANTNUKEWARNING.0, 0);
            assert_eq!(flags.0 & FOF_NOCONFIRMATION.0, 0);
        }
    }
}
