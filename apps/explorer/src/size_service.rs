//! One bounded, application-wide service for folder-size scans.
use explorer_core::{CancelToken, FolderSizeProgress, FolderSizeStatus, scan_folder_size};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
};

#[derive(Clone, Debug)]
pub(crate) enum SizeUpdateKind {
    Progress(FolderSizeProgress),
    Complete(FolderSizeProgress),
    Failed(String),
    ManualOnly,
}

pub(crate) struct SizeUpdate {
    pub path: PathBuf,
    pub generation: u64,
    pub kind: SizeUpdateKind,
}

pub(crate) struct SizeRequest {
    pub path: PathBuf,
    pub generation: u64,
    pub cancel: CancelToken,
    pub automatic: bool,
    pub updates: async_channel::Sender<SizeUpdate>,
}

struct Pending {
    jobs: VecDeque<SizeRequest>,
    workers: usize,
}
struct Service {
    pending: Mutex<Pending>,
    wake: Condvar,
}

/// Called on the UI thread: only bounded queue bookkeeping, no filesystem work.
pub(crate) fn request(request: SizeRequest) -> Result<(), String> {
    static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();
    let service = SERVICE.get_or_init(|| {
        let service = Arc::new(Service {
            pending: Mutex::new(Pending {
                jobs: VecDeque::new(),
                workers: 0,
            }),
            wake: Condvar::new(),
        });
        for index in 0..2 {
            let worker = service.clone();
            if std::thread::Builder::new()
                .name(format!("folder-size-{index}"))
                .spawn(move || worker.run())
                .is_ok()
            {
                service
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .workers += 1;
            }
        }
        service
    });
    let mut pending = service.pending.lock().unwrap_or_else(|e| e.into_inner());
    pending
        .jobs
        .retain(|job| !job.cancel.is_cancelled() && !job.updates.is_closed());
    if pending.workers == 0 {
        return Err("サイズ計算ワーカーを開始できませんでした".into());
    }
    if pending.jobs.len() >= 128 {
        return Err("サイズ計算の待機数が上限に達しました。後でもう一度お試しください".into());
    }
    if request.automatic {
        pending.jobs.push_back(request);
    } else {
        pending.jobs.push_front(request);
    }
    service.wake.notify_one();
    Ok(())
}

impl Service {
    fn run(&self) {
        loop {
            let request = {
                let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(job) = pending.jobs.pop_front() {
                        break job;
                    }
                    pending = self.wake.wait(pending).unwrap_or_else(|e| e.into_inner());
                }
            };
            if request.cancel.is_cancelled() || request.updates.is_closed() {
                continue;
            }
            let emit = |kind| SizeUpdate {
                path: request.path.clone(),
                generation: request.generation,
                kind,
            };
            if request.automatic && !automatic_allowed(&request.path) {
                let _ = request
                    .updates
                    .send_blocking(emit(SizeUpdateKind::ManualOnly));
                continue;
            }
            let result = scan_folder_size(&request.path, &request.cancel, |progress| {
                if request.cancel.is_cancelled() || request.updates.is_closed() {
                    return false;
                }
                // Progress can be coalesced/dropped under UI pressure; final totals cannot.
                let _ = request
                    .updates
                    .try_send(emit(SizeUpdateKind::Progress(progress)));
                true
            });
            if request.cancel.is_cancelled() {
                continue;
            }
            let kind = match result {
                Ok(result) if result.status == FolderSizeStatus::Complete => {
                    SizeUpdateKind::Complete(result.progress)
                }
                Ok(_) => continue,
                Err(error) => SizeUpdateKind::Failed(error.to_string()),
            };
            let _ = request.updates.send_blocking(emit(kind));
        }
    }
}

/// Worker-only volume policy. Network/removable/cloud/link roots need explicit activation.
fn automatic_allowed(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::{
            os::windows::fs::MetadataExt,
            path::{Component, Prefix},
        };
        let drive = match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
                _ => return false,
            },
            _ => return false,
        };
        if !path.is_absolute() {
            return false;
        }
        let root: Vec<u16> = format!("{}:\\", drive as char)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // DRIVE_FIXED = 3. This query stays on the worker even for mapped drives.
        if unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) } != 3 {
            return false;
        }
        for ancestor in path.ancestors() {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            let Ok(metadata) = std::fs::symlink_metadata(ancestor) else {
                return false;
            };
            if !metadata.is_dir() || metadata.file_attributes() & (0x400 | 0x1000) != 0 {
                return false;
            }
        }
        true
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let Ok(path_c) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(path_c.as_ptr(), &mut stats) } != 0
            || stats.f_flags & libc::MNT_LOCAL as u32 == 0
        {
            return false;
        }
        for ancestor in path.ancestors() {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            let Ok(metadata) = std::fs::symlink_metadata(ancestor) else {
                return false;
            };
            if metadata.file_type().is_symlink() {
                return false;
            }
        }
        // File-provider/cloud roots need user initiation even when backed by a local volume.
        !path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some("CloudStorage" | "Mobile Documents")
            )
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn automatic_sizes_never_probe_unc_or_relative_paths() {
        for path in [
            r"\\wsl.localhost\Ubuntu\home",
            r"\\server\share\folder",
            r"\\?\UNC\server\share",
            "relative",
            r"C:relative",
        ] {
            assert!(!automatic_allowed(Path::new(path)));
        }
    }
}
