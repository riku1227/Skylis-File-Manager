//! Application-wide archive workers. Navigation cancels inspection, not submitted file jobs.
//! All filesystem access stays on dedicated threads, including cache fingerprint checks.
use explorer_core::{
    CancelToken,
    archive::{self, ArchiveFormat, ArchiveIndex, ArchiveProgress},
};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

pub(crate) type JobId = u64;
const PENDING_JOBS: usize = 4;
const RETAINED_JOBS: usize = 16;
const PENDING_INSPECTIONS: usize = 8;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArchiveJobKind {
    Create(ArchiveFormat),
    Extract,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JobSnapshot {
    pub id: JobId,
    pub kind: ArchiveJobKind,
    pub label: String,
    pub destination: PathBuf,
    pub status: JobStatus,
    pub progress: Option<ArchiveProgress>,
    pub output: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ArchiveCompletion {
    pub output: Result<PathBuf, String>,
    pub affected_parents: Vec<PathBuf>,
    pub cancelled: bool,
}

#[derive(Clone)]
pub(crate) struct SharedArchiveJobs(pub Entity<ArchiveJobs>);
impl Global for SharedArchiveJobs {}

pub(crate) fn shared(cx: &App) -> Option<Entity<ArchiveJobs>> {
    cx.try_global::<SharedArchiveJobs>()
        .map(|shared| shared.0.clone())
}

pub(crate) fn init(cx: &mut App) -> Entity<ArchiveJobs> {
    if let Some(jobs) = shared(cx) {
        return jobs;
    }
    let jobs = cx.new(ArchiveJobs::new);
    cx.set_global(SharedArchiveJobs(jobs.clone()));
    jobs
}

pub(crate) struct ArchiveJobs {
    engine: Arc<Engine>,
    _updates: Task<()>,
}
impl EventEmitter<ArchiveCompletion> for ArchiveJobs {}

impl ArchiveJobs {
    fn new(cx: &mut Context<Self>) -> Self {
        let (engine, signals) = Engine::new();
        let worker = engine.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("archive-operations".into())
            .spawn(move || worker.run())
        {
            engine.state().failure = Some(format!("圧縮ワーカーを開始できません: {err}"));
        }
        let updates = cx.spawn(async move |this, cx| {
            while signals.recv().await.is_ok() {
                if this
                    .update(cx, |this, cx| this.publish_updates(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            engine,
            _updates: updates,
        }
    }

    /// Queue bookkeeping only; validation and traversal run on the worker.
    pub fn submit_create(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        format: ArchiveFormat,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        self.publish_updates(cx);
        let id = self.engine.submit(Operation::Create {
            paths,
            destination,
            format,
        })?;
        cx.notify();
        Ok(id)
    }

    pub fn submit_extract(
        &mut self,
        index: Arc<ArchiveIndex>,
        selected: Vec<String>,
        parent: PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        self.publish_updates(cx);
        let id = self.engine.submit(Operation::Extract {
            index,
            selected,
            parent,
        })?;
        cx.notify();
        Ok(id)
    }

    /// Bounded newest-first history; completed jobs retain their output/error for all windows.
    pub fn snapshot(&self) -> Vec<JobSnapshot> {
        self.engine
            .state()
            .records
            .iter()
            .rev()
            .map(|record| record.snapshot.clone())
            .collect()
    }

    pub fn cancel(&mut self, id: JobId, cx: &mut Context<Self>) -> bool {
        let changed = self.engine.cancel(id);
        if changed {
            self.publish_updates(cx);
        }
        changed
    }

    pub fn pending_count(&self) -> usize {
        self.engine.state().pending.len()
    }

    /// Clear finished notices without cancelling or hiding active operations.
    pub fn dismiss_finished(&mut self, cx: &mut Context<Self>) {
        self.publish_updates(cx);
        self.engine
            .state()
            .records
            .retain(|record| record.snapshot.status.is_active());
        cx.notify();
    }

    /// Includes queued and cancelling work; use when guarding the last application window.
    pub fn has_active(&self) -> bool {
        let state = self.engine.state();
        state.running.is_some() || !state.pending.is_empty()
    }

    fn publish_updates(&mut self, cx: &mut Context<Self>) {
        let completions: Vec<_> = self.engine.state().completions.drain(..).collect();
        for completion in completions {
            cx.emit(completion);
        }
        cx.notify();
    }
}

impl Drop for ArchiveJobs {
    fn drop(&mut self) {
        let mut state = self.engine.state();
        state.stopping = true;
        for record in &state.records {
            if record.snapshot.status.is_active() {
                record.cancel.cancel();
            }
        }
        self.engine.wake.notify_all();
    }
}

enum Operation {
    Create {
        paths: Vec<PathBuf>,
        destination: PathBuf,
        format: ArchiveFormat,
    },
    Extract {
        index: Arc<ArchiveIndex>,
        selected: Vec<String>,
        parent: PathBuf,
    },
}

impl Operation {
    fn description(&self) -> (ArchiveJobKind, String, PathBuf) {
        match self {
            Self::Create {
                destination,
                format,
                ..
            } => (
                ArchiveJobKind::Create(*format),
                destination
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                destination.clone(),
            ),
            Self::Extract { index, parent, .. } => (
                ArchiveJobKind::Extract,
                index
                    .source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                parent.clone(),
            ),
        }
    }

    fn parents(&self) -> Vec<PathBuf> {
        match self {
            Self::Create { destination, .. } => destination
                .parent()
                .map(|p| vec![p.to_path_buf()])
                .unwrap_or_default(),
            Self::Extract { parent, .. } => vec![parent.clone()],
        }
    }

    fn execute(
        self,
        cancel: &CancelToken,
        progress: impl FnMut(ArchiveProgress),
    ) -> Result<PathBuf, String> {
        match self {
            Self::Create {
                paths,
                destination,
                format,
            } => archive::create(&paths, &destination, format, cancel, progress),
            Self::Extract {
                index,
                selected,
                parent,
            } => archive::extract(&index, &selected, &parent, cancel, progress),
        }
    }
}

struct Pending {
    id: JobId,
    operation: Operation,
    cancel: CancelToken,
}
struct Recorded {
    snapshot: JobSnapshot,
    cancel: CancelToken,
}
struct State {
    next_id: JobId,
    pending: VecDeque<Pending>,
    running: Option<JobId>,
    records: VecDeque<Recorded>,
    completions: VecDeque<ArchiveCompletion>,
    failure: Option<String>,
    stopping: bool,
}

struct Engine {
    state: Mutex<State>,
    wake: Condvar,
    signals: async_channel::Sender<()>,
}

impl Engine {
    fn new() -> (Arc<Self>, async_channel::Receiver<()>) {
        let (signals, receiver) = async_channel::bounded(1);
        (
            Arc::new(Self {
                state: Mutex::new(State {
                    next_id: 1,
                    pending: VecDeque::new(),
                    running: None,
                    records: VecDeque::new(),
                    completions: VecDeque::new(),
                    failure: None,
                    stopping: false,
                }),
                wake: Condvar::new(),
                signals,
            }),
            receiver,
        )
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }
    fn signal(&self) {
        let _ = self.signals.try_send(());
    }

    fn submit(&self, operation: Operation) -> Result<JobId, String> {
        let mut state = self.state();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.stopping {
            return Err("圧縮ワーカーは終了しています".into());
        }
        if state.pending.len() >= PENDING_JOBS {
            return Err("圧縮・展開の待機数が上限（4件）に達しています".into());
        }
        // Completion events are never silently dropped, even if a caller submits many
        // jobs within one UI transaction and the listener has not run yet.
        if state.completions.len() + state.pending.len() + usize::from(state.running.is_some())
            >= RETAINED_JOBS
        {
            return Err("完了した圧縮操作を更新中です。少し待ってからお試しください".into());
        }
        let id = state.next_id;
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or("操作IDの上限に達しました")?;
        let (kind, label, destination) = operation.description();
        let cancel = CancelToken::new();
        while state.records.len() >= RETAINED_JOBS {
            let Some(index) = state
                .records
                .iter()
                .position(|r| !r.snapshot.status.is_active())
            else {
                return Err("操作履歴の上限に達しました".into());
            };
            state.records.remove(index);
        }
        state.records.push_back(Recorded {
            snapshot: JobSnapshot {
                id,
                kind,
                label,
                destination,
                status: JobStatus::Queued,
                progress: None,
                output: None,
                error: None,
            },
            cancel: cancel.clone(),
        });
        state.pending.push_back(Pending {
            id,
            operation,
            cancel,
        });
        self.wake.notify_one();
        Ok(id)
    }

    fn cancel(&self, id: JobId) -> bool {
        let mut state = self.state();
        let Some(record) = state
            .records
            .iter_mut()
            .find(|r| r.snapshot.id == id && r.snapshot.status.is_active())
        else {
            return false;
        };
        record.cancel.cancel();
        record.snapshot.status = JobStatus::Cancelling;
        if let Some(position) = state.pending.iter().position(|pending| pending.id == id) {
            let pending = state.pending.remove(position).unwrap();
            finish(
                &mut state,
                id,
                Err("圧縮ファイルの操作をキャンセルしました".into()),
                pending.operation.parents(),
                true,
                None,
            );
        }
        drop(state);
        self.signal();
        true
    }

    fn next(&self) -> Option<Pending> {
        let mut state = self.state();
        loop {
            if state.stopping {
                return None;
            }
            if let Some(pending) = state.pending.pop_front() {
                state.running = Some(pending.id);
                if let Some(record) = state
                    .records
                    .iter_mut()
                    .find(|r| r.snapshot.id == pending.id)
                {
                    record.snapshot.status = JobStatus::Running;
                }
                return Some(pending);
            }
            state = self.wake.wait(state).unwrap_or_else(|err| err.into_inner());
        }
    }

    fn run(self: Arc<Self>) {
        while let Some(pending) = self.next() {
            self.signal();
            let parents = pending.operation.parents();
            let mut last_sent = Instant::now();
            let mut newest_progress = None;
            // Keep the service alive if a third-party codec panics on malformed input.
            let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pending.operation.execute(&pending.cancel, |progress| {
                    newest_progress = Some(progress.clone());
                    if last_sent.elapsed() >= PROGRESS_INTERVAL {
                        if let Some(record) = self
                            .state()
                            .records
                            .iter_mut()
                            .find(|r| r.snapshot.id == pending.id)
                        {
                            record.snapshot.progress = Some(progress);
                        }
                        last_sent = Instant::now();
                        self.signal();
                    }
                })
            }))
            .unwrap_or_else(|_| Err("圧縮データの処理中に予期しないエラーが発生しました".into()));
            let cancelled = output.is_err() && pending.cancel.is_cancelled();
            let mut state = self.state();
            state.running = None;
            finish(
                &mut state,
                pending.id,
                output,
                parents,
                cancelled,
                newest_progress,
            );
            drop(state);
            self.signal();
        }
    }
}

fn finish(
    state: &mut State,
    id: JobId,
    output: Result<PathBuf, String>,
    affected_parents: Vec<PathBuf>,
    cancelled: bool,
    progress: Option<ArchiveProgress>,
) {
    if let Some(record) = state.records.iter_mut().find(|r| r.snapshot.id == id) {
        record.snapshot.status = if output.is_ok() {
            JobStatus::Completed
        } else if cancelled {
            JobStatus::Cancelled
        } else {
            JobStatus::Failed
        };
        record.snapshot.output = output.as_ref().ok().cloned();
        record.snapshot.error = output.as_ref().err().cloned();
        if progress.is_some() {
            record.snapshot.progress = progress;
        }
    }
    state.completions.push_back(ArchiveCompletion {
        output,
        affected_parents,
        cancelled,
    });
}

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

    fn operation(number: usize) -> Operation {
        Operation::Create {
            paths: vec![],
            destination: PathBuf::from(format!("test{number}.zip")),
            format: ArchiveFormat::Zip,
        }
    }

    #[test]
    fn operation_queue_is_bounded_cancellable_and_keeps_completion_records() {
        let (engine, _signals) = Engine::new();
        let running = engine.submit(operation(0)).unwrap();
        let pending = engine.next().unwrap();
        assert_eq!(pending.id, running);
        let queued: Vec<_> = (1..=4)
            .map(|n| engine.submit(operation(n)).unwrap())
            .collect();
        assert!(engine.submit(operation(5)).is_err());
        assert!(engine.cancel(queued[1]));
        assert_eq!(engine.state().pending.len(), 3);
        assert_eq!(engine.state().completions.len(), 1);
        assert!(engine.submit(operation(6)).is_ok());
        assert!(engine.cancel(running));
        assert!(pending.cancel.is_cancelled());
        assert_eq!(
            engine.state().records.front().unwrap().snapshot.status,
            JobStatus::Cancelling
        );
        let event = engine.state().completions.pop_front().unwrap();
        assert!(event.cancelled && event.output.is_err());
    }

    #[test]
    fn history_and_coalesced_notifications_are_bounded() {
        let (engine, signals) = Engine::new();
        for i in 0..40 {
            let id = engine.submit(operation(i)).unwrap();
            let request = engine.next().unwrap();
            assert_eq!(id, request.id);
            let mut state = engine.state();
            state.running = None;
            finish(
                &mut state,
                id,
                Ok(PathBuf::from("finished.zip")),
                vec![],
                false,
                None,
            );
            state.completions.clear();
            drop(state);
            engine.signal();
        }
        assert_eq!(engine.state().records.len(), RETAINED_JOBS);
        assert_eq!(signals.len(), 1);
        assert_eq!(
            engine.state().records.back().unwrap().snapshot.status,
            JobStatus::Completed
        );
        assert!(!engine.cancel(40));
    }

    #[test]
    fn success_after_cancel_request_is_still_reported_as_published() {
        let (engine, _signals) = Engine::new();
        let id = engine.submit(operation(0)).unwrap();
        let _ = engine.next().unwrap();
        engine.cancel(id);
        let mut state = engine.state();
        state.running = None;
        finish(
            &mut state,
            id,
            Ok(PathBuf::from("finished.zip")),
            vec![],
            false,
            None,
        );
        assert_eq!(state.records[0].snapshot.status, JobStatus::Completed);
        assert!(!state.completions[0].cancelled);
    }

    #[test]
    fn worker_completes_queued_jobs_after_failure_and_stops_when_woken() {
        let (engine, signals) = Engine::new();
        let first = engine.submit(operation(1)).unwrap();
        let second = engine.submit(operation(2)).unwrap();
        let worker = engine.clone();
        let thread = std::thread::spawn(move || worker.run());
        let deadline = Instant::now() + Duration::from_secs(5);
        while engine.state().completions.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Both deliberately invalid jobs fail independently; the worker remains usable.
        let complete = engine.state().completions.len();
        {
            let mut state = engine.state();
            state.stopping = true;
            engine.wake.notify_one();
        }
        thread.join().unwrap();
        assert_eq!(complete, 2);
        assert_eq!(
            engine
                .state()
                .records
                .iter()
                .map(|record| record.snapshot.id)
                .collect::<Vec<_>>(),
            [first, second]
        );
        assert!(
            engine
                .state()
                .records
                .iter()
                .all(|record| record.snapshot.status == JobStatus::Failed)
        );
        assert_eq!(signals.len(), 1);
    }

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
