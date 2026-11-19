use super::*;
use std::time::Duration;

impl OperationCenter {
    pub(crate) fn test_job(&mut self, origin: AnyWindowHandle, cx: &mut Context<Self>) -> JobId {
        let id = self
            .insert(
                JobKind::ArchiveExtract,
                JobBackend::Builtin,
                None,
                origin,
                Some(CancelToken::new()),
                cx,
            )
            .unwrap();
        self.record(id).unwrap().job.status = JobStatus::Running;
        id
    }

    pub(crate) fn test_finish(&mut self, id: JobId, cx: &mut Context<Self>) {
        self.test_finish_at(id, PathBuf::from("test.zip"), cx);
    }

    pub(crate) fn test_finish_at(&mut self, id: JobId, output: PathBuf, cx: &mut Context<Self>) {
        self.record_archive_completion(
            id,
            ArchiveCompletion {
                output: Ok(output),
                affected_parents: Vec::new(),
                cancelled: false,
            },
            cx,
        );
    }

    pub(crate) fn test_prompt_kind(
        &mut self,
        id: JobId,
        kind: PromptKind,
        cx: &mut Context<Self>,
    ) -> Receiver<PromptResponse> {
        let (send, receive) = bounded(1);
        self.install_prompt(id, kind, send, cx);
        receive
    }

    pub(crate) fn test_report_halfway(&mut self, id: JobId, cx: &mut Context<Self>) {
        self.update_progress(id, measuring(10, TotalState::Known(20)), cx);
    }

    pub(crate) fn test_prompt(
        &mut self,
        id: JobId,
        cx: &mut Context<Self>,
    ) -> Receiver<PromptResponse> {
        let (send, receive) = bounded(1);
        self.install_prompt(
            id,
            PromptKind::ConfirmPermanentDelete {
                representative: PathBuf::from("test.txt"),
                items: 1,
            },
            send,
            cx,
        );
        receive
    }
}

fn measuring(done: u64, total: TotalState) -> JobProgress {
    JobProgress {
        measure: ProgressMeasure::Measured {
            bytes_done: done,
            bytes_total: total,
            items_done: done,
            items_total: total,
            bytes_per_sec: None,
            eta_seconds: None,
        },
        ..JobProgress::default()
    }
}

#[test]
fn archive_updates_coalesce_at_100ms_with_one_pending_signal() {
    let (events, receiver) = bounded(1);
    let mailbox = Arc::new(Mutex::new(None));
    let mut worker = JobWorker {
        cancel: CancelToken::new(),
        mailbox: mailbox.clone(),
        events,
        last_publish: None,
        newest: None,
    };
    let start = Instant::now();
    worker.report_at(measuring(1, TotalState::Known(200)), start);
    for done in 2..=99 {
        worker.report_at(
            measuring(done, TotalState::Unknown),
            start + Duration::from_millis(done),
        );
    }
    assert!(matches!(receiver.try_recv(), Ok(WorkerEvent::Progress)));
    assert!(receiver.is_empty());
    assert_eq!(
        *mailbox.lock().unwrap(),
        Some(measuring(1, TotalState::Known(200)))
    );
    worker.report_at(
        measuring(100, TotalState::Unknown),
        start + Duration::from_millis(100),
    );
    worker.report_at(
        measuring(200, TotalState::Unknown),
        start + Duration::from_millis(200),
    );
    assert_eq!(receiver.len(), 1);
    assert_eq!(
        *mailbox.lock().unwrap(),
        Some(measuring(200, TotalState::Known(200)))
    );
}

#[gpui::test]
fn measured_total_and_percent_are_monotonic_until_success(cx: &mut gpui::TestAppContext) {
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    cx.update(|cx| {
        let center = init(cx);
        center.update(cx, |center, cx| {
            let id = center.test_job(window, cx);
            let job = &mut center.record(id).unwrap().job;
            job.update(measuring(5, TotalState::Counting { lower_bound: 10 }));
            assert_eq!(job.percent, None);
            job.update(measuring(6, TotalState::Unknown));
            assert_eq!(
                job.progress,
                measuring(6, TotalState::Counting { lower_bound: 10 })
            );
            job.update(measuring(9, TotalState::Known(10)));
            assert_eq!(job.percent, Some(90));
            job.update(measuring(9, TotalState::Known(20)));
            assert_eq!(job.percent, Some(90));
            job.update(measuring(25, TotalState::Unknown));
            assert_eq!(job.progress, measuring(25, TotalState::Known(25)));
            assert_eq!(job.percent, Some(99));
            job.finish(JobStatus::Completed, None);
            assert_eq!(job.percent, Some(100));
        });
    });
}

#[gpui::test]
fn issue_count_and_message_bytes_have_independent_limits(cx: &mut gpui::TestAppContext) {
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    cx.update(|cx| {
        init(cx).update(cx, |center, cx| {
            let first = center.test_job(window, cx);
            let job = &mut center.record(first).unwrap().job;
            for _ in 0..300 {
                job.add_issue(JobIssue {
                    item: None,
                    message: "error".into(),
                });
            }
            assert_eq!(job.issues().len(), 256);
            assert_eq!(job.issue_count, 300);

            let second = center.test_job(window, cx);
            let job = &mut center.record(second).unwrap().job;
            for _ in 0..3 {
                job.add_issue(JobIssue {
                    item: Some("x".repeat(128 * 1024 - 1)),
                    message: "!".into(),
                });
            }
            assert_eq!(job.issues().len(), 2);
            assert_eq!(job.issue_count, 3);

            let third = center.test_job(window, cx);
            let job = &mut center.record(third).unwrap().job;
            job.add_issue(JobIssue {
                item: Some("あ".repeat(100_000)),
                message: "問題メッセージ".repeat(20_000),
            });
            assert_eq!(job.issues().len(), 1);
            assert_eq!(job.issue_count, 1);
            let recorded = &job.issues()[0];
            assert!(recorded.message.ends_with('…'));
            let bytes = recorded.item.as_ref().map_or(0, String::len) + recorded.message.len();
            assert!(bytes <= ISSUE_BYTES_LIMIT);
        });
    });
}

