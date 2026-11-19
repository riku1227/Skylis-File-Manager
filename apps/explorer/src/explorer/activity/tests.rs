use super::*;
use crate::operation_center::JobId;
use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};

fn explorer(cx: &mut TestAppContext, display: ActivityDisplay) -> WindowHandle<Explorer> {
    cx.background_executor.allow_parking();
    cx.update(|cx| {
        if !cx.has_global::<crate::settings::SharedPreferences>() {
            let prefs = cx.new(|cx| {
                crate::settings::Preferences::new(
                    (
                        crate::settings_data::Settings {
                            activity_display: display,
                            ..Default::default()
                        },
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

#[gpui::test]
fn pill_and_popup_follow_job_across_tab_switch_close_and_detach(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::StatusBar);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    draw(&mut visual);
    assert!(visual.debug_bounds("activity-pill").is_none());
    let (center, id) = start(window, cx);
    center.update(cx, |center, cx| center.test_report_halfway(id, cx));
    click(&mut visual, "activity-pill");
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
    assert!(other.debug_bounds("activity-pill").is_some());
    assert!(center.read_with(cx, |center, _| {
        center
            .snapshots()
            .any(|job| job.id == id && job.percent == Some(50))
    }));
    center.update(cx, |center, cx| center.test_finish(id, cx));
    draw(&mut visual);
    draw(&mut other);
    assert!(visual.debug_bounds("activity-pill").is_none());
    assert!(other.debug_bounds("activity-pill").is_none());
    assert!(visual.debug_bounds("activity-popover").is_some());
}

#[gpui::test]
fn pop_out_uses_the_shared_list_and_closes_with_the_last_main_window(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::StatusBar);
    let (center, id) = start(window, cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    click(&mut visual, "activity-pill");
    click(&mut visual, "activity-pop-out");
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    let child = center
        .read_with(cx, |center, _| center.activity_window)
        .expect("activity window");
    let mut child_visual = VisualTestContext::from_window(child, cx);
    draw(&mut child_visual);
    assert!(child_visual.debug_bounds("activity-list").is_some());
    click(&mut visual, "activity-pill");
    assert_eq!(
        center.read_with(cx, |center, _| center.activity_window),
        Some(child)
    );

    center.update(cx, |center, cx| center.test_finish(id, cx));
    assert!(visual.simulate_close());
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    assert!(child.update(cx, |_, _, _| ()).is_err());
}

#[gpui::test]
fn automatic_window_opens_only_while_a_job_is_still_running_after_500ms(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::Window);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, short) = start(window, cx);
    draw(&mut visual);
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(Duration::from_millis(499));
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    center.update(cx, |center, cx| center.test_finish(short, cx));
    cx.background_executor
        .advance_clock(Duration::from_millis(50));
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));

    let (_, long) = start(window, cx);
    draw(&mut visual);
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(Duration::from_millis(499));
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    cx.background_executor
        .advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    let child = center
        .read_with(cx, |center, _| center.activity_window)
        .expect("window at 500ms");
    child.update(cx, |_, w, _| w.remove_window()).unwrap();
    draw(&mut visual);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    center.update(cx, |center, cx| center.test_finish(long, cx));
}

#[gpui::test]
fn prompt_is_routed_to_origin_guards_enter_and_restores_focus(cx: &mut TestAppContext) {
    let origin = explorer(cx, ActivityDisplay::StatusBar);
    let other = explorer(cx, ActivityDisplay::StatusBar);
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
    assert!(background.debug_bounds("activity-pill").is_some());
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
    let origin = explorer(cx, ActivityDisplay::StatusBar);
    let other = explorer(cx, ActivityDisplay::StatusBar);
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
fn exit_ghost_absorbs_pop_out_click_and_quit_exposes_active_jobs(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::StatusBar);
    let (center, id) = start(window, cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
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
    let pop_out = visual.debug_bounds("activity-pop-out").unwrap().center();
    cx.simulate_keystrokes(window.into(), "escape");
    draw(&mut visual);
    assert!(visual.debug_bounds("motion-ghost-shield").is_some());
    visual.simulate_click(pop_out, Modifiers::none());
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn clicking_outside_popover_closes_it_and_pill_toggles_cleanly(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::StatusBar);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    click(&mut visual, "activity-pill");
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

    click(&mut visual, "activity-pill");
    assert!(
        window
            .read_with(cx, |view, _| view.activity.popup_open)
            .unwrap()
    );

    click(&mut visual, "activity-pill");
    assert!(
        window
            .read_with(cx, |view, _| !view.activity.popup_open)
            .unwrap()
    );
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn settings_modal_blocks_toggle_activity_and_closes_open_popover(cx: &mut TestAppContext) {
    let window = explorer(cx, ActivityDisplay::StatusBar);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let (center, id) = start(window, cx);
    click(&mut visual, "activity-pill");
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
    assert!(center.read_with(cx, |center, _| center.activity_window.is_none()));
    center.update(cx, |center, cx| center.test_finish(id, cx));
}

#[gpui::test]
fn multiple_jobs_render_distinct_controls_and_open_destination_in_clicked_window(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let archive_path = dir.path().join("result.zip");
    std::fs::write(&archive_path, b"zip").unwrap();
    let origin = explorer(cx, ActivityDisplay::StatusBar);
    let other = explorer(cx, ActivityDisplay::StatusBar);
    let (center, id1) = start(origin, cx);
    let id2 = center.update(cx, |center, cx| center.test_job(origin.into(), cx));
    assert_eq!((id1.0, id2.0), (1, 2));
    center.update(cx, |center, cx| {
        center.test_finish_at(id1, archive_path.clone(), cx)
    });

    let mut other_visual = VisualTestContext::from_window(other.into(), cx);
    click(&mut other_visual, "activity-pill");
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
    let window = explorer(cx, ActivityDisplay::StatusBar);
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
