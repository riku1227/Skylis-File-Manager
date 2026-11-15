use super::*;
use std::{cell::Cell, rc::Rc};

struct HoverProbe(Rc<Cell<usize>>);
impl Render for HoverProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div()
            .size_full()
            .child(div().id("row").w(px(200.)).h(px(28.)).child(row_surface(
                Theme::light(),
                false,
                false,
            )))
    }
}

#[gpui::test]
fn row_hover_repaints_on_enter_and_exit_without_scroll(cx: &mut gpui::TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let counter = renders.clone();
    let window = cx.add_window(move |_, _| HoverProbe(counter));
    let handle = gpui::AnyWindowHandle::from(window);
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(300.), px(100.)), cx);
        w.draw(cx).clear(cx);
    })
    .unwrap();
    let before = renders.get();
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(50.), px(14.)), cx)
    })
    .unwrap();
    assert!(
        renders.get() > before,
        "enter must repaint without scrolling"
    );
    let entered = renders.get();
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(60.), px(14.)), cx)
    })
    .unwrap();
    assert_eq!(
        renders.get(),
        entered,
        "movement inside one row must not redraw continuously"
    );
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(300.), px(100.)), cx)
    })
    .unwrap();
    assert!(
        renders.get() > entered,
        "exit must repaint without scrolling"
    );
}

fn home_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
    // These integration checks use real directory/drive workers, not the deterministic executor.
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    cx.add_window(|w, cx| {
        let icons = cx.new(ShellIcons::new);
        FolderView::new(Location::Home, ThemeMode::Light, icons, w, cx)
    })
}

#[gpui::test]
fn toolbar_can_be_constructed_with_debug_assertions(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    window
        .update(cx, |view, w, cx| {
            let _ = view
                .render_toolbar(Theme::light(), w, cx)
                .into_any_element();
        })
        .unwrap();
}

#[gpui::test]
fn grouped_headers_are_not_selectable_and_scroll_maps_to_entry_rows(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    let mut entries = Vec::new();
    list_dir(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &CancelToken::new(),
        |event| {
            if let ListingEvent::Batch(batch) = event {
                entries.extend(batch);
            }
            true
        },
    );
    assert!(entries.len() >= 2);
    entries[0].modified = Some(std::time::SystemTime::UNIX_EPOCH);
    entries[1].modified = Some(std::time::SystemTime::now());
    window
        .update(cx, |view, _, _| {
            view.entries = entries;
            view.grouped = true;
            view.rebuild_visible(None);
            assert!(matches!(view.display_rows[0], DisplayRow::Header(_, _)));
            assert_eq!(view.entry_rows.len(), view.visible.len());
            for (ix, &row) in view.entry_rows.iter().enumerate() {
                assert!(matches!(view.display_rows[row], DisplayRow::Entry(index) if index == ix));
            }
            let keep = view.entries[view.visible[0]].path.clone();
            view.rebuild_visible(Some(&keep));
            assert_eq!(view.selected_entry().unwrap().path, keep);
            view.scroll_to_entry(0, ScrollStrategy::Nearest);
            assert_eq!(
                view.scroll
                    .0
                    .borrow()
                    .deferred_scroll_to_item
                    .unwrap()
                    .item_index,
                1
            );
        })
        .unwrap();
}

#[cfg(windows)]
#[gpui::test]
fn recycle_bin_participates_in_home_search_count_and_keyboard_selection(
    cx: &mut gpui::TestAppContext,
) {
    let window = home_window(cx);
    window
        .update(cx, |view, _, cx| {
            let normal_places = view.home_places().len();
            assert_eq!(view.item_count(), normal_places + 1);
            assert_eq!(view.recycle_bin_index(), Some(normal_places));
            view.query = "ゴミ箱".into();
            assert_eq!(view.item_count(), 1);
            assert_eq!(view.recycle_bin_index(), Some(0));
            view.move_selection(1, cx);
            assert_eq!(view.selected, Some(0));
            assert!(view.is_home());
            view.query = "__no_matching_place__".into();
            assert_eq!(view.item_count(), 0);
            assert_eq!(view.recycle_bin_index(), None);
        })
        .unwrap();
}

#[gpui::test]
fn home_does_not_list_a_directory_and_is_reachable_in_history(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    window
        .update(cx, |view, w, cx| {
            assert!(view.is_home());
            assert!(!view.loading);
            assert!(view._listing.is_none());
            let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            view.query = "filter-must-not-follow-navigation".into();
            view.navigate_to(folder.clone(), w, cx);
            assert_eq!(view.path(), Some(folder.as_path()));
            assert!(view.query.is_empty());
            assert!(view.search.read(cx).content.is_empty());
            view.go_back(&GoBack, w, cx);
            assert!(view.is_home());
            assert!(view._listing.is_none());
            view.go_forward(&GoForward, w, cx);
            assert_eq!(view.path(), Some(folder.as_path()));
        })
        .unwrap();
}

