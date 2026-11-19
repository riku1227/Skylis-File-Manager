use super::*;
use crate::operation_center::JobId;
use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};

fn explorer(cx: &mut TestAppContext) -> WindowHandle<Explorer> {
    cx.background_executor.allow_parking();
    cx.update(|cx| {
        if !cx.has_global::<crate::settings::SharedPreferences>() {
            let prefs = cx.new(|cx| {
                crate::settings::Preferences::new(
                    (
                        Default::default(),
                        Some("memory-only test preferences".into()),
                    ),
                    cx,
                )
            });
            cx.set_global(crate::settings::SharedPreferences(prefs));
        }
    });
    cx.add_window(|w, cx| {
        let icons = cx.new(ShellIcons::new);
        let tab =
            cx.new(|cx| FolderView::new(Location::Home, ThemeMode::Light, icons.clone(), w, cx));
        Explorer::with_tabs(vec![tab], icons, ThemeMode::Light, false, w, cx)
    })
}

fn draw(visual: &mut VisualTestContext) {
    visual.update(|w, cx| w.draw(cx).clear(cx));
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    draw(visual);
    let point = visual.debug_bounds(selector).expect(selector).center();
    visual.simulate_click(point, Modifiers::none());
    draw(visual);
}

fn start(
    owner: WindowHandle<Explorer>,
    cx: &mut TestAppContext,
) -> (Entity<OperationCenter>, JobId) {
    let center = cx.update(operation_center::init);
    let id = center.update(cx, |center, cx| center.test_job(owner.into(), cx));
    (center, id)
}

#[test]
fn activity_ring_averages_only_running_jobs_and_tracks_alerts_separately() {
    use JobStatus::*;
    let mixed = [
        (Running, Some(20), false),
        (Running, Some(41), false),
        (Queued, None, false),
        (Paused, Some(100), false),
        (Cancelling, None, false),
        (WaitingForUser(PromptId(1)), None, false),
        (Completed, Some(100), false),
        (Failed, Some(75), true),
    ];
    let summary = ActivitySummary::from_jobs(mixed);
    assert_eq!(summary.running, 2);
    assert_eq!(summary.average(), Some(31));
    assert_eq!(summary.queued, 1);
    assert_eq!(summary.cancelling, 1);
    assert_eq!(summary.decisions, 1);
    assert_eq!(summary.errors, 1);
    assert!(summary.description().contains("待機中 1 件"));

    let summary = ActivitySummary::from_jobs(mixed.into_iter().chain([(Running, None, false)]));
    assert_eq!(summary.running, 3);
    assert_eq!(summary.average(), None);
    assert!(summary.unknown_progress);

    let summary = ActivitySummary::from_jobs([(Cancelling, None, false)]);
    assert_eq!(summary.running, 0);
    assert_eq!(summary.average(), None);
    assert!(!summary.unknown_progress);
    assert!(summary.description().contains("中止処理中 1 件"));
}

#[gpui::test]
fn idle_activity_trigger_opens_and_toggles_inside_the_explorer(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    let trigger = visual.debug_bounds("activity-trigger").unwrap();
    let search = visual.debug_bounds("name-search-field").unwrap();
    let view_settings = visual.debug_bounds("display-settings-trigger").unwrap();
    let settings = visual.debug_bounds("settings").unwrap();
    assert_eq!(trigger.size.width, trigger.size.height);
    assert!(search.right() <= trigger.left());
    assert!(trigger.right() <= view_settings.left());
    assert!(view_settings.right() <= settings.left());
    click(&mut visual, "activity-trigger");
    let popup = visual.debug_bounds("activity-popover").unwrap();
    assert!(popup.top() >= trigger.bottom());
    assert_eq!(popup.right(), trigger.right());
    click(&mut visual, "activity-trigger");
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
}

