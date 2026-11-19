//! App-owned job ledger. Blocking I/O and prompt waits run on workers.
mod leases;
mod model;
mod transfer;

use crate::file_operations::{self, DeleteKind, TransferKind, TransferOutcome, TransferResult};
use async_channel::{Receiver, Sender, bounded};
use explorer_core::archive::{self, ArchiveFormat, ArchiveIndex};
use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, EventEmitter, Global, Pixels,
    Window, WindowId,
};
use leases::PathLease;
pub(crate) use model::*;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use transfer::TransferCommand;

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
        let _ = weak.update(cx, |center, cx| center.unregister_window(id, cx));
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
    transfer_pending: VecDeque<TransferCommand>,
    transfer_running: Option<JobId>,
    #[cfg(windows)]
    native_owner: Option<(WindowId, std::sync::Weak<()>)>,
    transfer_completions: usize,
    completions: VecDeque<ArchiveCompletion>,
    windows: Vec<AnyWindowHandle>,
    trigger_bounds: RefCell<HashMap<WindowId, Bounds<Pixels>>>,
}

struct Record {
    job: JobSnapshot,
    cancel: Option<CancelToken>,
    prompt: Option<(PromptRequest, Sender<PromptResponse>)>,
    lease: PathLease,
}

impl EventEmitter<ArchiveCompletion> for OperationCenter {}