#[gpui::test]
fn address_rejects_files_preserves_draft_and_opens_valid_directories(
    cx: &mut gpui::TestAppContext,
) {
    let window = home_window(cx);
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let file = folder.join("Cargo.toml").display().to_string();
    window
        .update(cx, |view, w, cx| {
            view.edit_address(&EditAddress, w, cx);
            view.address
                .update(cx, |input, cx| input.set_text(file.clone(), true, w, cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| view.submit_address(w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| {
            assert!(view.is_home());
            assert!(view.editing_address);
            assert!(view.address_error.is_some());
            assert_eq!(view.address.read(cx).content.as_ref(), file);
            view.address.update(cx, |input, cx| {
                input.set_text(folder.display().to_string(), true, w, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| view.submit_address(w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.path(), Some(folder.as_path()));
            assert!(!view.editing_address);
            assert!(view.address_error.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn cancelling_pending_address_cannot_navigate_later(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.edit_address(&EditAddress, w, cx);
            view.address.update(cx, |input, cx| {
                input.set_text(env!("CARGO_MANIFEST_DIR").into(), true, w, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| {
            view.submit_address(w, cx);
            view.cancel_address(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| assert!(view.is_home()))
        .unwrap();
}

// Read real entries without creating files: Entry's private sort key must agree
// with its name. These tests replace enumeration with controlled listing batches.
fn workflow_entries(names: &[&str]) -> Vec<Entry> {
    let mut entries = Vec::new();
    list_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &CancelToken::new(),
        |event| {
            if let ListingEvent::Batch(batch) = event {
                entries.extend(batch);
            }
            true
        },
    );
    names
        .iter()
        .map(|name| {
            entries
                .iter()
                .find(|entry| entry.name == *name)
                .unwrap_or_else(|| panic!("missing fixture entry {name}"))
                .clone()
        })
        .collect()
}

fn workflow_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
    // Keep headless layout/paint enabled: real input dispatch needs its focus
    // and hit-test trees. The state-only home fixture intentionally skips them.
    cx.background_executor.allow_parking();
    let window = cx.add_window(|w, cx| {
        let icons = cx.new(ShellIcons::new);
        FolderView::new(Location::Home, ThemeMode::Light, icons, w, cx)
    });
    window
        .update(cx, |view, w, cx| {
            w.activate_window();
            view.history = History::new(Location::Folder(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            ));
            view._drive_task = None;
            view._wsl_task = None;
            view.grouped = false;
            view.pane = false;
            cx.notify();
        })
        .unwrap();
    window
}

fn scroll_window(cx: &mut gpui::TestAppContext, gallery: bool) -> gpui::WindowHandle<FolderView> {
    cx.update(|cx| crate::motion_preferences::set_wheel_scroll_lines_for_test(cx, 3));
    let window = workflow_window(cx);
    window
        .update(cx, |view, _, cx| {
            let template = workflow_entries(&["assets.rs"]).remove(0);
            view.entries = (0..1000)
                .map(|index| {
                    let mut entry = template.clone();
                    entry.path = template
                        .path
                        .parent()
                        .unwrap()
                        .join(format!("fixture-{index}"))
                        .join(&entry.name);
                    entry
                })
                .collect();
            view.gallery = gallery;
            view.rebuild_visible(None);
            cx.notify();
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    window
}

fn dispatch_wheel(
    window: gpui::WindowHandle<FolderView>,
    cx: &mut gpui::TestAppContext,
    delta: gpui::ScrollDelta,
) {
    let bounds = window
        .read_with(cx, |v, _| v.scroll.0.borrow().base_handle.bounds())
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.dispatch_event(
            gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                position: bounds.origin + gpui::point(px(120.), px(100.)),
                delta,
                ..Default::default()
            }),
            cx,
        );
    })
    .unwrap();
}

fn advance_motion_frame(
    window: gpui::WindowHandle<FolderView>,
    cx: &mut gpui::TestAppContext,
    millis: u64,
) -> usize {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(millis));
    cx.update_window(window.into(), |_, w, cx| {
        let callbacks = w.simulate_next_frame(cx);
        w.draw(cx).clear(cx);
        callbacks
    })
    .unwrap()
}

#[gpui::test]
fn wheel_capture_moves_once_on_frames_in_both_virtual_layouts(cx: &mut gpui::TestAppContext) {
    for gallery in [false, true] {
        let window = scroll_window(cx, gallery);
        // Match the existing GPUI conversion at the folder's 13.5px text size
        // and default rounded line-height, not the virtual row's visual height.
        let line_height = px(22.);
        dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
        window
            .read_with(cx, |view, _| {
                assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
            })
            .unwrap();
        assert_eq!(advance_motion_frame(window, cx, 50), 1);
        window
            .read_with(cx, |view, _| {
                let offset = -f32::from(view.scroll.0.borrow().base_handle.offset().y);
                assert!(offset > 0. && offset < f32::from(line_height) * 3.);
            })
            .unwrap();
        advance_motion_frame(window, cx, 50);
        window
            .read_with(cx, |view, _| {
                let offset = -f32::from(view.scroll.0.borrow().base_handle.offset().y);
                assert!(
                    (offset - f32::from(line_height) * 3.).abs() < 0.1,
                    "offset {offset}"
                );
            })
            .unwrap();
        assert_eq!(
            advance_motion_frame(window, cx, 50),
            0,
            "idle must have no custom callbacks"
        );
    }
}

#[gpui::test]
fn precise_input_discards_pending_target_and_moves_from_painted_position(
    cx: &mut gpui::TestAppContext,
) {
    let window = scroll_window(cx, false);
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    dispatch_wheel(
        window,
        cx,
        gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-11.))),
    );
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(-11.))
        })
        .unwrap();
    advance_motion_frame(window, cx, 200);
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(-11.))
        })
        .unwrap();
}

#[gpui::test]
fn click_between_callback_and_paint_uses_the_visible_row(cx: &mut gpui::TestAppContext) {
    for elapsed in [50, 100] {
        let window = scroll_window(cx, false);
        let (position, expected) = window
            .read_with(cx, |view, _| {
                (
                    view.scroll.0.borrow().base_handle.bounds().origin
                        + gpui::point(px(120.), px(14.)),
                    view.entries[view.visible[0]].path.clone(),
                )
            })
            .unwrap();
        dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(elapsed));
        cx.update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            // Deliberately no draw here: the input must use the prior hitbox tree.
            w.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        window
            .read_with(cx, |view, _| {
                assert_eq!(
                    view.selected_entry().map(|entry| &entry.path),
                    Some(&expected)
                );
            })
            .unwrap();
        advance_motion_frame(window, cx, 200);
        window
            .read_with(cx, |view, _| {
                assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
            })
            .unwrap();
    }
}