#[gpui::test]
fn toolbar_ring_is_keyboard_reachable_and_owns_its_activation_keys(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        bind_trigger_keys(cx);
        cx.bind_keys([
            gpui::KeyBinding::new("enter", crate::OpenSelected, Some("Explorer")),
            gpui::KeyBinding::new("space", crate::QuickPreview, Some("Explorer")),
        ]);
    });
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    cx.simulate_keystrokes(window.into(), "tab enter");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_some());
    cx.simulate_keystrokes(window.into(), "escape space");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_some());
    cx.simulate_keystrokes(window.into(), "escape");
    draw(&mut visual);
    click(&mut visual, "activity-trigger");
    cx.simulate_keystrokes(window.into(), "escape enter");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_some());
}

#[gpui::test]
fn activity_shortcut_and_escape_restore_focus_and_keep_the_popup_in_view(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.bind_keys([gpui::KeyBinding::new(
            "ctrl-shift-j",
            ToggleActivity,
            Some("ExplorerTabs"),
        )]);
    });
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    let previous = window.update(cx, |_, w, cx| w.focused(cx)).unwrap();
    cx.simulate_keystrokes(window.into(), "ctrl-shift-j");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_some());
    visual.simulate_resize(gpui::size(px(420.), px(280.)));
    draw(&mut visual);
    visual.update(|w, cx| w.simulate_next_frame(cx));
    draw(&mut visual);
    let trigger = visual.debug_bounds("activity-trigger").unwrap();
    let popup = visual.debug_bounds("activity-popover").unwrap();
    assert!(popup.left() >= px(8.));
    assert!(popup.right() <= px(412.));
    assert!(popup.top() >= trigger.bottom());
    assert!(popup.bottom() <= px(272.));

    cx.simulate_keystrokes(window.into(), "ctrl-shift-j");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_none());
    cx.simulate_keystrokes(window.into(), "ctrl-shift-j");
    draw(&mut visual);
    cx.simulate_keystrokes(window.into(), "tab");
    cx.simulate_keystrokes(window.into(), "escape");
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_none());
    assert_eq!(
        window.update(cx, |_, w, cx| w.focused(cx)).unwrap(),
        previous
    );
}

#[gpui::test]
fn ring_and_popup_follow_job_across_tab_switch_close_and_detach(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-trigger").is_some());
    let (center, id) = start(window, cx);
    center.update(cx, |center, cx| center.test_report_halfway(id, cx));
    click(&mut visual, "activity-trigger");
    assert!(visual.debug_bounds("activity-popover").is_some());

    window
        .update(cx, |view, w, cx| {
            view.new_tab(&NewTab, w, cx);
            view.switch(0, w, cx);
            view.close(1, w, cx);
            view.new_tab(&NewTab, w, cx);
            view.detach_tab(&DetachTab, w, cx);
            assert!(view.activity.popup_open);
        })
        .unwrap();
    let detached = cx
        .update(|cx| {
            cx.windows()
                .into_iter()
                .find_map(|w| w.downcast::<Explorer>().filter(|w| *w != window))
        })
        .expect("detached explorer");
    let mut other = VisualTestContext::from_window(detached.into(), cx);
    draw(&mut other);
    assert!(other.debug_bounds("activity-trigger").is_some());
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == id && job.percent == Some(50))
    }));
    center.update(cx, |center, cx| center.test_finish(id, cx));
    draw(&mut visual);
    draw(&mut other);
    assert!(visual.debug_bounds("activity-trigger").is_some());
    assert!(other.debug_bounds("activity-trigger").is_some());
    assert!(visual.debug_bounds("activity-popover").is_some());
    click(&mut other, "activity-trigger");
    assert!(other.debug_bounds("activity-clear").is_some());
}

