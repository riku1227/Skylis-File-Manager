//! App-owned job ledger. Blocking I/O and prompt waits run on workers.
mod model;

use crate::{
    file_operation_progress,
    file_operations::{self, DeleteKind, TransferKind, TransferOutcome, TransferResult},
};
use async_channel::{Receiver, Sender, bounded};
use explorer_core::archive::{self, ArchiveFormat, ArchiveIndex};
use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, EventEmitter, Global, Pixels, Point,
    Window, WindowId,
};
pub(crate) use model::*;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PENDING_ARCHIVES: usize = 4;
const RETAINED_JOBS: usize = 32;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone)]
struct SharedOperationCenter(Entity<OperationCenter>);
impl Global for SharedOperationCenter {}

pub(crate) fn shared(cx: &App) -> Option<Entity<OperationCenter>> {
    cx.try_global::<SharedOperationCenter>()
        .map(|shared| shared.0.clone())
}

pub(crate) fn init(cx: &mut App) -> Entity<OperationCenter> {
    if let Some(center) = shared(cx) {
        return center;
    }
    let center = cx.new(OperationCenter::new);
    cx.set_global(SharedOperationCenter(center.clone()));
    let weak = center.downgrade();
    cx.on_window_closed(move |cx, id| {
        let child = weak
            .update(cx, |center, cx| {
                center.unregister_window(id, cx);
                center
                    .windows
                    .is_empty()
                    .then_some(center.activity_window)
                    .flatten()
            })
            .ok()
            .flatten();
        if let Some(child) = child {
            cx.defer(move |cx| {
                let _ = child.update(cx, |_, window, _| window.remove_window());
            });
        }
    })
    .detach();
    center
}

#[derive(Clone, Debug)]
pub(crate) struct ArchiveCompletion {
    pub output: Result<PathBuf, String>,
    pub affected_parents: Vec<PathBuf>,
    pub cancelled: bool,
}

pub(crate) struct OperationCenter {
    jobs: VecDeque<Record>,
    next_id: u64,
    next_prompt: u64,
    pending: VecDeque<ArchiveCommand>,
    archive_running: Option<JobId>,
    archive_worker: Option<Sender<ArchiveCommand>>,
    native: Option<JobId>,
    completions: VecDeque<ArchiveCompletion>,
    windows: Vec<AnyWindowHandle>,
    pill_bounds: RefCell<HashMap<WindowId, Bounds<Pixels>>>,
    pub activity_window: Option<AnyWindowHandle>,
    pub auto_presented: bool,
}

struct Record {
    job: JobSnapshot,
    cancel: Option<CancelToken>,
    prompt: Option<(PromptRequest, Sender<PromptResponse>)>,
}

impl EventEmitter<ArchiveCompletion> for OperationCenter {}