#[gpui::test]
fn motion_frame_rebuilds_visible_row_hitboxes(cx: &mut gpui::TestAppContext) {
    let window = scroll_window(cx, false);
    let (position, expected) = window
        .read_with(cx, |view, _| {
            (
                view.scroll.0.borrow().base_handle.bounds().origin + gpui::point(px(120.), px(14.)),
                view.entries[view.visible[1]].path.clone(),
            )
        })
        .unwrap();
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    advance_motion_frame(window, cx, 50); //33px: the next28px row is now at this point.
    cx.update_window(window.into(), |_, w, cx| {
        w.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    })
    .unwrap();
    window
        .read_with(cx, |view, _| {
            assert_eq!(
                view.selected_entry().map(|entry| &entry.path),
                Some(&expected)
            )
        })
        .unwrap();
}

#[gpui::test]
fn cancelled_hidden_or_rebuilt_folder_drains_callbacks_without_moving(
    cx: &mut gpui::TestAppContext,
) {
    for rebuild in [false, true] {
        let window = scroll_window(cx, false);
        dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
        // Multiple input requests still own just one pending callback.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(30));
        dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
        window
            .update(cx, |view, _, _| {
                if rebuild {
                    view.rebuild_visible(None);
                } else {
                    view.cancel_scroll_motion();
                }
            })
            .unwrap();
        assert_eq!(advance_motion_frame(window, cx, 200), 1);
        window
            .read_with(cx, |view, _| {
                assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
            })
            .unwrap();
        assert_eq!(advance_motion_frame(window, cx, 200), 0);
    }
}

#[gpui::test]
fn keyboard_jump_cancels_wheel_and_reduce_motion_keeps_input_direct(cx: &mut gpui::TestAppContext) {
    let window = scroll_window(cx, false);
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    window
        .update(cx, |view, _, cx| view.select(900, cx))
        .unwrap();
    advance_motion_frame(window, cx, 200);
    let jumped = window
        .read_with(cx, |view, _| {
            assert_eq!(view.selected, Some(900));
            let offset = view.scroll.0.borrow().base_handle.offset().y;
            assert!(offset < px(-1000.));
            offset
        })
        .unwrap();
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    window
        .read_with(cx, |view, _| {
            assert!(view.scroll.0.borrow().base_handle.offset().y < jumped)
        })
        .unwrap();
    assert_eq!(advance_motion_frame(window, cx, 200), 0);
}

#[gpui::test]
fn sibling_scroll_region_does_not_start_folder_motion(cx: &mut gpui::TestAppContext) {
    let window = scroll_window(cx, false);
    cx.update_window(window.into(), |_, w, cx| {
        w.dispatch_event(
            gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                position: gpui::point(px(50.), px(200.)),
                delta: gpui::ScrollDelta::Lines(gpui::point(0., -3.)),
                ..Default::default()
            }),
            cx,
        );
    })
    .unwrap();
    assert_eq!(advance_motion_frame(window, cx, 100), 0);
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
        })
        .unwrap();
}

