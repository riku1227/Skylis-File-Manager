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

    pub(crate) fn test_fail(&mut self, id: JobId, cx: &mut Context<Self>) {
        self.record_archive_completion(
            id,
            ArchiveCompletion {
                output: Err("解凍に失敗しました".into()),
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
                reason: PermanentDeleteReason::Explicit,
                known_bytes: None,
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
    let worker = JobWorker::new(CancelToken::new(), events);
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
        worker.progress.lock().unwrap().mailbox,
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
        worker.progress.lock().unwrap().mailbox,
        Some(measuring(200, TotalState::Known(200)))
    );
}

#[gpui::test]
fn resolved_transfer_kind_survives_progress_and_completion(cx: &mut gpui::TestAppContext) {
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    for (kinds, expected) in [
        ([TransferKind::Copy, TransferKind::Copy], JobKind::Copy),
        ([TransferKind::Move, TransferKind::Move], JobKind::Move),
        (
            [TransferKind::Move, TransferKind::Copy],
            JobKind::CopyAndMove,
        ),
    ] {
        let (events, _) = bounded(1);
        let worker = JobWorker::new(CancelToken::new(), events);
        worker.report_transfer_kind(kinds);
        let initial = worker.latest_progress().unwrap();
        worker.report(measuring(10, TotalState::Known(20)));
        let latest = worker.latest_progress().unwrap();
        assert_eq!(latest.resolved_kind, Some(expected));

        center.update(cx, |center, cx| {
            let id = center.test_job(window, cx);
            center.record(id).unwrap().job.kind = JobKind::CopyOrMove;
            center.update_progress(id, initial, cx);
            assert_eq!(center.record(id).unwrap().job.kind, expected);

            let (result, _) = bounded(1);
            center.finish_transfer(
                id,
                result,
                Ok(TransferOutcome::Completed),
                Some(latest),
                JobIssues::default(),
                cx,
            );
            let job = &center.record(id).unwrap().job;
            assert_eq!(job.kind, expected);
            assert_eq!(job.percent, Some(100));
        });
    }
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
    let mut worker = JobWorker::new(cancel.clone(), events);
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

#[gpui::test]
fn native_transfer_waits_in_a_bounded_fifo(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let (release, hold) = bounded::<()>(1);
    let (started, order) = bounded(13);
    let (running, queued) = center.update(cx, |center, cx| {
        let (running, _) = center
            .submit_transfer(
                JobKind::Trash,
                PathLease::default(),
                None,
                window,
                move |worker| {
                    hold.recv_blocking().unwrap();
                    assert!(worker.is_cancelled());
                    Ok(TransferOutcome::Aborted)
                },
                cx,
            )
            .unwrap();
        let mut queued = Vec::new();
        for number in 0..12 {
            let started = started.clone();
            queued.push(
                center
                    .submit_transfer(
                        JobKind::Copy,
                        PathLease::default(),
                        None,
                        window,
                        move |_| {
                            started.try_send(number).unwrap();
                            Ok(TransferOutcome::Completed)
                        },
                        cx,
                    )
                    .unwrap(),
            );
        }
        assert!(
            center
                .submit_transfer(
                    JobKind::Copy,
                    PathLease::default(),
                    None,
                    window,
                    |_| unreachable!(),
                    cx,
                )
                .is_err()
        );
        assert!(center.cancel(queued[2].0, cx));
        assert!(center.cancel(running, cx));
        (running, queued)
    });
    release.try_send(()).unwrap();
    drain_workers(&center, cx);
    assert_eq!(
        queued[2].1.try_recv().unwrap(),
        Ok(TransferOutcome::Aborted)
    );
    assert_eq!(
        std::iter::from_fn(|| order.try_recv().ok()).collect::<Vec<_>>(),
        (0..12).filter(|&n| n != 2).collect::<Vec<_>>()
    );
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == running && job.status == JobStatus::Cancelled)
    }));
}