impl OperationCenter {
    fn new(cx: &mut Context<Self>) -> Self {
        let receiver = file_operation_progress::subscribe_progress();
        cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                let progress = file_operation_progress::progress_snapshot();
                if this
                    .update(cx, |center, cx| {
                        if let Some(id) = center.native
                            && let Some(progress) = progress
                        {
                            center.update_progress(
                                id,
                                JobProgress {
                                    measure: ProgressMeasure::Relative {
                                        work_done: progress.work_done,
                                        work_total: progress.work_total,
                                    },
                                    current_item: progress.current_item,
                                    hydrating_cloud_likely: false,
                                },
                                cx,
                            );
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.on_app_quit(|this, _| {
            this.cancel_workers();
            async {}
        })
        .detach();
        Self {
            jobs: VecDeque::new(),
            next_id: 1,
            next_prompt: 1,
            pending: VecDeque::new(),
            archive_running: None,
            archive_worker: None,
            native: None,
            completions: VecDeque::new(),
            windows: Vec::new(),
            pill_bounds: RefCell::new(HashMap::new()),
            activity_window: None,
            auto_presented: false,
        }
    }

    pub fn snapshots(&self) -> impl DoubleEndedIterator<Item = &JobSnapshot> {
        self.jobs.iter().rev().map(|record| &record.job)
    }

    pub fn has_active(&self) -> bool {
        self.snapshots().any(|job| job.status.is_active())
    }

    pub fn native_owned_by(&self, window: WindowId) -> bool {
        cfg!(windows)
            && self.native.is_some_and(|id| {
                self.snapshots().any(|job| {
                    job.id == id
                        && job
                            .origin
                            .is_some_and(|origin| origin.window_id() == window)
                })
            })
    }

    pub fn register_window(&mut self, window: AnyWindowHandle) {
        if !self.windows.contains(&window) {
            self.windows.insert(0, window);
        }
    }

    pub fn activate_window(&mut self, window: AnyWindowHandle, cx: &mut Context<Self>) {
        if self.windows.last() != Some(&window) {
            self.windows.retain(|&candidate| candidate != window);
            self.windows.push(window);
            cx.notify();
        }
    }

    pub fn unregister_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.windows.retain(|window| window.window_id() != id);
        self.pill_bounds.borrow_mut().remove(&id);
        if self
            .activity_window
            .is_some_and(|window| window.window_id() == id)
        {
            self.activity_window = None;
        }
        cx.notify();
    }

    pub fn record_pill_bounds(&self, window: WindowId, bounds: Bounds<Pixels>) {
        self.pill_bounds.borrow_mut().insert(window, bounds);
    }

    pub fn clear_pill_bounds(&self, window: WindowId) {
        self.pill_bounds.borrow_mut().remove(&window);
    }

    pub fn pill_contains(&self, window: WindowId, point: Point<Pixels>) -> bool {
        self.pill_bounds
            .borrow()
            .get(&window)
            .is_some_and(|bounds| bounds.contains(&point))
    }

    pub fn target_window(&self, origin: Option<AnyWindowHandle>) -> Option<AnyWindowHandle> {
        origin
            .filter(|window| self.windows.contains(window))
            .or_else(|| self.windows.last().copied())
    }

    pub fn dismiss_finished(&mut self, cx: &mut Context<Self>) {
        self.jobs.retain(|record| record.job.status.is_active());
        cx.notify();
    }

    pub fn acknowledge_errors(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for record in &mut self.jobs {
            changed |= std::mem::take(&mut record.job.unread_error);
        }
        if changed {
            cx.notify();
        }
    }

    fn insert(
        &mut self,
        kind: JobKind,
        backend: JobBackend,
        destination: Option<PathBuf>,
        origin: AnyWindowHandle,
        cancel: Option<CancelToken>,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        let active = self
            .snapshots()
            .filter(|job| job.status.is_active())
            .count();
        if self.completions.len() + active >= RETAINED_JOBS {
            return Err("完了した操作を更新中です。少し待ってからお試しください".into());
        }
        let id = JobId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("操作IDの上限に達しました")?;
        if self.jobs.len() >= RETAINED_JOBS {
            let oldest = self
                .jobs
                .iter()
                .position(|record| !record.job.status.is_active())
                .expect("admission reserves one history slot");
            self.jobs.remove(oldest);
        }
        if active == 0 {
            self.auto_presented = false;
        }
        self.jobs.push_back(Record {
            job: JobSnapshot::new(
                id,
                kind,
                backend,
                destination,
                origin,
                cancel.is_some(),
                cx.background_executor().now(),
            ),
            cancel,
            prompt: None,
        });
        cx.notify();
        Ok(id)
    }

    fn record(&mut self, id: JobId) -> Option<&mut Record> {
        self.jobs.iter_mut().find(|record| record.job.id == id)
    }

    fn update_progress(&mut self, id: JobId, progress: JobProgress, cx: &mut Context<Self>) {
        if let Some(record) = self.record(id)
            && record.job.status.is_active()
        {
            record.job.update(progress);
            cx.notify();
        }
    }

    pub fn submit_create(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        format: ArchiveFormat,
        origin: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        let parent = destination.parent().map(PathBuf::from);
        self.submit_archive(
            JobKind::ArchiveCreate(format),
            parent,
            origin,
            move |worker| {
                let cancel = worker.cancel.core.clone();
                archive::create(&paths, &destination, format, &cancel, |progress| {
                    worker.report(progress.into());
                })
            },
            cx,
        )
    }

    pub fn submit_extract(
        &mut self,
        index: Arc<ArchiveIndex>,
        selected: Vec<String>,
        parent: PathBuf,
        origin: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        self.submit_archive(
            JobKind::ArchiveExtract,
            Some(parent.clone()),
            origin,
            move |worker| {
                let cancel = worker.cancel.core.clone();
                archive::extract(&index, &selected, &parent, &cancel, |progress| {
                    worker.report(progress.into());
                })
            },
            cx,
        )
    }

    pub(crate) fn submit_archive(
        &mut self,
        kind: JobKind,
        destination: Option<PathBuf>,
        origin: AnyWindowHandle,
        execute: impl FnOnce(&mut JobWorker) -> Result<PathBuf, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        if self.pending.len() >= PENDING_ARCHIVES {
            return Err("圧縮・展開の待機数が上限（4件）に達しています".into());
        }
        if self.archive_worker.is_none() {
            self.archive_worker = Some(start_archive_worker()?);
        }
        let cancel = CancelToken::new();
        let id = self.insert(
            kind,
            JobBackend::Builtin,
            destination.clone(),
            origin,
            Some(cancel.clone()),
            cx,
        )?;
        let (events, receiver) = bounded(1);
        let (finished, terminal) = bounded(1);
        let mailbox = Arc::new(Mutex::new(None));
        self.pending.push_back(ArchiveCommand {
            id,
            finished,
            execute: Box::new(execute),
            affected_parents: destination.into_iter().collect(),
            worker: JobWorker {
                cancel,
                mailbox: mailbox.clone(),
                events,
                last_publish: None,
                newest: None,
            },
        });
        self.start_next(cx);

        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |center, cx| match event {
                        WorkerEvent::Progress => {
                            let progress =
                                mailbox.lock().unwrap_or_else(|err| err.into_inner()).take();
                            if let Some(progress) = progress {
                                center.update_progress(id, progress, cx);
                            }
                        }
                        WorkerEvent::Prompt(kind, response) => {
                            center.install_prompt(id, kind, response, cx)
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = terminal.recv().await;
            let _ = this.update(cx, |center, cx| match result {
                Ok((completion, progress)) => {
                    center.finish_archive(id, completion, progress, cx);
                }
                Err(_) if center.archive_running == Some(id) => {
                    center.archive_worker = None;
                    center.finish_archive(
                        id,
                        ArchiveCompletion {
                            output: Err("圧縮ワーカーが予期せず停止しました".into()),
                            affected_parents: Vec::new(),
                            cancelled: false,
                        },
                        None,
                        cx,
                    );
                }
                Err(_) => {}
            });
        })
        .detach();
        Ok(id)
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        if self.archive_running.is_some() {
            return;
        }
        while let Some(command) = self.pending.pop_front() {
            let id = command.id;
            if self.archive_worker.is_none() {
                match start_archive_worker() {
                    Ok(worker) => self.archive_worker = Some(worker),
                    Err(error) => {
                        self.record_archive_completion(
                            id,
                            ArchiveCompletion {
                                output: Err(error),
                                affected_parents: command.affected_parents,
                                cancelled: false,
                            },
                            cx,
                        );
                        continue;
                    }
                }
            }
            let record = self.record(id).expect("queued job is retained");
            record.job.status = JobStatus::Running;
            match self
                .archive_worker
                .as_ref()
                .expect("started worker")
                .try_send(command)
            {
                Ok(()) => {
                    self.archive_running = Some(id);
                    cx.notify();
                    break;
                }
                Err(err) => {
                    if err.is_closed() {
                        self.archive_worker = None;
                    }
                    let command = err.into_inner();
                    self.record_archive_completion(
                        id,
                        ArchiveCompletion {
                            output: Err("圧縮ワーカーと通信できません".into()),
                            affected_parents: command.affected_parents,
                            cancelled: false,
                        },
                        cx,
                    );
                }
            }
        }
    }

    fn finish_archive(
        &mut self,
        id: JobId,
        completion: ArchiveCompletion,
        progress: Option<JobProgress>,
        cx: &mut Context<Self>,
    ) {
        if let Some(progress) = progress {
            self.update_progress(id, progress, cx);
        }
        self.archive_running = None;
        self.record_archive_completion(id, completion, cx);
        self.start_next(cx);
    }

    fn record_archive_completion(
        &mut self,
        id: JobId,
        completion: ArchiveCompletion,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut record) = self.take_record(id) {
            record.prompt.take();
            record.cancel.take();
            record.job.output = completion.output.as_ref().ok().cloned();
            record.job.finish(
                match &completion.output {
                    Ok(_) => JobStatus::Completed,
                    Err(_) if completion.cancelled => JobStatus::Cancelled,
                    Err(_) => JobStatus::Failed,
                },
                completion
                    .output
                    .as_ref()
                    .err()
                    .filter(|_| !completion.cancelled)
                    .cloned(),
            );
            self.jobs.push_back(record);
        }
        if self.completions.is_empty() {
            let weak = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |center, cx| {
                    for completion in center.completions.drain(..) {
                        cx.emit(completion);
                    }
                });
            });
        }
        self.completions.push_back(completion);
        cx.notify();
    }

    fn take_record(&mut self, id: JobId) -> Option<Record> {
        self.jobs
            .iter()
            .position(|record| record.job.id == id)
            .and_then(|index| self.jobs.remove(index))
    }

    pub fn cancel(&mut self, id: JobId, cx: &mut Context<Self>) -> bool {
        let Some(record) = self.record(id) else {
            return false;
        };
        let Some(cancel) = &record.cancel else {
            return false;
        };
        cancel.cancel();
        record.prompt.take();
        record.job.status = JobStatus::Cancelling;
        if let Some(index) = self.pending.iter().position(|command| command.id == id) {
            let command = self.pending.remove(index).expect("existing command");
            self.record_archive_completion(
                id,
                ArchiveCompletion {
                    output: Err("圧縮ファイルの操作をキャンセルしました".into()),
                    affected_parents: command.affected_parents,
                    cancelled: true,
                },
                cx,
            );
        }
        cx.notify();
        true
    }

    fn cancel_workers(&mut self) {
        for record in &mut self.jobs {
            if let Some(cancel) = &record.cancel {
                cancel.cancel();
            }
        }
    }

    fn install_prompt(
        &mut self,
        id: JobId,
        kind: PromptKind,
        response: Sender<PromptResponse>,
        cx: &mut Context<Self>,
    ) {
        let prompt = PromptId(self.next_prompt);
        let Some(next) = self.next_prompt.checked_add(1) else {
            return;
        };
        let Some(record) = self.record(id) else {
            return;
        };
        if record.job.status != JobStatus::Running {
            return;
        }
        record.prompt = Some((
            PromptRequest {
                job: id,
                id: prompt,
                kind,
            },
            response,
        ));
        record.job.status = JobStatus::WaitingForUser(prompt);
        self.next_prompt = next;
        cx.notify();
    }

    pub fn prompt_for(&self, window: AnyWindowHandle) -> Option<PromptRequest> {
        self.jobs.iter().find_map(|record| {
            let (request, _) = record.prompt.as_ref()?;
            (self.target_window(record.job.origin) == Some(window)).then(|| request.clone())
        })
    }

    pub fn respond(&mut self, response: PromptResponse, cx: &mut Context<Self>) -> bool {
        let Some(record) = self.record(response.job) else {
            return false;
        };
        let Some((request, _)) = &record.prompt else {
            return false;
        };
        if request.id != response.prompt
            || !request.kind.accepts(response.action, response.apply_to_all)
        {
            return false;
        }
        let (_, sender) = record.prompt.take().expect("validated pending prompt");
        let sent = sender.try_send(response).is_ok();
        record.job.status = JobStatus::Running;
        cx.notify();
        sent
    }

    pub fn transfer(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        kind: TransferKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<Receiver<TransferResult>, String> {
        self.submit_native(kind.into(), Some(destination.clone()), window, cx, || {
            file_operations::transfer(paths, destination, kind, window)
        })
    }

    pub fn delete(
        &mut self,
        paths: Vec<PathBuf>,
        kind: DeleteKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<Receiver<TransferResult>, String> {
        let job = match kind {
            DeleteKind::Recycle => JobKind::Trash,
            DeleteKind::Permanent => JobKind::PermanentDelete,
        };
        self.submit_native(job, None, window, cx, || {
            file_operations::delete(paths, kind, window)
        })
    }

    fn submit_native(
        &mut self,
        kind: JobKind,
        destination: Option<PathBuf>,
        window: &Window,
        cx: &mut Context<Self>,
        launch: impl FnOnce() -> anyhow::Result<Receiver<TransferResult>>,
    ) -> Result<Receiver<TransferResult>, String> {
        if self.native.is_some() {
            return Err(
                "ファイル操作が進行中です。完了またはキャンセル後に操作してください".into(),
            );
        }
        let engine = cx
            .try_global::<crate::settings::SharedPreferences>()
            .map(|prefs| prefs.0.read(cx).value.file_transfer_engine)
            .unwrap_or_default();
        let id = self.insert(
            kind,
            JobBackend::OsDelegated { reason: engine },
            destination,
            window.window_handle(),
            None,
            cx,
        )?;
        let worker_result = match launch() {
            Ok(result) => result,
            Err(error) => {
                let error = error.to_string();
                if let Some(record) = self.record(id) {
                    record.job.finish(JobStatus::Failed, Some(error.clone()));
                }
                return Err(error);
            }
        };
        self.native = Some(id);
        self.record(id).expect("new job").job.status = JobStatus::Running;
        let (sender, result) = bounded(1);
        cx.spawn(async move |this, cx| {
            let outcome = worker_result.recv().await.unwrap_or_else(|_| {
                Err("ファイル操作の結果を取得できませんでした。元と保存先を確認してください".into())
            });
            let _ = this.update(cx, |center, cx| {
                center.native = None;
                if let Some(mut record) = center.take_record(id) {
                    let (status, issue) = match &outcome {
                        Ok(TransferOutcome::Completed) => (JobStatus::Completed, None),
                        Ok(TransferOutcome::Aborted) => (JobStatus::Cancelled, None),
                        Err(error) => (JobStatus::Failed, Some(error.clone())),
                    };
                    record.job.finish(status, issue);
                    center.jobs.push_back(record);
                }
                cx.notify();
            });
            let _ = sender.try_send(outcome);
        })
        .detach();
        Ok(result)
    }
}

impl Drop for OperationCenter {
    fn drop(&mut self) {
        self.cancel_workers();
    }
}

type ArchiveWork = Box<dyn FnOnce(&mut JobWorker) -> Result<PathBuf, String> + Send>;

struct ArchiveCommand {
    id: JobId,
    finished: Sender<(ArchiveCompletion, Option<JobProgress>)>,
    execute: ArchiveWork,
    affected_parents: Vec<PathBuf>,
    worker: JobWorker,
}

enum WorkerEvent {
    Progress,
    Prompt(PromptKind, Sender<PromptResponse>),
}

fn start_archive_worker() -> Result<Sender<ArchiveCommand>, String> {
    let (commands, receiver) = bounded::<ArchiveCommand>(1);
    std::thread::Builder::new()
        .name("archive-operations".into())
        .spawn(move || {
            while let Ok(ArchiveCommand {
                execute,
                affected_parents,
                finished,
                mut worker,
                ..
            }) = receiver.recv_blocking()
            {
                let output =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(&mut worker)))
                        .unwrap_or_else(|_| {
                            Err("圧縮データの処理中に予期しないエラーが発生しました".into())
                        });
                let cancelled = output.is_err() && worker.cancel.core.is_cancelled();
                let _ = finished.try_send((
                    ArchiveCompletion {
                        output,
                        affected_parents,
                        cancelled,
                    },
                    worker.newest,
                ));
            }
        })
        .map_err(|err| format!("圧縮ワーカーを開始できません: {err}"))?;
    Ok(commands)
}