#[gpui::test]
fn running_job_leaves_progress_in_the_toolbar_until_requested(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    draw(&mut visual);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-trigger").is_some());
    assert!(visual.debug_bounds("activity-popover").is_none());
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn completed_history_has_its_own_section_and_keeps_failures_visible(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let (center, completed) = start(window, cx);
    center.update(cx, |center, cx| center.test_finish(completed, cx));
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    click(&mut visual, "activity-trigger");
    assert!(visual.debug_bounds("activity-ring-summary").is_none());
    let completed_heading = visual.debug_bounds("activity-completed-heading").unwrap();
    let status = visual.debug_bounds("activity-completed-status-1").unwrap();
    assert!(completed_heading.bottom() <= status.top());
    let destination = visual.debug_bounds("activity-open-destination-1").unwrap();
    assert!(destination.size.width < completed_heading.size.width);

    let (_, failed) = start(window, cx);
    center.update(cx, |center, cx| center.test_fail(failed, cx));
    let _ = start(window, cx);
    draw(&mut visual);
    let failure_heading = visual.debug_bounds("activity-errors-heading").unwrap();
    let running_heading = visual.debug_bounds("activity-running-heading").unwrap();
    let completed_heading = visual.debug_bounds("activity-completed-heading").unwrap();
    assert!(visual.debug_bounds("activity-ring-summary").is_some());
    assert!(failure_heading.bottom() <= running_heading.top());
    assert!(running_heading.bottom() <= completed_heading.top());
}

#[gpui::test]
fn terminal_error_is_badged_and_remains_in_history_without_a_blocking_sheet(
    cx: &mut TestAppContext,
) {
    let window = explorer(cx);
    let (center, id) = start(window, cx);
    center.update(cx, |center, cx| center.test_fail(id, cx));
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-error-badge").is_some());
    assert!(visual.debug_bounds("activity-confirmation-sheet").is_none());
    assert!(visual.debug_bounds("activity-popover").is_none());

    click(&mut visual, "activity-trigger");
    assert!(visual.debug_bounds("activity-clear").is_some());
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == id && job.status == JobStatus::Failed)
    }));
    click(&mut visual, "activity-clear");
    assert!(center.read_with(cx, |center, _| center.snapshots().next().is_none()));
}

#[gpui::test]
fn arriving_prompt_takes_focus_from_open_popover_and_restores_folder_focus(
    cx: &mut TestAppContext,
) {
    let window = explorer(cx);
    let (center, id) = start(window, cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    let previous = window
        .update(cx, |view, w, cx| {
            let previous = w.focused(cx);
            view.show_activity(w, cx);
            previous
        })
        .unwrap();
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-popover").is_some());

    let response = center.update(cx, |center, cx| center.test_prompt(id, cx));
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-confirmation-sheet").is_some());
    window
        .update(cx, |view, w, _| {
            assert!(!view.activity.popup_open);
            assert!(view.activity.sheet_focus.is_focused(w));
        })
        .unwrap();
    cx.simulate_keystrokes(window.into(), "escape");
    assert_eq!(response.try_recv().unwrap().action, PromptAction::Cancel);
    draw(&mut visual);
    assert_eq!(
        window.update(cx, |_, w, cx| w.focused(cx)).unwrap(),
        previous
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn prompt_is_routed_to_origin_guards_enter_and_restores_focus(cx: &mut TestAppContext) {
    let origin = explorer(cx);
    let other = explorer(cx);
    let keys = ["enter", "right", "enter"];
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "ctrl-t",
            NewTab,
            Some("ExplorerTabs"),
        )])
    });
    let (center, id) = start(origin, cx);
    let response = center.update(cx, |center, cx| center.test_prompt(id, cx));
    let mut source = VisualTestContext::from_window(origin.into(), cx);
    let mut background = VisualTestContext::from_window(other.into(), cx);
    draw(&mut source);
    draw(&mut background);
    assert!(source.debug_bounds("activity-confirmation-sheet").is_some());
    assert!(
        background
            .debug_bounds("activity-confirmation-sheet")
            .is_none()
    );
    assert!(source.debug_bounds("activity-popover").is_none());
    assert!(background.debug_bounds("activity-decision-badge").is_some());
    click(&mut background, "activity-trigger");
    assert!(background.debug_bounds("activity-popover").is_some());
    assert!(background.debug_bounds("activity-cancel-1").is_none());
    assert!(
        background
            .debug_bounds("activity-confirm-action-1")
            .is_none()
    );
    origin
        .update(cx, |view, w, _| {
            assert!(view.activity.sheet_focus.is_focused(w));
            assert_eq!(view.activity.prompt.as_ref().unwrap().selected, 0);
        })
        .unwrap();
    for key in keys {
        cx.simulate_keystrokes(origin.into(), key);
    }
    cx.simulate_keystrokes(origin.into(), "ctrl-t");
    assert_eq!(origin.read_with(cx, |view, _| view.tabs.len()).unwrap(), 1);
    assert!(response.try_recv().is_err());
    cx.background_executor
        .advance_clock(Duration::from_millis(400));
    cx.simulate_keystrokes(origin.into(), "enter");
    let response = response.try_recv().unwrap();
    assert_eq!(response.action, PromptAction::Confirm);
    assert!(!center.update(cx, |center, cx| center.respond(response, cx)));
    draw(&mut source);
    origin
        .update(cx, |view, w, cx| {
            assert!(view.tabs[view.active].focus_handle(cx).is_focused(w));
        })
        .unwrap();
}