#[gpui::test]
fn history_keeps_active_jobs_and_reserves_slots_for_completion_delivery(
    cx: &mut gpui::TestAppContext,
) {
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let active = center.update(cx, |center, cx| {
        let active = center.test_job(window, cx);
        let _response = center.test_prompt(active, cx);
        for _ in 0..31 {
            let id = center.test_job(window, cx);
            center.test_finish(id, cx);
        }
        assert_eq!(center.completions.len(), 31);
        assert!(
            center
                .insert(JobKind::Trash, JobBackend::Builtin, None, window, None, cx)
                .is_err()
        );
        active
    });
    center.update(cx, |center, cx| {
        assert!(center.completions.is_empty());
        for _ in 0..10 {
            let id = center.test_job(window, cx);
            center.test_finish(id, cx);
        }
        assert_eq!(center.snapshots().count(), 32);
        assert!(matches!(
            center.record(active).unwrap().job.status,
            JobStatus::WaitingForUser(_)
        ));
        center.dismiss_finished(cx);
        assert_eq!(center.snapshots().count(), 1);
    });
}

#[test]
fn cancelling_a_prompt_wakes_the_worker_without_a_response() {
    let (events, receiver) = bounded(1);
    let cancel = CancelToken::new();
    let (done, result) = std::sync::mpsc::channel();
    let mut worker = JobWorker {
        cancel: cancel.clone(),
        mailbox: Arc::new(Mutex::new(None)),
        events,
        last_publish: None,
        newest: None,
    };
    let thread = std::thread::spawn(move || {
        let response = worker.prompt(PromptKind::ItemError {
            item: PathBuf::from("test"),
            message: "test".into(),
        });
        done.send(response).unwrap();
    });
    let WorkerEvent::Prompt(_, response) = receiver.recv_blocking().unwrap() else {
        panic!("prompt expected");
    };
    cancel.cancel();
    assert_eq!(result.recv_timeout(Duration::from_secs(2)).unwrap(), None);
    assert!(response.is_closed());
    thread.join().unwrap();
}

#[gpui::test]
fn archive_lane_is_bounded_and_success_after_cancellation_remains_completed(
    cx: &mut gpui::TestAppContext,
) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let (release, hold) = bounded::<()>(1);
    let running = center.update(cx, |center, cx| {
        let running = center
            .submit_archive(
                JobKind::ArchiveExtract,
                None,
                window,
                move |_| {
                    hold.recv_blocking().ok();
                    Ok("published.zip".into())
                },
                cx,
            )
            .unwrap();
        let mut queued = Vec::new();
        for _ in 0..4 {
            queued.push(
                center
                    .submit_archive(
                        JobKind::ArchiveExtract,
                        None,
                        window,
                        |_| Err("invalid".into()),
                        cx,
                    )
                    .unwrap(),
            );
        }
        assert!(
            center
                .submit_archive(
                    JobKind::ArchiveExtract,
                    None,
                    window,
                    |_| unreachable!(),
                    cx
                )
                .is_err()
        );
        assert!(center.cancel(queued[0], cx));
        assert_eq!(
            center.record(queued[0]).unwrap().job.status,
            JobStatus::Cancelled
        );
        assert!(center.cancel(running, cx));
        assert_eq!(
            center.record(running).unwrap().job.status,
            JobStatus::Cancelling
        );
        running
    });
    release.try_send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while center.read_with(cx, |center, _| center.has_active()) {
        assert!(Instant::now() < deadline, "workers should drain");
        cx.run_until_parked();
        std::thread::yield_now();
    }
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == running && job.status == JobStatus::Completed)
            && center
                .snapshots()
                .filter(|job| job.status == JobStatus::Failed)
                .count()
                == 3
    }));
}

#[gpui::test]
fn archive_worker_disconnect_releases_lane_and_allows_recovery(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let (cmd_tx, cmd_rx) = bounded::<ArchiveCommand>(1);
    std::thread::spawn(move || {
        let _dropped = cmd_rx.recv_blocking();
    });
    let first = center.update(cx, |center, cx| {
        center.archive_worker = Some(cmd_tx);
        center
            .submit_archive(
                JobKind::ArchiveExtract,
                None,
                window,
                |_| Ok("unused".into()),
                cx,
            )
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while center.read_with(cx, |center, _| center.has_active()) {
        assert!(Instant::now() < deadline, "disconnected job should fail");
        cx.run_until_parked();
        std::thread::yield_now();
    }
    center.read_with(cx, |center, _| {
        assert!(center.archive_worker.is_none());
        assert!(center.archive_running.is_none());
        assert!(
            center
                .snapshots()
                .any(|job| job.id == first && job.status == JobStatus::Failed)
        );
    });
    let second = center.update(cx, |center, cx| {
        center
            .submit_archive(
                JobKind::ArchiveExtract,
                None,
                window,
                |_| Ok("recovered.zip".into()),
                cx,
            )
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while center.read_with(cx, |center, _| center.has_active()) {
        assert!(Instant::now() < deadline, "replacement worker should run");
        cx.run_until_parked();
        std::thread::yield_now();
    }
    center.read_with(cx, |center, _| {
        assert!(center.snapshots().any(|job| {
            job.id == second
                && job.status == JobStatus::Completed
                && job.output == Some(PathBuf::from("recovered.zip"))
        }));
    });
}