#[gpui::test]
fn hidden_window_does_not_resume_elapsed_motion_after_becoming_visible(
    cx: &mut gpui::TestAppContext,
) {
    let window = scroll_window(cx, false);
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    cx.simulate_window_visibility_change(window.into(), gpui::WindowVisibility::Hidden);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.simulate_window_visibility_change(window.into(), gpui::WindowVisibility::Visible);
    advance_motion_frame(window, cx, 100);
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
        })
        .unwrap();
    assert_eq!(advance_motion_frame(window, cx, 100), 0);
}

#[gpui::test]
fn keyboard_preview_stops_old_motion_but_allows_new_source_scroll(cx: &mut gpui::TestAppContext) {
    let window = scroll_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    window
        .update(cx, |view, w, cx| {
            view.toggle_preview(&QuickPreview, w, cx);
            assert!(view.preview_open);
        })
        .unwrap();
    advance_motion_frame(window, cx, 100);
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(0.))
        })
        .unwrap();
    advance_motion_frame(window, cx, 100);
    window
        .read_with(cx, |view, _| {
            assert!(view.scroll.0.borrow().base_handle.offset().y < px(0.))
        })
        .unwrap();
}

fn selected_file_names(view: &FolderView) -> Vec<String> {
    view.selected_paths()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn shift_click() -> MouseDownEvent {
    MouseDownEvent {
        button: MouseButton::Left,
        modifiers: gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        click_count: 1,
        ..Default::default()
    }
}

#[gpui::test]
fn restored_pending_selection_is_available_to_file_commands(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, _, cx| {
            view.pending_select = Some("folder.rs".into());
            view.on_listing_event(ListingEvent::Batch(workflow_entries(&["explorer.rs"])), cx);
            assert!(view.selected_paths().is_empty());
            assert_eq!(view.pending_select.as_deref(), Some("folder.rs"));

            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&["folder.rs", "assets.rs"])),
                cx,
            );
            assert_eq!(view.selected_entry().unwrap().name, "folder.rs");
            assert_eq!(selected_file_names(view), ["folder.rs"]);
            assert_eq!(
                view.selection_anchor.as_deref(),
                Some(view.selected_entry().unwrap().path.as_path())
            );
            assert!(view.pending_select.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn shift_selection_keeps_its_path_anchor_after_sorting(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&[
                    "assets.rs",
                    "explorer.rs",
                    "folder.rs",
                    "main.rs",
                    "search.rs",
                ])),
                cx,
            );
            view.select(3, cx); // main.rs
            view.set_sort(SortKey::Name, cx); // descending; main.rs moves to index 1
            assert_eq!(view.selected_entry().unwrap().name, "main.rs");
            view.pointer_select(3, &shift_click(), w, cx); // explorer.rs
            assert_eq!(
                selected_file_names(view),
                ["main.rs", "folder.rs", "explorer.rs"]
            );
        })
        .unwrap();
}

#[gpui::test]
fn shift_selection_keeps_its_path_anchor_when_earlier_entries_arrive(
    cx: &mut gpui::TestAppContext,
) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&["explorer.rs", "folder.rs", "main.rs"])),
                cx,
            );
            view.select(2, cx); // main.rs
            view.on_listing_event(ListingEvent::Batch(workflow_entries(&["assets.rs"])), cx);
            assert_eq!(view.selected_entry().unwrap().name, "main.rs");
            view.pointer_select(2, &shift_click(), w, cx); // folder.rs
            assert_eq!(selected_file_names(view), ["folder.rs", "main.rs"]);
        })
        .unwrap();
}

#[gpui::test]
fn shift_selection_uses_the_visible_path_anchor_after_filtering(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&[
                    "assets.rs",
                    "explorer.rs",
                    "folder.rs",
                    "main.rs",
                    "search.rs",
                ])),
                cx,
            );
            view.select(2, cx); // folder.rs
            let keep = view.selected_entry().unwrap().path.clone();
            view.query = "er.rs".into(); // explorer.rs, folder.rs
            view.rebuild_visible(Some(&keep));
            view.pointer_select(0, &shift_click(), w, cx);
            assert_eq!(selected_file_names(view), ["explorer.rs", "folder.rs"]);
        })
        .unwrap();
}