impl OperationCenter {
    fn new(cx: &mut Context<Self>) -> Self {
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
            transfer_pending: VecDeque::new(),
            transfer_running: None,
            #[cfg(windows)]
            native_owner: None,
            transfer_completions: 0,
            completions: VecDeque::new(),
            windows: Vec::new(),
            trigger_bounds: RefCell::new(HashMap::new()),
        }
    }

    pub fn snapshots(&self) -> impl DoubleEndedIterator<Item = &JobSnapshot> {
        self.jobs.iter().rev().map(|record| &record.job)
    }

    pub fn has_active(&self) -> bool {
        self.snapshots().any(|job| job.status.is_active())
    }

    pub fn native_owned_by(&self, window: WindowId) -> bool {
        #[cfg(windows)]
        {
            self.native_owner
                .as_ref()
                .is_some_and(|(id, owner)| *id == window && owner.upgrade().is_some())
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            false
        }
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
        self.trigger_bounds.borrow_mut().remove(&id);
        if self.windows.is_empty() {
            let prompted: Vec<JobId> = self
                .jobs
                .iter()
                .filter(|record| record.prompt.is_some())
                .map(|record| record.job.id)
                .collect();
            for job in prompted {
                self.cancel(job, cx);
            }
        }
        cx.notify();
    }

    pub fn record_trigger_bounds(&self, window: WindowId, bounds: Bounds<Pixels>) -> bool {
        self.trigger_bounds.borrow_mut().insert(window, bounds) != Some(bounds)
    }

    pub fn trigger_bounds(&self, window: WindowId) -> Option<Bounds<Pixels>> {
        self.trigger_bounds.borrow().get(&window).copied()
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
        if self.completions.len() + self.transfer_completions + active >= RETAINED_JOBS {
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
        self.jobs.push_back(Record {
            job: JobSnapshot::new(id, kind, backend, destination, origin, cancel.is_some()),
            cancel,
            prompt: None,
            lease: PathLease::default(),
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
        let lease = PathLease::archive(&paths, Some(&destination));
        self.submit_archive_with_lease(
            JobKind::ArchiveCreate(format),
            lease,
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
        let lease = PathLease::archive(std::slice::from_ref(&index.source), Some(&parent));
        self.submit_archive_with_lease(
            JobKind::ArchiveExtract,
            lease,
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

    #[cfg(test)]
    pub(crate) fn submit_archive(
        &mut self,
        kind: JobKind,
        destination: Option<PathBuf>,
        origin: AnyWindowHandle,
        execute: impl FnOnce(&mut JobWorker) -> Result<PathBuf, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<JobId, String> {
        self.submit_archive_with_lease(kind, PathLease::default(), destination, origin, execute, cx)
    }

    fn submit_archive_with_lease(
        &mut self,
        kind: JobKind,
        lease: PathLease,
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
        self.record(id).expect("new archive job").lease = lease;
        let (finished, terminal) = bounded(1);
        let worker = self.job_worker(id, cancel, cx);
        self.pending.push_back(ArchiveCommand {
            id,
            finished,
            execute: Box::new(execute),
            affected_parents: destination.into_iter().collect(),
            worker,
        });
        self.start_ready(cx);
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

    fn start_ready(&mut self, cx: &mut Context<Self>) {
        self.start_archive(cx);
        self.start_transfer(cx);
    }

    fn waits_for_other_lane(
        &self,
        id: JobId,
        other_running: Option<JobId>,
        waiting: impl Iterator<Item = JobId>,
    ) -> bool {
        let lease = &self
            .jobs
            .iter()
            .find(|record| record.job.id == id)
            .expect("queued job retained")
            .lease;
        other_running
            .into_iter()
            .chain(waiting.filter(|earlier| earlier.0 < id.0))
            .any(|other| {
                self.jobs
                    .iter()
                    .any(|record| record.job.id == other && lease.conflicts(&record.lease))
            })
    }

    fn start_archive(&mut self, cx: &mut Context<Self>) {
        if self.archive_running.is_some() {
            return;
        }
        let mut failed = false;
        while let Some(id) = self.pending.front().map(|command| command.id) {
            if self.waits_for_other_lane(
                id,
                self.transfer_running,
                self.transfer_pending.iter().map(|command| command.id),
            ) {
                break;
            }
            let command = self.pending.pop_front().expect("ready archive job");
            if self.archive_worker.is_none() {
                match start_archive_worker() {
                    Ok(worker) => self.archive_worker = Some(worker),
                    Err(error) => {
                        failed = true;
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
                    failed = true;
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
        if failed {
            self.start_transfer(cx);
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
        self.start_ready(cx);
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
            record.lease = PathLease::default();
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
        } else if let Some(index) = self
            .transfer_pending
            .iter()
            .position(|command| command.id == id)
        {
            let command = self
                .transfer_pending
                .remove(index)
                .expect("queued transfer");
            self.finish_transfer(
                id,
                command.result,
                Ok(TransferOutcome::Aborted),
                None,
                Default::default(),
                cx,
            );
        }
        self.start_ready(cx);
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

    fn job_worker(&self, id: JobId, cancel: CancelToken, cx: &mut Context<Self>) -> JobWorker {
        let (events, receiver) = bounded(1);
        let worker = JobWorker::new(cancel, events);
        let progress = worker.progress.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |center, cx| match event {
                        WorkerEvent::Progress => {
                            let sample = progress
                                .try_lock()
                                .ok()
                                .and_then(|mut state| state.mailbox.take());
                            if let Some(sample) = sample {
                                center.update_progress(id, sample, cx);
                            }
                        }
                        WorkerEvent::Prompt(kind, response) => {
                            if !center.windows.is_empty() {
                                center.install_prompt(id, kind, response, cx);
                            }
                        }
                        #[cfg(windows)]
                        WorkerEvent::Owner(excluded, response) => {
                            center.resolve_owner(id, &excluded, response, cx)
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        worker
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
    #[cfg(windows)]
    Owner(Vec<WindowId>, Sender<Option<transfer::NativeOwner>>),
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
                    worker.latest_progress(),
                ));
            }
        })
        .map_err(|err| format!("圧縮ワーカーを開始できません: {err}"))?;
    Ok(commands)
}

#[derive(Clone)]
pub(crate) struct JobWorker {
    cancel: CancelToken,
    progress: Arc<Mutex<WorkerProgress>>,
    issues: Arc<Mutex<JobIssues>>,
    events: Sender<WorkerEvent>,
}

#[derive(Default)]
struct WorkerProgress {
    mailbox: Option<JobProgress>,
    last_publish: Option<Instant>,
    newest: Option<JobProgress>,
}

impl JobWorker {
    fn new(cancel: CancelToken, events: Sender<WorkerEvent>) -> Self {
        Self {
            cancel,
            progress: Default::default(),
            issues: Default::default(),
            events,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.core.is_cancelled()
    }

    pub fn report(&self, progress: JobProgress) {
        self.report_at(progress, Instant::now());
    }

    fn latest_progress(&self) -> Option<JobProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .newest
            .clone()
    }

    fn report_at(&self, progress: JobProgress, now: Instant) {
        let mut state = self.progress.lock().unwrap_or_else(|err| err.into_inner());
        state.newest =
            Some(progress.normalized(state.newest.as_ref().unwrap_or(&JobProgress::default())));
        if state
            .last_publish
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL)
        {
            state.mailbox = state.newest.clone();
            state.last_publish = Some(now);
            drop(state);
            let _ = self.events.try_send(WorkerEvent::Progress);
        }
    }

    pub fn issue(&self, issue: JobIssue) {
        self.issues
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .add(issue);
    }

    pub fn prompt(&mut self, kind: PromptKind) -> Option<PromptResponse> {
        if let Some(newest) = self.latest_progress() {
            let mut state = self.progress.lock().unwrap_or_else(|err| err.into_inner());
            state.mailbox = Some(newest);
            state.last_publish = Some(Instant::now());
            drop(state);
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