#[gpui::test]
fn closing_the_prompt_origin_routes_it_to_the_last_active_explorer(cx: &mut TestAppContext) {
    let origin = explorer(cx);
    let other = explorer(cx);
    let (center, id) = start(origin, cx);
    center.update(cx, |center, cx| center.activate_window(other.into(), cx));
    let response = center.update(cx, |center, cx| center.test_prompt(id, cx));
    let prompt = center
        .read_with(cx, |center, _| center.prompt_for(origin.into()))
        .unwrap();

    let mut source = VisualTestContext::from_window(origin.into(), cx);
    assert!(source.simulate_close());
    origin.update(cx, |_, w, _| w.remove_window()).unwrap();
    let mut background = VisualTestContext::from_window(other.into(), cx);
    draw(&mut background);
    assert!(
        background
            .debug_bounds("activity-confirmation-sheet")
            .is_some()
    );
    cx.simulate_keystrokes(other.into(), "escape");
    assert_eq!(response.try_recv().unwrap().action, PromptAction::Cancel);
    assert!(!center.update(cx, |center, cx| center.respond(
        PromptResponse {
            job: id,
            prompt: prompt.id,
            action: PromptAction::Confirm,
            apply_to_all: false,
        },
        cx
    )));
}

#[gpui::test]
fn exit_ghost_absorbs_history_clear_click_and_quit_exposes_active_jobs(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let (center, id) = start(window, cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (_, finished) = start(window, cx);
    center.update(cx, |center, cx| center.test_finish(finished, cx));
    cx.update(crate::explorer::request_quit);
    draw(&mut visual);
    assert!(
        window
            .read_with(cx, |view, _| view.activity.popup_open
                && view.tab_error.is_some())
            .unwrap()
    );
    assert!(!visual.simulate_close());
    cx.background_executor
        .advance_clock(Duration::from_millis(160));
    draw(&mut visual);
    let clear = visual.debug_bounds("activity-clear").unwrap().center();
    cx.simulate_keystrokes(window.into(), "escape");
    draw(&mut visual);
    assert!(visual.debug_bounds("motion-ghost-shield").is_some());
    visual.simulate_click(clear, Modifiers::none());
    assert!(center.read_with(cx, |center, _| {
        center.snapshots().any(|job| job.id == finished)
    }));
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn clicking_outside_popover_closes_it_and_trigger_toggles_cleanly(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    click(&mut visual, "activity-trigger");
    assert!(
        window
            .read_with(cx, |view, _| view.activity.popup_open)
            .unwrap()
    );

    visual.simulate_click(gpui::point(px(40.), px(120.)), Modifiers::none());
    draw(&mut visual);
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );

    click(&mut visual, "activity-trigger");
    assert!(
        window
            .read_with(cx, |view, _| view.activity.popup_open)
            .unwrap()
    );

    click(&mut visual, "activity-trigger");
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn settings_modal_blocks_toggle_activity_and_closes_open_popover(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    click(&mut visual, "activity-trigger");
    assert!(
        window
            .read_with(cx, |view, _| view.activity.popup_open)
            .unwrap()
    );

    window
        .update(cx, |_, w, cx| w.dispatch_action(Box::new(OpenSettings), cx))
        .unwrap();
    draw(&mut visual);
    assert!(
        window
            .read_with(cx, |view, _| view.settings_open
                && !view.activity.popup_open)
            .unwrap()
    );

    window
        .update(cx, |_, w, cx| {
            w.dispatch_action(Box::new(ToggleActivity), cx)
        })
        .unwrap();
    draw(&mut visual);
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn multiple_jobs_render_distinct_controls_and_open_destination_in_clicked_window(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let archive_path = dir.path().join("result.zip");
    std::fs::write(&archive_path, b"zip").unwrap();
    let origin = explorer(cx);
    let other = explorer(cx);
    let (center, id1) = start(origin, cx);
    let id2 = center.update(cx, |center, cx| center.test_job(origin.into(), cx));
    assert_eq!((id1.0, id2.0), (1, 2));
    center.update(cx, |center, cx| {
        center.test_finish_at(id1, archive_path.clone(), cx)
    });

    let mut other_visual = VisualTestContext::from_window(other.into(), cx);
    click(&mut other_visual, "activity-trigger");
    assert!(other_visual.debug_bounds("activity-cancel-2").is_some());
    click(&mut other_visual, "activity-open-destination-1");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        let selected = other
            .read_with(cx, |view, cx| {
                let tab = view.tabs[view.active].read(cx);
                (tab.path().map(|p| p.to_path_buf()), tab.selected_name())
            })
            .unwrap();
        if selected.0.as_deref() == Some(dir.path()) && selected.1.as_deref() == Some("result.zip")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "clicked window should navigate and select output"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        origin
            .read_with(cx, |view, cx| view.tabs[view.active]
                .read(cx)
                .path()
                .map(|p| p.to_path_buf()))
            .unwrap(),
        None
    );
    assert!(
        other
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id2, cx));
}

#[gpui::test]
fn ghost_sheet_ignores_apply_all_and_modal_blocks_duplicate_tab(cx: &mut TestAppContext) {
    let window = explorer(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    let _rx = center.update(cx, |center, cx| {
        center.test_prompt_kind(
            id,
            PromptKind::FileConflict {
                source: PathBuf::from("C:/a/file.txt"),
                destination: PathBuf::from("C:/b/file.txt"),
                allow_replace: true,
                allow_apply_to_all: true,
            },
            cx,
        )
    });
    draw(&mut visual);
    cx.background_executor
        .advance_clock(Duration::from_millis(160));
    draw(&mut visual);
    window
        .update(cx, |_, w, cx| {
            w.dispatch_action(Box::new(tab_workflows::DuplicateTab), cx)
        })
        .unwrap();
    assert_eq!(window.read_with(cx, |view, _| view.tabs.len()).unwrap(), 1);

    let apply_all = visual
        .debug_bounds("activity-apply-all")
        .expect("apply-all bounds")
        .center();
    cx.simulate_keystrokes(window.into(), "escape");
    draw(&mut visual);
    assert!(visual.debug_bounds("motion-ghost-shield").is_some());
    visual.simulate_click(apply_all, Modifiers::none());
    draw(&mut visual);
    assert!(
        window
            .read_with(cx, |view, _| view.activity.prompt.is_none())
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}