#[gpui::test]
fn gallery_vertical_navigation_moves_rows_but_preview_moves_one_file(
    cx: &mut gpui::TestAppContext,
) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&[
                    "assets.rs",
                    "explorer.rs",
                    "folder.rs",
                    "main.rs",
                    "media.rs",
                    "search.rs",
                    "settings.rs",
                    "shell_icons.rs",
                ])),
                cx,
            );
            view.gallery = true;
            view.gallery_columns = 3;
            view.rebuild_display_rows();
            view.select(1, cx);
            view.select_next(&SelectNext, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "media.rs");
            view.select_prev(&SelectPrev, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "explorer.rs");

            // Set only the preview state: never invoke a native Quick Look window.
            view.preview_open = true;
            view.select_next(&SelectNext, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "folder.rs");
            view.select_prev(&SelectPrev, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "explorer.rs");
            view.close_preview(cx);

            view.select(7, cx);
            view.select_next(&SelectNext, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "shell_icons.rs");
            view.select(0, cx);
            view.select_prev(&SelectPrev, w, cx);
            assert_eq!(view.selected_entry().unwrap().name, "assets.rs");
        })
        .unwrap();
}

#[gpui::test]
fn gallery_left_and_right_actions_select_adjacent_files(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&["assets.rs", "explorer.rs", "folder.rs"])),
                cx,
            );
            view.set_gallery(true, cx);
            view.select(1, cx);
            w.focus(&view.focus, cx);
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    cx.dispatch_action(window.into(), SelectRight);
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["folder.rs"]);
        })
        .unwrap();
    cx.dispatch_action(window.into(), SelectLeft);
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["explorer.rs"]);
        })
        .unwrap();
}

#[gpui::test]
fn empty_folder_background_right_click_opens_the_context_menu(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_mouse_down(
        gpui::point(px(320.), px(180.)),
        MouseButton::Right,
        gpui::Modifiers::none(),
    );
    window
        .read_with(cx, |view, _| {
            assert!(
                view.context_menu.is_some(),
                "empty folders still need Paste/Refresh"
            );
            assert!(view.selected_paths().is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn context_menu_keeps_its_target_when_listing_inserts_an_earlier_folder(
    cx: &mut gpui::TestAppContext,
) {
    let window = workflow_window(cx);
    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = opened.clone();
    let _subscription = window
        .update(cx, |view, w, cx| {
            view.on_listing_event(ListingEvent::Batch(workflow_entries(&["folder"])), cx);
            view.open_context_menu(
                Some(0),
                &MouseDownEvent {
                    button: MouseButton::Right,
                    position: gpui::point(px(320.), px(180.)),
                    ..Default::default()
                },
                w,
                cx,
            );
            view.on_listing_event(ListingEvent::Batch(workflow_entries(&["explorer"])), cx);
            cx.subscribe(&cx.entity(), move |_, _, event: &FolderEvent, _| {
                let FolderEvent::OpenInTab(path) = event;
                captured.borrow_mut().push(path.clone());
            })
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    // The second command for a directory is Open in New Tab; it emits an event
    // rather than launching a native app or actually opening another window.
    cx.simulate_keystrokes(window.into(), "down enter");
    assert_eq!(
        *opened.borrow(),
        [Path::new(env!("CARGO_MANIFEST_DIR")).join("src/folder")]
    );
}

#[gpui::test]
fn blank_drag_selects_a_range_without_starting_file_drag(cx: &mut gpui::TestAppContext) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, _, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&["assets.rs", "explorer.rs", "folder.rs"])),
                cx,
            )
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let origin = window
        .read_with(cx, |view, _| {
            view.scroll.0.borrow().base_handle.bounds().origin
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["explorer.rs", "folder.rs"])
        })
        .unwrap();
}

fn marquee_window(
    cx: &mut gpui::TestAppContext,
    gallery: bool,
) -> (gpui::WindowHandle<FolderView>, gpui::Point<Pixels>) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, _, cx| {
            view.on_listing_event(
                ListingEvent::Batch(workflow_entries(&["assets.rs", "explorer.rs", "folder.rs"])),
                cx,
            );
            if gallery {
                view.set_gallery(true, cx);
            }
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let origin = window
        .read_with(cx, |view, _| {
            view.scroll.0.borrow().base_handle.bounds().origin
        })
        .unwrap();
    (window, origin)
}

fn highlighted_file_names(view: &FolderView) -> Vec<&str> {
    view.visible
        .iter()
        .enumerate()
        .filter(|(ix, _)| view.item_is_selected(*ix))
        .map(|(_, &index)| view.entries[index].name.as_str())
        .collect()
}

#[gpui::test]
fn marquee_toggle_recomputes_from_baseline_when_the_rectangle_shrinks(
    cx: &mut gpui::TestAppContext,
) {
    // Control on Windows/Linux and Command on macOS share the same gesture.
    for modifiers in [
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        gpui::Modifiers {
            platform: true,
            ..Default::default()
        },
    ] {
        let (window, origin) = marquee_window(cx, false);
        window.update(cx, |view, _, cx| view.select(1, cx)).unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let start = origin + gpui::point(px(120.), px(140.));
        let expanded = origin + gpui::point(px(220.), px(32.));
        let shrunk = origin + gpui::point(px(220.), px(62.));
        visual.simulate_mouse_down(start, MouseButton::Left, modifiers);
        visual.simulate_mouse_move(expanded, MouseButton::Left, modifiers);
        window
            .read_with(cx, |view, _| {
                assert_eq!(highlighted_file_names(view), ["folder.rs"]);
                assert_eq!(
                    selected_file_names(view),
                    ["explorer.rs"],
                    "paths stay unchanged until release"
                );
            })
            .unwrap();
        visual.simulate_mouse_move(shrunk, MouseButton::Left, modifiers);
        window
            .read_with(cx, |view, _| {
                assert_eq!(highlighted_file_names(view), ["explorer.rs", "folder.rs"]);
            })
            .unwrap();
        visual.simulate_mouse_up(shrunk, MouseButton::Left, modifiers);
        window
            .read_with(cx, |view, _| {
                assert_eq!(selected_file_names(view), ["explorer.rs", "folder.rs"]);
                assert!(!view.marquee_active());
            })
            .unwrap();
    }
}

#[gpui::test]
fn marquee_shift_adds_to_selection_and_modified_blank_clicks_preserve_it(
    cx: &mut gpui::TestAppContext,
) {
    let (window, origin) = marquee_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let blank = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };
    visual.simulate_mouse_down(blank, MouseButton::Left, shift);
    visual.simulate_mouse_move(end, MouseButton::Left, shift);
    visual.simulate_mouse_up(end, MouseButton::Left, shift);
    window
        .read_with(cx, |view, _| {
            assert_eq!(
                selected_file_names(view),
                ["assets.rs", "explorer.rs", "folder.rs"]
            );
        })
        .unwrap();
    for modifiers in [
        shift,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    ] {
        visual.simulate_mouse_down(blank, MouseButton::Left, modifiers);
        visual.simulate_mouse_up(blank, MouseButton::Left, modifiers);
        window
            .read_with(cx, |view, _| assert_eq!(view.multi_selection.len(), 3))
            .unwrap();
    }
    visual.simulate_mouse_down(blank, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(blank, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert!(view.multi_selection.is_empty());
            assert!(view.selected.is_none());
            assert!(view.selection_anchor.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn escape_cancels_marquee_and_restores_selection_and_anchor(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert!(view.marquee_active());
            assert_eq!(highlighted_file_names(view), ["explorer.rs", "folder.rs"]);
        })
        .unwrap();
    cx.dispatch_action(window.into(), ClearSearch);
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert!(!view.marquee_active());
            assert_eq!(selected_file_names(view), ["assets.rs"]);
            assert_eq!(
                view.selection_anchor.as_ref().unwrap().file_name().unwrap(),
                "assets.rs"
            );
        })
        .unwrap();
}