fn drain_workers(center: &Entity<OperationCenter>, cx: &mut gpui::TestAppContext) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while center.read_with(cx, |center, _| center.has_active()) {
        assert!(Instant::now() < deadline, "workers should drain");
        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn permanent_delete_only_mutates_after_confirmation(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("test.txt");
    std::fs::write(&path, b"keep every byte").unwrap();
    for (reason, action) in [
        (
            PermanentDeleteReason::RecycleUnavailable,
            PromptAction::Cancel,
        ),
        (PermanentDeleteReason::Explicit, PromptAction::Confirm),
    ] {
        let selected = path.clone();
        let receiver = center.update(cx, |center, cx| {
            center.register_window(window);
            center
                .submit_transfer(
                    JobKind::PermanentDelete,
                    PathLease::default(),
                    None,
                    window,
                    move |worker| {
                        file_operations::confirm_permanent_delete(
                            &[selected],
                            reason,
                            worker,
                            |paths, _| {
                                std::fs::remove_file(&paths[0])?;
                                Ok(TransferOutcome::Completed)
                            },
                        )
                        .map_err(|error| format!("{error:#}"))
                    },
                    cx,
                )
                .unwrap()
                .1
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Some(request) = center.read_with(cx, |center, _| center.prompt_for(window)) {
                break request;
            }
            assert!(Instant::now() < deadline, "confirmation should be offered");
            cx.run_until_parked();
            std::thread::yield_now();
        };
        assert_eq!(std::fs::read(&path).unwrap(), b"keep every byte");
        assert_eq!(
            request.kind,
            PromptKind::ConfirmPermanentDelete {
                representative: path.clone(),
                items: 1,
                reason,
                known_bytes: Some(15),
            }
        );
        center.update(cx, |center, cx| {
            assert!(center.respond(
                PromptResponse {
                    job: request.job,
                    prompt: request.id,
                    action,
                    apply_to_all: false,
                },
                cx
            ))
        });
        drain_workers(&center, cx);
        if action == PromptAction::Cancel {
            assert_eq!(receiver.try_recv().unwrap(), Ok(TransferOutcome::Aborted));
            assert_eq!(std::fs::read(&path).unwrap(), b"keep every byte");
        } else {
            assert_eq!(receiver.try_recv().unwrap(), Ok(TransferOutcome::Completed));
            assert!(!path.exists());
        }
    }
}

#[gpui::test]
fn path_leases_serialize_both_lanes_in_submission_order(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let root = PathBuf::from("work");
    let source = root.join("source");
    let child = source.join("child");
    let output = root.join("output.zip");
    let (release_archive, archive_gate) = bounded::<()>(1);
    let (release_move, move_gate) = bounded::<()>(1);
    let (archive, moving, extract, delete) = center.update(cx, |center, cx| {
        let archive = center
            .submit_archive_with_lease(
                JobKind::ArchiveCreate(ArchiveFormat::Zip),
                PathLease::archive(&[source.clone()], Some(&output)),
                None,
                window,
                move |_| {
                    archive_gate.recv_blocking().unwrap();
                    Ok("output.zip".into())
                },
                cx,
            )
            .unwrap();
        let moving = center
            .submit_transfer(
                JobKind::Move,
                PathLease::transfer(
                    JobKind::Move,
                    &[child.clone()],
                    Some(&root.join("destination")),
                ),
                None,
                window,
                move |_| {
                    move_gate.recv_blocking().unwrap();
                    Ok(TransferOutcome::Completed)
                },
                cx,
            )
            .unwrap()
            .0;
        let extract = center
            .submit_archive_with_lease(
                JobKind::ArchiveExtract,
                PathLease::archive(&[output.clone()], Some(&child)),
                None,
                window,
                |_| Ok("child".into()),
                cx,
            )
            .unwrap();
        let delete = center
            .submit_transfer(
                JobKind::Trash,
                PathLease::transfer(JobKind::Trash, &[output], None),
                None,
                window,
                |_| Ok(TransferOutcome::Completed),
                cx,
            )
            .unwrap()
            .0;
        assert_eq!(center.archive_running, Some(archive));
        assert_eq!(center.transfer_running, None);
        for job in [moving, extract, delete] {
            assert_eq!(center.record(job).unwrap().job.status, JobStatus::Queued);
        }
        (archive, moving, extract, delete)
    });
    release_archive.try_send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !center.read_with(cx, |center, _| center.transfer_running == Some(moving)) {
        assert!(Instant::now() < deadline);
        cx.run_until_parked();
        std::thread::yield_now();
    }
    center.update(cx, |center, _| {
        assert_eq!(
            center.record(archive).unwrap().job.status,
            JobStatus::Completed
        );
        assert_eq!(
            center.record(extract).unwrap().job.status,
            JobStatus::Queued
        );
        assert_eq!(center.record(delete).unwrap().job.status, JobStatus::Queued);
    });
    release_move.try_send(()).unwrap();
    drain_workers(&center, cx);
    center.read_with(cx, |center, _| {
        let completed: Vec<_> = center.snapshots().rev().map(|job| job.id).collect();
        assert_eq!(completed, [archive, moving, extract, delete]);
    });
}

#[test]
fn unrelated_jobs_and_shared_copy_sources_do_not_conflict() {
    let root = PathBuf::from("work");
    let source = root.join("source");
    let child = source.join("file");
    let archive = PathLease::archive(&[source.clone()], Some(&root.join("archive.zip")));
    let copy = PathLease::transfer(JobKind::Copy, &[child.clone()], Some(&root.join("copy")));
    let unrelated = PathLease::transfer(JobKind::PermanentDelete, &[root.join("unrelated")], None);
    let remove_parent = PathLease::transfer(JobKind::Trash, &[root], None);
    assert!(!archive.conflicts(&copy));
    assert!(!copy.conflicts(&archive));
    assert!(!archive.conflicts(&unrelated));
    assert!(archive.conflicts(&remove_parent));
    assert!(remove_parent.conflicts(&copy));
    for kind in [
        JobKind::Move,
        JobKind::CopyOrMove,
        JobKind::CopyAndMove,
        JobKind::Trash,
        JobKind::PermanentDelete,
    ] {
        assert!(archive.conflicts(&PathLease::transfer(
            kind,
            std::slice::from_ref(&child),
            None
        )));
    }
}

#[cfg(windows)]
#[test]
fn leases_match_extended_and_case_varied_windows_paths() {
    let archive = PathLease::archive(&[PathBuf::from(r"\\?\UNC\Server\Share\Area")], None);
    let delete = PathLease::transfer(
        JobKind::Trash,
        &[PathBuf::from(r"\\server\share\AREA\File")],
        None,
    );
    assert!(archive.conflicts(&delete));
    let archive = PathLease::archive(&[PathBuf::from(r"\\?\C:\Work")], None);
    let delete = PathLease::transfer(JobKind::Trash, &[PathBuf::from(r"c:/work/File")], None);
    assert!(archive.conflicts(&delete));
}

#[cfg(windows)]
#[gpui::test]
fn a_transfer_resolves_a_live_owner_after_the_origin_closes(cx: &mut gpui::TestAppContext) {
    let origin = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let survivor = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    center.update(cx, |center, cx| {
        center.register_window(origin);
        center.register_window(survivor);
        center.unregister_window(origin.window_id(), cx);
        let owner = center
            .find_window_owner(Some(origin), &[], |window| {
                (window == survivor).then_some(23)
            })
            .expect("surviving window owns the native operation");
        assert_eq!(owner.hwnd, 23);
        assert!(!center.native_owned_by(origin.window_id()));
        assert!(center.native_owned_by(survivor.window_id()));
        drop(owner);
        assert!(!center.native_owned_by(survivor.window_id()));
        center.unregister_window(survivor.window_id(), cx);
        assert!(
            center
                .find_window_owner(Some(origin), &[], |_| unreachable!())
                .is_none()
        );
    });
}

#[cfg(windows)]
#[gpui::test]
fn no_owner_fails_the_queued_transfer_before_native_io(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let (release, hold) = bounded::<()>(1);
    let (id, result) = center.update(cx, |center, cx| {
        center.register_window(window);
        center
            .submit_transfer(
                JobKind::Trash,
                PathLease::default(),
                None,
                window,
                move |_| {
                    hold.recv_blocking().unwrap();
                    Ok(TransferOutcome::Completed)
                },
                cx,
            )
            .unwrap();
        let queued = center
            .submit_transfer(
                JobKind::Copy,
                PathLease::default(),
                None,
                window,
                |worker| {
                    worker
                        .native_owner()
                        .map(|_| panic!("native work requires a valid owner"))
                        .map_err(|error| error.to_string())
                },
                cx,
            )
            .unwrap();
        assert_eq!(
            center.record(queued.0).unwrap().job.status,
            JobStatus::Queued
        );
        center.unregister_window(window.window_id(), cx);
        queued
    });
    release.try_send(()).unwrap();
    drain_workers(&center, cx);
    assert!(
        result
            .try_recv()
            .unwrap()
            .unwrap_err()
            .contains("ウィンドウがありません")
    );
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == id && job.status == JobStatus::Failed)
    }));
}

