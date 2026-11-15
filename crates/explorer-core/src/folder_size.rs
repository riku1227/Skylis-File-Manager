//! Bounded, cancellable directory traversal for background size workers.
use std::{
    fs, io,
    path::Path,
    time::{Duration, Instant},
};

use crate::CancelToken;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderSizeProgress {
    /// Logical file lengths, not allocated disk space. Hard links count once per entry.
    pub bytes: u64,
    pub files: u64,
    /// Successfully opened directories, including the root.
    pub directories: u64,
    /// Inaccessible or concurrently removed entries/subdirectories.
    pub skipped_errors: u64,
    /// Links and Windows reparse points intentionally excluded from traversal.
    pub skipped_links: u64,
    /// Subdirectories excluded to bound directory handles and traversal memory.
    pub depth_limited: u64,
}

impl FolderSizeProgress {
    /// The scan completed but cannot claim an exact total for the traversable tree.
    pub fn is_partial(self) -> bool {
        self.skipped_errors != 0 || self.depth_limited != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderSizeStatus {
    Complete,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderSizeResult {
    pub progress: FolderSizeProgress,
    pub status: FolderSizeStatus,
}

/// Blocking scan: call on a bounded background worker, never the UI thread.
///
/// Only regular file lengths are summed. Symlinks, junctions, cloud placeholders
/// and all other Windows reparse points are excluded, so this does not hydrate
/// cloud files or cross link cycles. The caller must label partial totals when
/// `progress.is_partial()` is true. An inaccessible root is an error, not zero.
///
/// Progress is emitted at most every 100 ms, plus one final snapshot. Returning
/// false stops the scan immediately. Cancellation is cooperative between I/O
/// calls; a filesystem call already blocked inside the OS cannot be interrupted
/// by this token. Do not spawn a replacement worker for every navigation.
///
/// Traversal retains at most 128 open directory iterators and no breadth-sized
/// path queue. Deeper subtrees are reported through `depth_limited`.
pub fn scan_folder_size(
    path: &Path,
    cancel: &CancelToken,
    emit: impl FnMut(FolderSizeProgress) -> bool,
) -> io::Result<FolderSizeResult> {
    scan_with_limits(path, cancel, emit, Duration::from_millis(100), 128)
}

fn scan_with_limits(
    path: &Path,
    cancel: &CancelToken,
    mut emit: impl FnMut(FolderSizeProgress) -> bool,
    interval: Duration,
    max_depth: usize,
) -> io::Result<FolderSizeResult> {
    let mut progress = FolderSizeProgress::default();
    let cancelled = |progress| FolderSizeResult {
        progress,
        status: FolderSizeStatus::Cancelled,
    };
    if cancel.is_cancelled() {
        return Ok(cancelled(progress));
    }
    let metadata = fs::symlink_metadata(path)?;
    if excluded_link(&metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "size calculation requires a real directory, not a file or link",
        ));
    }
    if cancel.is_cancelled() {
        return Ok(cancelled(progress));
    }
    let mut stack = vec![fs::read_dir(path)?];
    progress.directories = 1;
    let mut last_emit = Instant::now();
    while !stack.is_empty() {
        if cancel.is_cancelled() {
            return Ok(cancelled(progress));
        }
        let Some(entry) = stack.last_mut().expect("nonempty stack").next() else {
            stack.pop();
            continue;
        };
        match entry {
            Err(_) => progress.skipped_errors += 1,
            Ok(entry) => {
                if cancel.is_cancelled() {
                    return Ok(cancelled(progress));
                }
                // DirEntry::metadata does not follow symbolic links. On Windows
                // it reuses enumeration metadata instead of adding a stat per file.
                match entry.metadata() {
                    Err(_) => progress.skipped_errors += 1,
                    Ok(metadata) if excluded_link(&metadata) => progress.skipped_links += 1,
                    Ok(metadata) if metadata.is_file() => {
                        progress.bytes = progress.bytes.saturating_add(metadata.len());
                        progress.files += 1;
                    }
                    Ok(metadata) if metadata.is_dir() => {
                        if stack.len() >= max_depth.max(1) {
                            progress.depth_limited += 1;
                        } else {
                            if cancel.is_cancelled() {
                                return Ok(cancelled(progress));
                            }
                            match fs::read_dir(entry.path()) {
                                Ok(children) => {
                                    stack.push(children);
                                    progress.directories += 1;
                                }
                                Err(_) => progress.skipped_errors += 1,
                            }
                        }
                    }
                    Ok(_) => {} // Devices and sockets have no regular-file size.
                }
            }
        }
        if last_emit.elapsed() >= interval {
            if !emit(progress) {
                return Ok(cancelled(progress));
            }
            last_emit = Instant::now();
        }
    }
    if cancel.is_cancelled() || !emit(progress) || cancel.is_cancelled() {
        return Ok(cancelled(progress));
    }
    Ok(FolderSizeResult {
        progress,
        status: FolderSizeStatus::Complete,
    })
}

fn excluded_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "skylis-size-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn counts_nested_regular_files_and_reports_final_snapshot() {
        let dir = TempDir::new();
        fs::create_dir_all(dir.0.join("資料/empty")).unwrap();
        fs::write(dir.0.join("a"), [0; 7]).unwrap();
        fs::write(dir.0.join("資料/b"), [0; 13]).unwrap();
        let mut snapshots = Vec::new();
        let result = scan_folder_size(&dir.0, &CancelToken::new(), |s| {
            snapshots.push(s);
            true
        })
        .unwrap();
        assert_eq!(result.status, FolderSizeStatus::Complete);
        assert_eq!(result.progress.bytes, 20);
        assert_eq!(result.progress.files, 2);
        assert_eq!(result.progress.directories, 3);
        assert_eq!(result.progress.skipped_errors, 0);
        assert_eq!(snapshots.last(), Some(&result.progress));
    }

    #[test]
    fn empty_directory_is_complete_zero_not_unknown() {
        let dir = TempDir::new();
        let result = scan_folder_size(&dir.0, &CancelToken::new(), |_| true).unwrap();
        assert_eq!(result.status, FolderSizeStatus::Complete);
        assert_eq!(result.progress.bytes, 0);
        assert_eq!(result.progress.files, 0);
        assert_eq!(result.progress.directories, 1);
    }

    #[test]
    fn cancellation_before_start_does_no_filesystem_work() {
        let dir = TempDir::new();
        let token = CancelToken::new();
        token.cancel();
        let result = scan_folder_size(&dir.0.join("missing"), &token, |_| true).unwrap();
        assert_eq!(result.status, FolderSizeStatus::Cancelled);
        assert_eq!(result.progress, FolderSizeProgress::default());
    }

    #[test]
    fn cancellation_stops_before_remaining_files() {
        let dir = TempDir::new();
        for i in 0..30 {
            fs::write(dir.0.join(i.to_string()), [0; 2]).unwrap();
        }
        let token = CancelToken::new();
        let result = scan_with_limits(
            &dir.0,
            &token,
            |s| {
                if s.files > 0 {
                    token.cancel();
                }
                true
            },
            Duration::ZERO,
            128,
        )
        .unwrap();
        assert_eq!(result.status, FolderSizeStatus::Cancelled);
        assert!(result.progress.files > 0 && result.progress.files < 30);
    }

    #[test]
    fn closed_progress_receiver_cancels_work() {
        let dir = TempDir::new();
        for i in 0..30 {
            fs::write(dir.0.join(i.to_string()), [0; 2]).unwrap();
        }
        let result =
            scan_with_limits(&dir.0, &CancelToken::new(), |_| false, Duration::ZERO, 128).unwrap();
        assert_eq!(result.status, FolderSizeStatus::Cancelled);
        assert!(result.progress.files < 30);
    }

    #[test]
    fn progress_is_throttled_and_streamed_totals_are_monotonic() {
        let dir = TempDir::new();
        for i in 0..10 {
            fs::write(dir.0.join(i.to_string()), [0; 2]).unwrap();
        }
        let mut updates = Vec::new();
        scan_with_limits(
            &dir.0,
            &CancelToken::new(),
            |s| {
                updates.push(s);
                true
            },
            Duration::from_secs(3600),
            128,
        )
        .unwrap();
        assert_eq!(updates.len(), 1, "fast scans only need the final snapshot");
        assert_eq!(updates[0].bytes, 20);

        updates.clear();
        scan_with_limits(
            &dir.0,
            &CancelToken::new(),
            |s| {
                updates.push(s);
                true
            },
            Duration::ZERO,
            128,
        )
        .unwrap();
        assert!(updates.len() > 1);
        assert!(
            updates
                .windows(2)
                .all(|pair| { pair[0].bytes <= pair[1].bytes && pair[0].files <= pair[1].files })
        );
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_descendant_reports_partial_without_losing_readable_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let private = dir.0.join("private");
        fs::create_dir(&private).unwrap();
        fs::write(private.join("secret"), [0; 9]).unwrap();
        fs::write(dir.0.join("readable"), [0; 3]).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0)).unwrap();
        let is_denied = fs::read_dir(&private).is_err();
        let result = scan_folder_size(&dir.0, &CancelToken::new(), |_| true);
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        if !is_denied {
            eprintln!("Permission test unavailable for privileged process");
            return;
        }
        let result = result.unwrap();
        assert_eq!(result.status, FolderSizeStatus::Complete);
        assert_eq!(result.progress.bytes, 3);
        assert_eq!(result.progress.skipped_errors, 1);
        assert!(result.progress.is_partial());
    }

    #[test]
    fn inaccessible_root_is_an_error_not_a_zero_size() {
        let dir = TempDir::new();
        assert_eq!(
            scan_folder_size(&dir.0.join("missing"), &CancelToken::new(), |_| true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        fs::write(dir.0.join("file"), "x").unwrap();
        assert_eq!(
            scan_folder_size(&dir.0.join("file"), &CancelToken::new(), |_| true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn traversal_resource_limit_is_reported_as_partial() {
        let dir = TempDir::new();
        fs::create_dir_all(dir.0.join("one/two/three")).unwrap();
        fs::write(dir.0.join("one/a"), [0; 5]).unwrap();
        fs::write(dir.0.join("one/two/b"), [0; 9]).unwrap();
        let result =
            scan_with_limits(&dir.0, &CancelToken::new(), |_| true, Duration::ZERO, 2).unwrap();
        assert_eq!(result.status, FolderSizeStatus::Complete);
        assert_eq!(result.progress.bytes, 5);
        assert_eq!(result.progress.depth_limited, 1);
        assert!(result.progress.is_partial());
    }

    #[test]
    fn hard_links_count_each_logical_directory_entry() {
        let dir = TempDir::new();
        fs::write(dir.0.join("a"), [0; 5]).unwrap();
        fs::hard_link(dir.0.join("a"), dir.0.join("b")).unwrap();
        let result = scan_folder_size(&dir.0, &CancelToken::new(), |_| true).unwrap();
        assert_eq!(result.progress.bytes, 10);
        assert_eq!(result.progress.files, 2);
    }

    #[test]
    fn symlink_cycles_are_skipped_and_root_links_are_rejected() {
        let dir = TempDir::new();
        fs::write(dir.0.join("a"), [0; 5]).unwrap();
        let link = dir.0.join("cycle");
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&dir.0, &link);
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&dir.0, &link);
        if let Err(error) = created {
            #[cfg(windows)]
            if matches!(error.kind(), io::ErrorKind::PermissionDenied)
                || error.raw_os_error() == Some(1314)
            {
                eprintln!("Symlink test unavailable without Windows symlink privileges");
                return;
            }
            panic!("create test symlink: {error}");
        }
        let result = scan_folder_size(&dir.0, &CancelToken::new(), |_| true).unwrap();
        assert_eq!(result.progress.bytes, 5);
        assert_eq!(result.progress.skipped_links, 1);
        assert_eq!(
            scan_folder_size(&link, &CancelToken::new(), |_| true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        fs::remove_dir(&link)
            .or_else(|_| fs::remove_file(&link))
            .unwrap();
    }
}