#[gpui::test]
fn rebuilding_rows_cancels_marquee_before_its_indices_can_change(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .update(cx, |view, _, cx| {
            assert!(view.marquee_active());
            view.set_sort(SortKey::Name, cx);
            assert!(!view.marquee_active());
        })
        .unwrap();
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["assets.rs"]);
        })
        .unwrap();
}

#[gpui::test]
fn gallery_blank_strip_selects_only_the_intersected_columns(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, true);
    let (columns, width) = window
        .read_with(cx, |view, _| {
            (
                view.gallery_columns,
                f32::from(view.scroll.0.borrow().base_handle.bounds().size.width),
            )
        })
        .unwrap();
    assert!(columns >= 2, "fixture needs at least two gallery columns");
    let tile_width = (width - 20. - 8. * (columns - 1) as f32) / columns as f32;
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    // The label strip above the tiles is blank even when it has no group label.
    let start = origin + gpui::point(px(12.), px(10.));
    let end = origin + gpui::point(px(10. + tile_width + 8. + tile_width / 2.), px(80.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["assets.rs", "explorer.rs"]);
        })
        .unwrap();
}

#[gpui::test]
fn marquee_scrolls_with_a_stationary_pointer_and_stops_on_release(cx: &mut gpui::TestAppContext) {
    assert_marquee_edge_scroll(cx, false);
}

#[gpui::test]
fn marquee_scrolls_when_pointer_jumps_outside_the_window(cx: &mut gpui::TestAppContext) {
    assert_marquee_edge_scroll(cx, true);
}

