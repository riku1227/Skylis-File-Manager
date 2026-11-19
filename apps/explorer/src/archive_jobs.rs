//! Dedicated bounded worker for read-only archive inspection.
use explorer_core::{
    CancelToken,
    archive::{self, ArchiveIndex},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock},
};

const PENDING_INSPECTIONS: usize = 8;

pub(crate) type InspectionResult = Result<Arc<ArchiveIndex>, String>;
struct InspectRequest {
    path: PathBuf,
    cancel: CancelToken,
    cached: Option<Arc<ArchiveIndex>>,
    result: async_channel::Sender<InspectionResult>,
}
struct Inspections {
    pending: Mutex<VecDeque<InspectRequest>>,
    wake: Condvar,
    available: bool,
}

/// One metadata worker with eight pending requests. Closing the receiver or cancelling
/// the navigation token discards obsolete work; no filesystem metadata is read here.
pub(crate) fn inspect_async(
    path: PathBuf,
    cancel: CancelToken,
    cached: Option<Arc<ArchiveIndex>>,
) -> Result<async_channel::Receiver<InspectionResult>, String> {
    static SERVICE: OnceLock<Arc<Inspections>> = OnceLock::new();
    let service = SERVICE.get_or_init(|| {
        // Availability is set before publishing the service through OnceLock.
        let mut service = Arc::new(Inspections {
            pending: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            available: true,
        });
        let worker = service.clone();
        if std::thread::Builder::new()
            .name("archive-index".into())
            .spawn(move || worker.run())
            .is_err()
        {
            Arc::get_mut(&mut service)
                .expect("failed worker released its Arc")
                .available = false;
        }
        service
    });
    if !service.available {
        return Err("圧縮ファイルの読み込みワーカーを開始できません".into());
    }
    if cancel.is_cancelled() {
        return Err("圧縮ファイルの読み込みをキャンセルしました".into());
    }
    let mut queue = service
        .pending
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    queue.retain(|request| {
        if request.result.is_closed() {
            request.cancel.cancel();
        }
        !request.cancel.is_cancelled()
    });
    if queue.len() >= PENDING_INSPECTIONS {
        return Err("圧縮ファイルの読み込み待機数が上限（8件）に達しています".into());
    }
    let (result, receiver) = async_channel::bounded(1);
    queue.push_back(InspectRequest {
        path,
        cancel,
        cached,
        result,
    });
    service.wake.notify_one();
    Ok(receiver)
}

impl Inspections {
    fn run(&self) {
        loop {
            let request = {
                let mut queue = self.pending.lock().unwrap_or_else(|err| err.into_inner());
                loop {
                    if let Some(request) = queue.pop_front() {
                        break request;
                    }
                    queue = self.wake.wait(queue).unwrap_or_else(|err| err.into_inner());
                }
            };
            if request.cancel.is_cancelled() || request.result.is_closed() {
                continue;
            }
            let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                inspect_request(&request)
            }))
            .unwrap_or_else(|_| {
                Err("圧縮ファイルの読み込み中に予期しないエラーが発生しました".into())
            });
            if !request.cancel.is_cancelled() {
                let _ = request.result.try_send(output);
            }
        }
    }
}

fn inspect_request(request: &InspectRequest) -> InspectionResult {
    if request.cancel.is_cancelled() {
        return Err("圧縮ファイルの読み込みをキャンセルしました".into());
    }
    if let Some(cached) = &request.cached
        && cached.source == request.path
        && cache_is_current(cached)
    {
        return Ok(cached.clone());
    }
    archive::inspect(&request.path, &request.cancel).map(Arc::new)
}

fn cache_is_current(index: &ArchiveIndex) -> bool {
    archive::source_is_current(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use explorer_core::archive::ArchiveFormat;
    #[test]
    fn inspection_reuses_current_cache_but_rejects_stale_or_cancelled_requests() {
        let root = std::env::temp_dir().join(format!(
            "skylis-archive-inspection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        // macOS temp_dir may begin with the system /var -> /private/var symlink.
        let root = root.canonicalize().unwrap();
        let source = root.join("source.txt");
        let output = root.join("source.zip");
        std::fs::write(&source, b"source").unwrap();
        let cancel = CancelToken::new();
        archive::create(
            &[source.clone()],
            &output,
            ArchiveFormat::Zip,
            &cancel,
            |_| {},
        )
        .unwrap();
        let cached = Arc::new(archive::inspect(&output, &cancel).unwrap());
        let (sender, _receiver) = async_channel::bounded(1);
        let request = InspectRequest {
            path: output.clone(),
            cancel: cancel.clone(),
            cached: Some(cached.clone()),
            result: sender,
        };
        assert!(Arc::ptr_eq(&cached, &inspect_request(&request).unwrap()));
        std::fs::write(&output, b"corrupt archive with a different length").unwrap();
        assert!(inspect_request(&request).is_err());
        cancel.cancel();
        assert!(inspect_request(&request).is_err());
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
