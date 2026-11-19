use super::*;

const PENDING_TRANSFERS: usize = 12;

impl JobWorker {
    pub fn report_transfer_kind(&self, kinds: impl IntoIterator<Item = TransferKind>) {
        let mut kinds = kinds.into_iter();
        let Some(first) = kinds.next() else {
            return;
        };
        let kind = if kinds.any(|kind| kind != first) {
            JobKind::CopyAndMove
        } else {
            first.into()
        };
        self.report(JobProgress {
            resolved_kind: Some(kind),
            ..JobProgress::default()
        });
    }
}

type TransferWork = Box<dyn FnOnce(&mut JobWorker) -> TransferResult + Send>;

pub(super) struct TransferCommand {
    pub(super) id: JobId,
    pub(super) result: Sender<TransferResult>,
    execute: TransferWork,
}

impl OperationCenter {
    pub fn transfer(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        kind: TransferKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<Receiver<TransferResult>, String> {
        file_operations::validate_arguments(&paths, &destination).map_err(|err| err.to_string())?;
        let lease = PathLease::transfer(kind.into(), &paths, Some(&destination));
        self.submit_transfer(
            kind.into(),
            lease,
            Some(destination.clone()),
            window.window_handle(),
            move |worker| file_operations::transfer(&paths, &destination, kind, worker),
            cx,
        )
        .map(|(_, result)| result)
    }

    pub fn delete(
        &mut self,
        paths: Vec<PathBuf>,
        kind: DeleteKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<Receiver<TransferResult>, String> {
        file_operations::validate_delete_arguments(&paths).map_err(|err| err.to_string())?;
        if !file_operations::supports_delete(kind) {
            return Err("このOSでは指定した削除方法に対応していません".into());
        }
        let job = match kind {
            DeleteKind::Recycle => JobKind::Trash,
            DeleteKind::Permanent => JobKind::PermanentDelete,
        };
        let lease = PathLease::transfer(job, &paths, None);
        self.submit_transfer(
            job,
            lease,
            None,
            window.window_handle(),
            move |worker| file_operations::delete(&paths, kind, worker),
            cx,
        )
        .map(|(_, result)| result)
    }

    pub(super) fn submit_transfer(
        &mut self,
        kind: JobKind,
        lease: PathLease,
        destination: Option<PathBuf>,
        origin: AnyWindowHandle,
        execute: impl FnOnce(&mut JobWorker) -> TransferResult + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<(JobId, Receiver<TransferResult>), String> {
        if self.transfer_pending.len() >= PENDING_TRANSFERS {
            return Err("ファイル転送・削除の待機数が上限（12件）に達しています".into());
        }
        let id = self.insert(
            kind,
            JobBackend::OsDelegated,
            destination,
            origin,
            Some(CancelToken::new()),
            cx,
        )?;
        self.record(id).expect("new transfer job").lease = lease;
        let (result, receiver) = bounded(1);
        self.transfer_pending.push_back(TransferCommand {
            id,
            result,
            execute: Box::new(execute),
        });
        self.start_ready(cx);
        Ok((id, receiver))
    }

    pub(super) fn start_transfer(&mut self, cx: &mut Context<Self>) {
        if self.transfer_running.is_some() {
            return;
        }
        while let Some(id) = self.transfer_pending.front().map(|command| command.id) {
            if self.waits_for_other_lane(
                id,
                self.archive_running,
                self.pending.iter().map(|command| command.id),
            ) {
                return;
            }
            let TransferCommand {
                result, execute, ..
            } = self
                .transfer_pending
                .pop_front()
                .expect("ready transfer job");
            let record = self.record(id).expect("queued transfer");
            let cancellable =
                cfg!(windows) || (cfg!(target_os = "macos") && record.job.kind == JobKind::Trash);
            let cancel = record
                .cancel
                .as_ref()
                .expect("queued transfers are cancellable")
                .clone();
            if !cancellable {
                record.cancel = None;
            }
            record.job.cancellable = cancellable;
            record.job.status = JobStatus::Running;

            let (finished, terminal) = bounded(1);
            let mut worker = self.job_worker(id, cancel, cx);
            match std::thread::Builder::new()
                .name("file-operations".into())
                .spawn(move || {
                    let outcome = if worker.is_cancelled() {
                        Ok(TransferOutcome::Aborted)
                    } else {
                        execute(&mut worker)
                    };
                    let progress = worker.latest_progress();
                    let issues = std::mem::take(
                        &mut *worker.issues.lock().unwrap_or_else(|err| err.into_inner()),
                    );
                    let _ = finished.try_send((outcome, progress, issues));
                }) {
                Ok(_) => {
                    self.transfer_running = Some(id);
                    cx.spawn(async move |this, cx| {
                        let (outcome, progress, issues) = terminal.recv().await.unwrap_or_else(|_| (
                            Err("ファイル操作のワーカーが予期せず停止しました。元と保存先を確認してください".into()),
                            None, JobIssues::default(),
                        ));
                        let _ = this.update(cx, |center, cx| {
                            center.transfer_running = None;
                            center.finish_transfer(id, result, outcome, progress, issues, cx);
                            center.start_ready(cx);
                        });
                    }).detach();
                    cx.notify();
                    return;
                }
                Err(error) => {
                    self.finish_transfer(
                        id,
                        result,
                        Err(format!("ファイル操作を開始できません: {error}")),
                        None,
                        JobIssues::default(),
                        cx,
                    );
                    // An earlier transfer could have been holding the archive head.
                    self.start_archive(cx);
                }
            }
        }
    }

    pub(super) fn finish_transfer(
        &mut self,
        id: JobId,
        result: Sender<TransferResult>,
        mut outcome: TransferResult,
        progress: Option<JobProgress>,
        issues: JobIssues,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut record) = self.take_record(id) {
            record.cancel.take();
            record.prompt.take();
            record.lease = PathLease::default();
            if let Some(progress) = progress {
                record.job.update(progress);
            }
            record.job.issue_count = issues.count;
            record.job.issues = issues;
            if record.job.issue_count != 0 && outcome == Ok(TransferOutcome::Completed) {
                outcome = Ok(TransferOutcome::CompletedWithIssues);
            }
            let (status, issue) = match &outcome {
                Ok(TransferOutcome::Completed) => (JobStatus::Completed, None),
                Ok(TransferOutcome::CompletedWithIssues) => (JobStatus::CompletedWithIssues, None),
                Ok(TransferOutcome::Aborted) => (JobStatus::Cancelled, None),
                Err(error) => (JobStatus::Failed, Some(error.clone())),
            };
            record.job.finish(status, issue);
            self.jobs.push_back(record);
        }
        // Keep the completion reserved in the same admission budget as archive events.
        self.transfer_completions += 1;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |center, _| center.transfer_completions -= 1);
            let _ = result.try_send(outcome);
        });
        cx.notify();
    }

    #[cfg(windows)]
    pub(super) fn resolve_owner(
        &mut self,
        id: JobId,
        excluded: &[WindowId],
        response: Sender<Option<NativeOwner>>,
        cx: &mut Context<Self>,
    ) {
        if self.transfer_running != Some(id) || response.is_closed() {
            return;
        }
        let origin = self.record(id).and_then(|record| record.job.origin);
        let owner = self.find_window_owner(origin, excluded, |handle| {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            handle
                .update(cx, |_, window, _| {
                    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
                        RawWindowHandle::Win32(raw) => Some(raw.hwnd.get()),
                        _ => None,
                    }
                })
                .ok()
                .flatten()
        });
        let _ = response.try_send(owner);
    }

    #[cfg(windows)]
    pub(super) fn find_window_owner(
        &mut self,
        origin: Option<AnyWindowHandle>,
        excluded: &[WindowId],
        mut hwnd: impl FnMut(AnyWindowHandle) -> Option<isize>,
    ) -> Option<NativeOwner> {
        let preferred = self.target_window(origin);
        let (id, hwnd) = preferred
            .into_iter()
            .chain(
                self.windows
                    .iter()
                    .rev()
                    .copied()
                    .filter(|handle| Some(*handle) != preferred),
            )
            .filter(|handle| !excluded.contains(&handle.window_id()))
            .find_map(|handle| {
                hwnd(handle)
                    .filter(|&hwnd| hwnd != 0)
                    .map(|hwnd| (handle.window_id(), hwnd))
            })?;
        let hold = Arc::new(());
        self.native_owner = Some((id, Arc::downgrade(&hold)));
        Some(NativeOwner {
            hwnd,
            id,
            _hold: hold,
        })
    }
}

#[cfg(windows)]
pub(crate) struct NativeOwner {
    pub hwnd: isize,
    id: WindowId,
    _hold: Arc<()>,
}

#[cfg(windows)]
impl JobWorker {
    pub fn native_owner(&self) -> anyhow::Result<Option<NativeOwner>> {
        use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindow};
        let mut excluded = Vec::new();
        loop {
            let (send, receive) = bounded(1);
            let owner = gpui::block_on(self.cancel.until_cancelled(async {
                self.events
                    .send(WorkerEvent::Owner(excluded.clone(), send))
                    .await
                    .ok()?;
                receive.recv().await.ok().flatten()
            }))
            .flatten();
            let Some(owner) = owner else {
                if self.is_cancelled() {
                    return Ok(None);
                }
                anyhow::bail!("ファイル操作に使用できるウィンドウがありません");
            };
            if unsafe { IsWindow(Some(HWND(owner.hwnd as *mut _))).as_bool() } {
                return Ok(Some(owner));
            }
            excluded.push(owner.id);
        }
    }
}