#[gpui::test]
fn marquee_after_cancelled_wheel_starts_with_a_fresh_clock(cx: &mut gpui::TestAppContext) {
    let window = scroll_window(cx, false);
    let bounds = window
        .read_with(cx, |view, _| view.scroll.0.borrow().base_handle.bounds())
        .unwrap();
    dispatch_wheel(window, cx, gpui::ScrollDelta::Lines(gpui::point(0., -3.)));
    let row = bounds.origin + gpui::point(px(120.), px(14.));
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_mouse_down(row, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(row, MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(advance_motion_frame(window, cx, 32), 1);
    assert_eq!(advance_motion_frame(window, cx, 1000), 0);

    let start = bounds.origin + gpui::point(px(4.), px(100.));
    let end = gpui::point(bounds.origin.x + px(200.), bounds.bottom() - px(2.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(advance_motion_frame(window, cx, 0), 1);
    window
        .read_with(cx, |view, _| {
            assert_eq!(
                view.scroll.0.borrow().base_handle.offset().y,
                px(0.),
                "idle time must not become marquee distance"
            );
        })
        .unwrap();
    advance_motion_frame(window, cx, 16);
    window
        .read_with(cx, |view, _| {
            assert!(view.scroll.0.borrow().base_handle.offset().y < px(0.))
        })
        .unwrap();
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
}

fn assert_marquee_edge_scroll(cx: &mut gpui::TestAppContext, outside: bool) {
    let window = workflow_window(cx);
    window
        .update(cx, |view, _, cx| {
            let template = workflow_entries(&["assets.rs"]).remove(0);
            // Distinct paths with the same basename keep Entry's private name sort
            // key valid while supplying more rows than fit in the viewport.
            view.entries = (0..100)
                .map(|index| {
                    let mut entry = template.clone();
                    entry.path = template
                        .path
                        .parent()
                        .unwrap()
                        .join(format!("fixture-{index}"))
                        .join(&entry.name);
                    entry
                })
                .collect();
            view.rebuild_visible(None);
            cx.notify();
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let bounds = window
        .read_with(cx, |view, _| view.scroll.0.borrow().base_handle.bounds())
        .unwrap();
    let start = bounds.origin + gpui::point(px(4.), px(100.));
    let end = gpui::point(
        bounds.origin.x + px(200.),
        bounds.bottom() + px(if outside { 100. } else { -2. }),
    );
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.run_until_parked();
    // A native callback can arrive at the same clock instant as pointer input.
    // It must keep edge demand alive for the next nonzero elapsed frame.
    assert_eq!(advance_motion_frame(window, cx, 0), 1);
    let before = window
        .read_with(cx, |view, _| view.scroll.0.borrow().base_handle.offset().y)
        .unwrap();
    advance_motion_frame(window, cx, 32);
    let first = window
        .read_with(cx, |view, _| view.scroll.0.borrow().base_handle.offset().y)
        .unwrap();
    assert!(
        first < before,
        "edge scrolling must not require another mouse event"
    );
    advance_motion_frame(window, cx, 32);
    let second = window
        .read_with(cx, |view, _| view.scroll.0.borrow().base_handle.offset().y)
        .unwrap();
    assert!(
        second < first,
        "native frames must keep scrolling while the pointer is stationary"
    );
    let max = window
        .update(cx, |view, _, _| {
            let handle = view.scroll.0.borrow().base_handle.clone();
            let max = f32::from(handle.max_offset().y).abs();
            handle.set_offset(gpui::point(handle.offset().x, px(-(max - 0.005))));
            max
        })
        .unwrap();
    advance_motion_frame(window, cx, 32);
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, px(-max))
        })
        .unwrap();
    assert_eq!(
        advance_motion_frame(window, cx, 32),
        0,
        "an exact boundary must end frame demand"
    );
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    let released = window
        .read_with(cx, |view, _| {
            assert!(!view.marquee_active());
            assert!(!view.multi_selection.is_empty());
            view.scroll.0.borrow().base_handle.offset().y
        })
        .unwrap();
    advance_motion_frame(window, cx, 128);
    window
        .read_with(cx, |view, _| {
            assert_eq!(view.scroll.0.borrow().base_handle.offset().y, released)
        })
        .unwrap();
}

#[gpui::test]
fn file_rows_still_start_file_drags_instead_of_marquees(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, false);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(14.));
    let end = origin + gpui::point(px(220.), px(45.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, cx| {
            assert!(!view.marquee_active());
            assert_eq!(selected_file_names(view), ["assets.rs"]);
            assert!(
                cx.has_active_drag(),
                "file row drag handlers must survive the blank-space wrapper"
            );
        })
        .unwrap();
    // End the synthetic drag without dropping files onto the current folder.
    cx.update_window(window.into(), |_, w, cx| {
        cx.stop_active_drag(w);
    })
    .unwrap();
}

#[gpui::test]
fn grouped_list_header_is_a_blank_start_area_and_never_a_selected_item(
    cx: &mut gpui::TestAppContext,
) {
    let (window, _) = marquee_window(cx, false);
    window
        .update(cx, |view, _, cx| {
            let now = std::time::SystemTime::now();
            for entry in &mut view.entries {
                entry.modified = Some(now);
            }
            view.grouped = true;
            view.rebuild_display_rows();
            cx.notify();
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let origin = window
        .read_with(cx, |view, _| {
            view.scroll.0.borrow().base_handle.bounds().origin
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(12.));
    let end = origin + gpui::point(px(220.), px(70.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert_eq!(selected_file_names(view), ["assets.rs", "explorer.rs"])
        })
        .unwrap();
}

fn marquee_keeps_baseline_after_selection_action(
    cx: &mut gpui::TestAppContext,
    action: impl gpui::Action,
) {
    let (window, origin) = marquee_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| assert!(view.marquee_active()))
        .unwrap();
    cx.dispatch_action(window.into(), action);
    cx.dispatch_action(window.into(), ClearSearch);
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| {
            assert!(!view.marquee_active());
            assert_eq!(
                selected_file_names(view),
                ["assets.rs"],
                "selection keys must not mutate the saved drag baseline"
            );
            assert_eq!(
                view.selection_anchor.as_ref().unwrap().file_name().unwrap(),
                "assets.rs"
            );
        })
        .unwrap();
}

#[gpui::test]
fn marquee_arrow_then_escape_preserves_starting_selection(cx: &mut gpui::TestAppContext) {
    marquee_keeps_baseline_after_selection_action(cx, SelectNext);
}

#[gpui::test]
fn marquee_select_all_then_escape_preserves_starting_selection(cx: &mut gpui::TestAppContext) {
    marquee_keeps_baseline_after_selection_action(cx, crate::actions::SelectAllFiles);
}

#[gpui::test]
fn marquee_height_resize_cancels_without_another_pointer_event(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, false);
    window.update(cx, |view, _, cx| view.select(0, cx)).unwrap();
    let old_size = cx
        .update_window(window.into(), |_, w, _| w.viewport_size())
        .unwrap();
    let old_height = window
        .read_with(cx, |view, _| {
            view.scroll.0.borrow().base_handle.bounds().size.height
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let start = origin + gpui::point(px(120.), px(140.));
    let end = origin + gpui::point(px(220.), px(32.));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    window
        .read_with(cx, |view, _| assert!(view.marquee_active()))
        .unwrap();
    visual.simulate_resize(gpui::size(old_size.width, old_size.height - px(80.)));
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(cx, |view, _| {
            assert!(
                view.scroll.0.borrow().base_handle.bounds().size.height < old_height,
                "the fixture must actually resize the selection viewport"
            );
            assert!(
                !view.marquee_active(),
                "height-only layout changes must cancel without a pointer move"
            );
            assert_eq!(selected_file_names(view), ["assets.rs"]);
            assert_eq!(
                view.selection_anchor.as_ref().unwrap().file_name().unwrap(),
                "assets.rs"
            );
        })
        .unwrap();
}

#[gpui::test]
fn marquee_receives_outside_motion_before_its_first_repaint(cx: &mut gpui::TestAppContext) {
    let (window, origin) = marquee_window(cx, false);
    let view = window.root(cx).unwrap();
    let start = origin + gpui::point(px(120.), px(140.));
    cx.update_window(window.into(), |_, w, cx| {
        // Real platforms may deliver both events before the next animation
        // frame. VisualTestContext's per-event draw would conceal this race.
        w.dispatch_event(
            gpui::PlatformInput::MouseDown(MouseDownEvent {
                position: start,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
        assert!(
            view.read(cx).marquee_in_progress(),
            "blank mouse-down must begin the gesture"
        );
        w.dispatch_event(
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position: gpui::point(px(-10.), px(-10.)),
                pressed_button: Some(MouseButton::Left),
                modifiers: gpui::Modifiers::none(),
            }),
            cx,
        );
        assert!(
            view.read(cx).marquee_active(),
            "outside motion must reach a new gesture before its first repaint"
        );
        assert!(!cx.has_active_drag());
    })
    .unwrap();
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn standalone_preview_tracks_source_selection_and_closes_on_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let source = workflow_window(cx);
    let preview = source
        .update(cx, |view, window, cx| {
            // Directories avoid launching nondeterministic decoder work in this lifecycle test.
            view.entries = workflow_entries(&["folder", "media"]);
            view.rebuild_visible(None);
            view.select(0, cx);
            view.toggle_preview(&QuickPreview, window, cx);
            view.preview_window.expect("separate preview window")
        })
        .unwrap();
    assert_ne!(source.window_id(), preview.window_id());
    cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("down", SelectNext, Some("Explorer"))]));
    cx.update_window(source.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    cx.update_window(preview.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    cx.simulate_keystrokes(preview.into(), "down");
    cx.run_until_parked();
    source
        .read_with(cx, |view, _| assert_eq!(view.selected, Some(1)))
        .unwrap();
    preview
        .read_with(cx, |preview, _| {
            assert_eq!(preview.entry_name_for_test(), "media");
        })
        .unwrap();
    source
        .update(cx, |view, _, cx| view.load(Location::Home, None, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(preview.read_with(cx, |_, _| ()).is_err());
    source
        .read_with(cx, |view, _| assert!(!view.preview_open))
        .unwrap();
}