pub(crate) struct JobWorker {
    cancel: CancelToken,
    mailbox: Arc<Mutex<Option<JobProgress>>>,
    events: Sender<WorkerEvent>,
    last_publish: Option<Instant>,
    newest: Option<JobProgress>,
}

impl JobWorker {
    fn report(&mut self, progress: JobProgress) {
        self.report_at(progress, Instant::now());
    }

    fn report_at(&mut self, progress: JobProgress, now: Instant) {
        if let Some(previous) = &self.newest {
            self.newest = Some(progress.normalized(previous));
        } else {
            self.newest = Some(progress.normalized(&JobProgress::default()));
        }
        if self
            .last_publish
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL)
        {
            *self.mailbox.lock().unwrap_or_else(|err| err.into_inner()) = self.newest.clone();
            self.last_publish = Some(now);
            let _ = self.events.try_send(WorkerEvent::Progress);
        }
    }

    /// Future I/O backends use this on the worker, before any destructive step.
    #[allow(dead_code)]
    pub fn prompt(&mut self, kind: PromptKind) -> Option<PromptResponse> {
        if let Some(newest) = &self.newest {
            *self.mailbox.lock().unwrap_or_else(|err| err.into_inner()) = Some(newest.clone());
            self.last_publish = Some(Instant::now());
            let _ = self.events.try_send(WorkerEvent::Progress);
        }
        let (sender, receiver) = bounded(1);
        gpui::block_on(self.cancel.until_cancelled(async {
            self.events
                .send(WorkerEvent::Prompt(kind, sender))
                .await
                .ok()?;
            receiver.recv().await.ok()
        }))
        .flatten()
    }
}

#[derive(Clone)]
struct CancelToken {
    core: explorer_core::CancelToken,
    signal: Sender<()>,
    cancelled: Receiver<()>,
}

impl CancelToken {
    fn new() -> Self {
        let (signal, cancelled) = bounded(1);
        Self {
            core: explorer_core::CancelToken::new(),
            signal,
            cancelled,
        }
    }

    fn cancel(&self) {
        self.core.cancel();
        self.signal.close();
    }

    async fn until_cancelled<T>(&self, task: impl Future<Output = T>) -> Option<T> {
        use std::task::Poll;
        let mut task = std::pin::pin!(task);
        let mut signal = std::pin::pin!(self.cancelled.recv());
        std::future::poll_fn(|cx| {
            if signal.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            task.as_mut().poll(cx).map(Some)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