#[gpui::test]
fn failed_archive_send_releases_lease_and_wakes_queued_transfer(cx: &mut gpui::TestAppContext) {
    cx.background_executor.allow_parking();
    let window = cx
        .add_empty_window()
        .update(|window, _| window.window_handle());
    let center = cx.update(init);
    let shared = PathBuf::from("shared");
    let (release_holder, holder_gate) = bounded::<()>(1);
    let (holder, failed_archive, transfer_id, transfer_rx) = center.update(cx, |center, cx| {
        let holder = center
            .submit_archive_with_lease(
                JobKind::ArchiveCreate(ArchiveFormat::Zip),
                PathLease::archive(&[PathBuf::from("other")], None),
                None,
                window,
                move |_| {
                    holder_gate.recv_blocking().unwrap();
                    Ok("holder.zip".into())
                },
                cx,
            )
            .unwrap();
        let (closed_tx, closed_rx) = bounded::<ArchiveCommand>(1);
        drop(closed_rx);
        center.archive_worker = Some(closed_tx);
        let failed_archive = center
            .submit_archive_with_lease(
                JobKind::ArchiveCreate(ArchiveFormat::Zip),
                PathLease::archive(std::slice::from_ref(&shared), None),
                None,
                window,
                |_| Ok("unused.zip".into()),
                cx,
            )
            .unwrap();
        let (transfer_id, transfer_rx) = center
            .submit_transfer(
                JobKind::Trash,
                PathLease::transfer(JobKind::Trash, std::slice::from_ref(&shared), None),
                None,
                window,
                |_| Ok(TransferOutcome::Completed),
                cx,
            )
            .unwrap();
        assert_eq!(center.archive_running, Some(holder));
        assert_eq!(center.transfer_running, None);
        assert_eq!(
            center.record(transfer_id).unwrap().job.status,
            JobStatus::Queued
        );
        (holder, failed_archive, transfer_id, transfer_rx)
    });
    release_holder.try_send(()).unwrap();
    drain_workers(&center, cx);
    assert_eq!(
        transfer_rx.try_recv().unwrap(),
        Ok(TransferOutcome::Completed)
    );
    center.read_with(cx, |center, _| {
        assert!(
            center
                .snapshots()
                .any(|job| job.id == holder && job.status == JobStatus::Completed)
        );
        assert!(
            center
                .snapshots()
                .any(|job| job.id == failed_archive && job.status == JobStatus::Failed)
        );
        assert!(
            center
                .snapshots()
                .any(|job| job.id == transfer_id && job.status == JobStatus::Completed)
        );
    });
}
