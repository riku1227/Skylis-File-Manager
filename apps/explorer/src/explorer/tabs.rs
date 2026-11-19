use super::*;
#[cfg(test)]
use gpui::size;
use gpui::{Bounds, Pixels, Point, WindowHandle, point};

#[derive(Clone)]
pub(super) struct TabDrag {
    pub id: gpui::EntityId,
    pub label: gpui::SharedString,
    pub mode: ThemeMode,
}

impl Render for TabDrag {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::resolve(self.mode, window.appearance());
        div()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.c("tab.active.border"))
            .bg(theme.c("surface.canvas"))
            .text_color(theme.c("text.primary"))
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

/// Insertion slots refer to the list before removal, including the slot after its last tab.
fn reordered_index(from: usize, slot: usize, len: usize) -> usize {
    slot.min(len)
        .saturating_sub(usize::from(slot > from))
        .min(len.saturating_sub(1))
}

fn screen_scale(window: &Window) -> f32 {
    if cfg!(windows) {
        window.scale_factor()
    } else {
        1.
    }
}

fn screen_position(position: Point<Pixels>, window: &Window) -> Point<Pixels> {
    (window.bounds().origin + position) * screen_scale(window)
}

impl Explorer {
    pub(super) fn new_window(&mut self, _: &NewWindow, w: &mut Window, cx: &mut Context<Self>) {
        self.open_window_at(Location::Home, w, cx);
    }

    fn open_window_at(&mut self, location: Location, w: &mut Window, cx: &mut Context<Self>) {
        let icons = self.icons.clone();
        let mode = self.mode;
        let compact = self.compact;
        let bounds = Bounds::new(w.bounds().origin + point(px(32.), px(32.)), w.bounds().size);
        if let Err(error) = cx.open_window(
            crate::window_chrome::window_options(bounds, "Skylis"),
            move |w, cx| {
                cx.new(|cx| {
                    let folder = cx.new(|cx| FolderView::new(location, mode, icons.clone(), w, cx));
                    Self::with_tabs(vec![folder], icons, mode, compact, w, cx)
                })
            },
        ) {
            self.tab_error = Some(format!("新しいウィンドウを開けません: {error}"));
            cx.notify();
        }
    }
    fn tab_drop_slot_at(&self, position: Point<Pixels>, window: &Window) -> Option<usize> {
        let strip = self.tab_strip.get();
        let right = window.viewport_size().width - px(if cfg!(windows) { 138. } else { 0. });
        if position.y >= px(0.)
            && position.y < px(38.)
            && position.x >= strip.origin.x
            && position.x < right
        {
            Some(self.tab_motion.drop_slot(
                position.x - strip.origin.x - self.tab_scroll.offset().x,
                window,
            ))
        } else {
            None
        }
    }
    pub(super) fn update_tab_drop_indicator(
        &mut self,
        position: Point<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = self.tab_drop_slot_at(position, w);
        if self.drop_slot != slot {
            self.drop_slot = slot;
            cx.notify();
        }
        let screen = screen_position(position, w);
        let current = w.window_handle();
        for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
            let Some(handle) = handle.downcast::<Explorer>() else {
                continue;
            };
            let _ = handle.update(cx, |target, window, cx| {
                let local = screen / screen_scale(window) - window.bounds().origin;
                let slot = if target_is_under_pointer(window, screen) {
                    target.tab_drop_slot_at(local, window)
                } else {
                    None
                };
                if target.drop_slot != slot {
                    target.drop_slot = slot;
                    cx.notify();
                }
            });
        }
    }

    pub(super) fn clear_tab_drop_indicators(&mut self, w: &Window, cx: &mut Context<Self>) {
        self.drop_slot = None;
        let current = w.window_handle();
        for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
            if let Some(handle) = handle.downcast::<Explorer>() {
                let _ = handle.update(cx, |target, _, cx| {
                    if target.drop_slot.take().is_some() {
                        cx.notify();
                    }
                });
            }
        }
    }
    pub(super) fn watch_tab(
        &mut self,
        tab: &Entity<FolderView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tab.update(cx, |tab, cx| {
            tab.close_preview(cx);
            tab.pane = self.pane;
            tab.attach_window(window, cx);
        });
        let observer = cx.observe(tab, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(
            tab,
            window,
            |this, _, event: &crate::folder::FolderEvent, w, cx| match event {
                crate::folder::FolderEvent::OpenInTab(location) => {
                    this.open_tab_at(location.clone(), w, cx)
                }
                crate::folder::FolderEvent::OpenInWindow(location) => {
                    this.open_window_at(location.clone(), w, cx)
                }
                crate::folder::FolderEvent::SetPane(enabled) => {
                    this.pane = *enabled;
                    for tab in &this.tabs {
                        tab.update(cx, |tab, cx| {
                            tab.pane = *enabled;
                            cx.notify();
                        });
                    }
                    cx.notify();
                }
            },
        );
        self.tab_subscriptions
            .insert(tab.entity_id(), vec![observer, events]);
    }

    pub(super) fn open_tab_at(
        &mut self,
        location: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = cx.new(|cx| FolderView::new(location, self.mode, self.icons.clone(), window, cx));
        tab.update(cx, |tab, _| tab.compact = self.compact);
        self.watch_tab(&tab, window, cx);
        self.tab_motion
            .open(self.tabs.len(), tab.entity_id(), cx.reduce_motion());
        self.tabs.push(tab);
        self.settings_open = false;
        self.switch(self.tabs.len() - 1, window, cx);
    }

    fn remove_moved_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.remove(index);
        self.tab_motion.remove(tab.entity_id());
        self.tab_subscriptions.remove(&tab.entity_id());
        self.dragging_tab = None;
        self.tab_menu = None;
        if self.tabs.is_empty() {
            window.remove_window();
        } else {
            self.active = after_close(self.active, index, self.tabs.len());
            self.switch(self.active, window, cx);
        }
    }

    fn accept_tab(
        &mut self,
        tab: Entity<FolderView>,
        slot: usize,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active) = self.tabs.get(self.active) {
            active.update(cx, |tab, cx| {
                tab.pause_name_search(cx);
                tab.cancel_scroll_motion();
                tab.close_preview(cx);
                tab.close_display_settings(cx);
            });
        }
        tab.update(cx, |tab, cx| {
            tab.mode = self.mode;
            tab.compact = self.compact;
            cx.notify();
        });
        self.watch_tab(&tab, w, cx);
        let slot = slot.min(self.tabs.len());
        self.tab_motion.insert(slot, tab.entity_id());
        self.tabs.insert(slot, tab);
        self.settings_open = false;
        self.active = slot;
        self.switch(slot, w, cx);
        w.activate_window();
    }

    fn transfer_tab(
        &mut self,
        index: usize,
        destination: WindowHandle<Explorer>,
        slot: usize,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() == 1
            && self
                .activity
                .center
                .read(cx)
                .native_owned_by(w.window_handle().window_id())
        {
            self.tab_error =
                Some("ファイル操作中は、最後のタブを別のウィンドウへ移動できません。".into());
            cx.notify();
            return;
        }
        let tab = self.tabs[index].clone();
        // Keep the source until destination update succeeds: a closed target never loses a tab.
        if destination
            .update(cx, |target, target_window, cx| {
                target.accept_tab(tab, slot, target_window, cx)
            })
            .is_ok()
        {
            self.remove_moved_tab(index, w, cx);
        } else {
            self.tab_error =
                Some("移動先のウィンドウが閉じられました。タブは保持しています。".into());
            cx.notify();
        }
    }

    fn detach_at(
        &mut self,
        index: usize,
        origin: Point<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() == 1
            && self
                .activity
                .center
                .read(cx)
                .native_owned_by(w.window_handle().window_id())
        {
            self.tab_error =
                Some("ファイル操作中は、最後のタブを別のウィンドウへ移動できません。".into());
            cx.notify();
            return;
        }
        let tab = self.tabs[index].clone();
        let icons = self.icons.clone();
        let mode = self.mode;
        let compact = self.compact;
        let bounds = Bounds::new(origin, w.bounds().size);
        let result = cx.open_window(
            crate::window_chrome::window_options(bounds, "Skylis"),
            move |w, cx| cx.new(|cx| Explorer::with_tabs(vec![tab], icons, mode, compact, w, cx)),
        );
        match result {
            Ok(_) => self.remove_moved_tab(index, w, cx),
            Err(error) => {
                self.tab_error = Some(format!("新しいウィンドウを開けません: {error}"));
                cx.notify();
            }
        }
    }

    pub(super) fn detach_tab(&mut self, _: &DetachTab, w: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        if self.tabs.len() > 1 {
            self.detach_at(
                self.active,
                w.bounds().origin + point(px(32.), px(32.)),
                w,
                cx,
            );
        }
    }
    fn detach_menu_tab(&mut self, index: usize, w: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        if self.tabs.len() > 1 && index < self.tabs.len() {
            self.detach_at(index, w.bounds().origin + point(px(32.), px(32.)), w, cx);
        }
    }

    pub(super) fn merge_windows(
        &mut self,
        _: &MergeWindowTabs,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.windows().iter().any(|handle| {
            *handle != w.window_handle()
                && self
                    .activity
                    .center
                    .read(cx)
                    .native_owned_by(handle.window_id())
        }) {
            self.tab_error =
                Some("ファイル操作が完了してからウィンドウをまとめてください。".into());
            cx.notify();
            return;
        }
        self.tab_menu = None;
        let current = w.window_handle();
        for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
            let Some(handle) = handle.downcast::<Explorer>() else {
                continue;
            };
            if let Ok(tabs) = handle.update(cx, |other, window, _| {
                other.tab_subscriptions.clear();
                let tabs = std::mem::take(&mut other.tabs);
                window.remove_window();
                tabs
            }) {
                for tab in tabs {
                    self.accept_tab(tab, self.tabs.len(), w, cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn finish_tab_drag(
        &mut self,
        position: Point<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_tab_drag_with_hit_test(position, w, cx, target_is_under_pointer);
    }

    fn finish_tab_drag_with_hit_test(
        &mut self,
        position: Point<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
        is_under_pointer: impl Fn(&Window, Point<Pixels>) -> bool,
    ) {
        let Some(id) = self.dragging_tab.take() else {
            return;
        };
        self.clear_tab_drop_indicators(w, cx);
        let Some(index) = self.tabs.iter().position(|tab| tab.entity_id() == id) else {
            return;
        };
        cx.stop_active_drag(w);
        let screen = screen_position(position, w);
        // Local coordinates alone do not identify the drop target: another
        // Skylis window can cover this window's strip or content area.
        if is_under_pointer(w, screen)
            && let Some(slot) = self.tab_drop_slot_at(position, w)
        {
            // Stop the outgoing tab before its index changes, including when a
            // search/address child owns focus and the folder blur hook won't run.
            if self.tabs[self.active].entity_id() != self.tabs[index].entity_id() {
                self.tabs[self.active].update(cx, |tab, cx| tab.pause_name_search(cx));
            }
            self.tabs[self.active].update(cx, |tab, _| tab.cancel_scroll_motion());
            let to = reordered_index(index, slot, self.tabs.len());
            let tab = self.tabs.remove(index);
            self.tab_motion.remove(id);
            self.tab_motion.insert(to, id);
            self.tabs.insert(to, tab);
            self.active = to;
            self.switch(to, w, cx);
            return;
        }
        let current = w.window_handle();
        for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
            let Some(handle) = handle.downcast::<Explorer>() else {
                continue;
            };
            let target = handle
                .update(cx, |target, window, _| {
                    let local = screen / screen_scale(window) - window.bounds().origin;
                    if is_under_pointer(window, screen) {
                        target.tab_drop_slot_at(local, window)
                    } else {
                        None
                    }
                })
                .ok()
                .flatten();
            if let Some(slot) = target {
                self.transfer_tab(index, handle, slot, w, cx);
                return;
            }
        }
        // Check overlapping destination strips first. A drop elsewhere inside
        // the source still cancels rather than creating a window over a file row.
        if Bounds::new(Point::default(), w.viewport_size()).contains(&position) {
            cx.notify();
            return;
        }
        let origin = screen / screen_scale(w) - point(px(90.), px(18.));
        if self.tabs.len() == 1 && move_existing_window(w, origin, cx) {
            cx.notify();
        } else {
            self.detach_at(index, origin, w, cx);
        }
    }

    pub(super) fn render_tab_menu(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some((id, position)) = self.tab_menu else {
            return div().into_any_element();
        };
        let position = point(
            position
                .x
                .min((window.viewport_size().width - px(240.)).max(px(0.))),
            position.y,
        );
        let mut menu = div()
            .id("tab-menu")
            .absolute()
            .left(position.x)
            .top(position.y)
            .w(px(240.))
            .p(px(5.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.c("border.subtle"))
            .bg(theme.c("surface.canvas"))
            .shadow_md()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tab_menu = None;
                cx.notify();
            }));
        for (name, label, enabled) in [
            ("duplicate-tab", "タブを複製", true),
            (
                "reopen-closed-tab",
                "閉じたタブを開き直す",
                !self.closed_tabs.is_empty(),
            ),
            ("detach-tab", "別のウィンドウに移動", self.tabs.len() > 1),
            (
                "merge-tabs",
                "ほかのウィンドウのタブをまとめる",
                cx.windows().len() > 1,
            ),
        ] {
            menu = menu.child(
                div()
                    .id(name)
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(4.))
                    .when(!enabled, |d| d.opacity(0.4))
                    .when(enabled, |d| {
                        d.cursor_pointer().hover(|s| s.bg(theme.c("state.hover")))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, w, cx| {
                        cx.stop_propagation();
                        if !enabled {
                            return;
                        }
                        if name == "reopen-closed-tab" {
                            this.reopen_closed_tab(w, cx);
                        } else if name == "duplicate-tab" {
                            if let Some(index) =
                                this.tabs.iter().position(|tab| tab.entity_id() == id)
                            {
                                this.duplicate_tab_at(index, w, cx);
                            }
                        } else if name == "merge-tabs" {
                            this.merge_windows(&MergeWindowTabs, w, cx);
                        } else if let Some(index) =
                            this.tabs.iter().position(|tab| tab.entity_id() == id)
                        {
                            this.detach_menu_tab(index, w, cx);
                        }
                    })),
            );
        }
        menu.into_any_element()
    }
}

#[cfg(windows)]
fn target_is_under_pointer(window: &Window, screen: Point<Pixels>) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    unsafe {
        let target = windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint(
            windows_sys::Win32::Foundation::POINT {
                x: f32::from(screen.x) as i32,
                y: f32::from(screen.y) as i32,
            },
        );
        windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
            target,
            windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
        ) == handle.hwnd.get() as _
    }
}
#[cfg(not(windows))]
fn target_is_under_pointer(_: &Window, _: Point<Pixels>) -> bool {
    true
}

#[cfg(windows)]
fn move_existing_window(window: &Window, origin: Point<Pixels>, cx: &App) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get();
    let x = (f32::from(origin.x) * window.scale_factor()) as i32;
    let y = (f32::from(origin.y) * window.scale_factor()) as i32;
    let maximized = window.is_maximized();
    // Native move/resize callbacks can re-enter GPUI. Mirror the backend's deferred
    // window resize: run only after the current entity/window update has returned.
    cx.foreground_executor()
        .spawn(async move {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            unsafe {
                if IsWindow(hwnd as _) == 0 {
                    return;
                }
                if maximized {
                    ShowWindow(hwnd as _, SW_RESTORE);
                }
                SetWindowPos(
                    hwnd as _,
                    std::ptr::null_mut(),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER,
                );
            }
        })
        .detach();
    true
}
#[cfg(not(windows))]
fn move_existing_window(_: &Window, _: Point<Pixels>, _: &App) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reorder_slots_account_for_removal_and_end_drop() {
        assert_eq!(reordered_index(0, 3, 3), 2);
        assert_eq!(reordered_index(2, 0, 3), 0);
        assert_eq!(reordered_index(1, 2, 3), 1);
        assert_eq!(reordered_index(0, 1, 1), 0);
    }
    fn test_window(cx: &mut gpui::TestAppContext) -> WindowHandle<Explorer> {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        cx.update(|cx| {
            if !cx.has_global::<crate::settings::SharedPreferences>() {
                let preferences = cx.new(|cx| {
                    crate::settings::Preferences::new(
                        (
                            Default::default(),
                            Some("memory-only test preferences".into()),
                        ),
                        cx,
                    )
                });
                cx.set_global(crate::settings::SharedPreferences(preferences));
            }
        });
        cx.add_window(|w, cx| {
            let icons = cx.new(ShellIcons::new);
            let tab = cx
                .new(|cx| FolderView::new(Location::Home, ThemeMode::Light, icons.clone(), w, cx));
            Explorer::with_tabs(vec![tab], icons, ThemeMode::Light, false, w, cx)
        })
    }

    #[gpui::test]
    fn accepting_tab_dismisses_popup_in_previous_active_tab(cx: &mut gpui::TestAppContext) {
        let source = test_window(cx);
        let destination = test_window(cx);
        let previous = destination
            .update(cx, |view, w, cx| {
                let tab = view.tabs[0].clone();
                tab.update(cx, |tab, cx| tab.open_display_settings(w, cx));
                tab
            })
            .unwrap();
        source
            .update(cx, |view, w, cx| {
                view.transfer_tab(0, destination, 1, w, cx)
            })
            .unwrap();
        destination
            .update(cx, |view, w, cx| {
                previous.update(cx, |tab, cx| {
                    assert!(
                        !tab.close_display_settings(cx),
                        "incoming tab must dismiss previous popup before switching back"
                    )
                });
                view.switch(0, w, cx);
            })
            .unwrap();
    }

    #[gpui::test]
    fn pane_is_window_local_and_transferred_tabs_adopt_destination(cx: &mut gpui::TestAppContext) {
        let first = test_window(cx);
        let second = test_window(cx);
        first
            .update(cx, |view, w, cx| {
                view.new_tab(&NewTab, w, cx);
                view.tabs[0].update(cx, |_, cx| {
                    cx.emit(crate::folder::FolderEvent::SetPane(false))
                });
            })
            .unwrap();
        cx.run_until_parked();
        first
            .read_with(cx, |view, cx| {
                assert!(view.tabs.iter().all(|tab| !tab.read(cx).pane))
            })
            .unwrap();
        second
            .read_with(cx, |view, cx| {
                assert!(view.pane);
                assert!(view.tabs[0].read(cx).pane);
            })
            .unwrap();
        first
            .update(cx, |view, w, cx| view.transfer_tab(0, second, 1, w, cx))
            .unwrap();
        cx.run_until_parked();
        second
            .read_with(cx, |view, cx| {
                assert_eq!(view.tabs.len(), 2);
                assert!(view.tabs.iter().all(|tab| tab.read(cx).pane));
            })
            .unwrap();
        first
            .read_with(cx, |view, cx| assert!(!view.tabs[0].read(cx).pane))
            .unwrap();
    }

    #[gpui::test]
    fn displayed_refresh_interval_follows_window_display_changes(cx: &mut gpui::TestAppContext) {
        let window = test_window(cx);
        let interval = std::time::Duration::from_secs(1) / 120;
        cx.simulate_display_refresh_interval_change(gpui::DisplayId::new(1), Some(interval));
        window
            .read_with(cx, |view, _| {
                assert_eq!(view.display_interval, Some(interval))
            })
            .unwrap();
        cx.simulate_display_refresh_interval_change(gpui::DisplayId::new(1), None);
        window
            .update(cx, |view, window, _| {
                assert_eq!(view.display_interval, None);
                assert_eq!(
                    window.frame_rate_limit(),
                    None,
                    "unknown Hz must retain native timing"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn presentation_limit_tracks_shared_settings_and_new_windows(cx: &mut gpui::TestAppContext) {
        let first = test_window(cx);
        let second = test_window(cx);
        first
            .update(cx, |_, w, _| assert_eq!(w.frame_rate_limit(), None))
            .unwrap();
        cx.update(|cx| {
            let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
            preferences.update(cx, |p, cx| p.change(|s| s.high_refresh_rate = false, cx));
        });
        for handle in [first, second] {
            handle
                .update(cx, |_, w, _| assert_eq!(w.frame_rate_limit(), Some(60.0)))
                .unwrap();
        }
        let third = test_window(cx);
        third
            .update(cx, |_, w, _| assert_eq!(w.frame_rate_limit(), Some(60.0)))
            .unwrap();
        cx.update(|cx| {
            let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
            preferences.update(cx, |p, cx| p.change(|s| s.high_refresh_rate = true, cx));
        });
        for handle in [first, second, third] {
            handle
                .update(cx, |_, w, _| assert_eq!(w.frame_rate_limit(), None))
                .unwrap();
        }
    }

    #[gpui::test]
    fn performance_display_tracks_shared_preferences_in_existing_and_new_windows(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = test_window(cx);
        let second = test_window(cx);
        cx.update(|cx| {
            let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
            preferences.update(cx, |p, cx| p.change(|s| s.performance_overlay = true, cx));
        });
        let third = test_window(cx);
        for window in [first, second, third] {
            window
                .update(cx, |_, window, _| {
                    assert!(window.frame_diagnostics_count().is_some())
                })
                .unwrap();
        }
        cx.update(|cx| {
            let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
            preferences.update(cx, |p, cx| p.change(|s| s.performance_overlay = false, cx));
        });
        for window in [first, second, third] {
            window
                .update(cx, |_, window, _| {
                    assert_eq!(window.frame_diagnostics_count(), None)
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn transferring_tabs_keeps_entity_and_only_target_subscriptions(cx: &mut gpui::TestAppContext) {
        let source = test_window(cx);
        let target = test_window(cx);
        let moved = source
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                let id = view.tabs[1].entity_id();
                view.transfer_tab(1, target, 0, w, cx);
                assert_eq!(view.tabs.len(), 1);
                assert_eq!(view.active, 0);
                assert!(!view.tab_subscriptions.contains_key(&id));
                id
            })
            .unwrap();
        target
            .update(cx, |view, _, _| {
                assert_eq!(view.tabs.len(), 2);
                assert_eq!(view.tabs[0].entity_id(), moved);
                assert_eq!(view.active, 0);
                assert!(view.tab_subscriptions.contains_key(&moved));
            })
            .unwrap();
    }

    #[gpui::test]
    fn merging_windows_preserves_every_entity_and_closes_empty_source(
        cx: &mut gpui::TestAppContext,
    ) {
        let target = test_window(cx);
        let source = test_window(cx);
        let id = source
            .read_with(cx, |view, _| view.tabs[0].entity_id())
            .unwrap();
        target
            .update(cx, |view, w, cx| {
                view.merge_windows(&MergeWindowTabs, w, cx);
                assert_eq!(view.tabs.len(), 2);
                assert!(view.tabs.iter().any(|tab| tab.entity_id() == id));
                assert_eq!(view.tab_subscriptions.len(), 2);
            })
            .unwrap();
        assert!(source.read_with(cx, |_, _| ()).is_err());
    }

    #[gpui::test]
    fn failed_destination_keeps_source_tab(cx: &mut gpui::TestAppContext) {
        let source = test_window(cx);
        let target = test_window(cx);
        target.update(cx, |_, w, _| w.remove_window()).unwrap();
        source
            .update(cx, |view, w, cx| {
                let before = view.tabs[0].entity_id();
                view.transfer_tab(0, target, 0, w, cx);
                assert_eq!(view.tabs.len(), 1);
                assert_eq!(view.tabs[0].entity_id(), before);
                assert!(view.tab_error.is_some());
            })
            .unwrap();
    }

    #[gpui::test]
    fn dropping_in_tab_strip_reorders_the_existing_entities(cx: &mut gpui::TestAppContext) {
        let window = test_window(cx);
        window
            .update(cx, |view, w, cx| {
                let first = view.tabs[0].entity_id();
                view.open_tab_at(Location::Home, w, cx);
                view.open_tab_at(Location::Home, w, cx);
                let outgoing = view.tabs[view.active].clone();
                outgoing.update(cx, |tab, cx| tab.hold_name_search_for_test(cx));
                view.tab_strip
                    .set(Bounds::new(point(px(10.), px(6.)), size(px(700.), px(28.))));
                view.dragging_tab = Some(first);
                // Headless windows have no native HWND for WindowFromPoint.
                view.finish_tab_drag_with_hit_test(point(px(650.), px(14.)), w, cx, |_, _| true);
                assert_eq!(view.tabs[2].entity_id(), first);
                assert_eq!(view.active, 2);
                assert!(view.dragging_tab.is_none());
                assert!(!outgoing.read(cx).name_search_running_for_test());
            })
            .unwrap();
    }

    #[gpui::test]
    fn detaching_background_tab_keeps_source_active_search(cx: &mut gpui::TestAppContext) {
        let window = test_window(cx);
        window
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                view.open_tab_at(Location::Home, w, cx);
                view.switch(0, w, cx);
                let active = view.tabs[0].clone();
                active.update(cx, |tab, cx| tab.hold_name_search_for_test(cx));
                view.detach_menu_tab(2, w, cx);
                assert_eq!(view.tabs.len(), 2);
                assert_eq!(view.tabs[view.active].entity_id(), active.entity_id());
                assert!(active.read(cx).name_search_running_for_test());
            })
            .unwrap();
    }
    #[gpui::test]
    fn overlapping_front_window_receives_drop_inside_source_tab_strip(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = test_window(cx);
        let target = test_window(cx);
        target
            .update(cx, |view, _, _| {
                view.tab_strip
                    .set(Bounds::new(point(px(10.), px(6.)), size(px(700.), px(28.))));
            })
            .unwrap();
        let moved = source
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                let moved = view.tabs[1].entity_id();
                view.tab_strip
                    .set(Bounds::new(point(px(10.), px(6.)), size(px(700.), px(28.))));
                view.dragging_tab = Some(moved);
                view.finish_tab_drag_with_hit_test(point(px(40.), px(14.)), w, cx, |window, _| {
                    window.window_handle() == target.into()
                });
                assert_eq!(
                    view.tabs.len(),
                    1,
                    "the covered source must not reorder or cancel"
                );
                moved
            })
            .unwrap();
        target
            .read_with(cx, |view, _| {
                assert_eq!(view.tabs.len(), 2);
                assert_eq!(view.tabs[0].entity_id(), moved);
                assert_eq!(view.active, 0);
            })
            .unwrap();
        assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    }

    #[gpui::test]
    fn releasing_outside_creates_a_window_without_recreating_the_folder(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = test_window(cx);
        let moved = window
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                let moved = view.tabs[1].entity_id();
                view.dragging_tab = Some(moved);
                view.finish_tab_drag(point(px(-300.), px(-300.)), w, cx);
                assert_eq!(view.tabs.len(), 1);
                moved
            })
            .unwrap();
        let ids = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .filter_map(|handle| handle.downcast::<Explorer>())
                .flat_map(|handle| {
                    handle
                        .read_with(cx, |view, _| {
                            view.tabs.iter().map(Entity::entity_id).collect::<Vec<_>>()
                        })
                        .unwrap()
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(ids.len(), 2);
        assert_eq!(ids.iter().filter(|&&id| id == moved).count(), 1);
    }

    #[gpui::test]
    fn cleared_drag_never_detaches_on_later_mouse_up(cx: &mut gpui::TestAppContext) {
        let window = test_window(cx);
        window
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                view.dragging_tab = None;
                view.finish_tab_drag(point(px(-300.), px(-300.)), w, cx);
                assert_eq!(view.tabs.len(), 2);
            })
            .unwrap();
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    }

    #[gpui::test]
    fn open_in_window_creates_a_separate_window_at_the_requested_place(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = test_window(cx);
        let location = Location::Folder(env!("CARGO_MANIFEST_DIR").into());
        source
            .update(cx, |view, _, cx| {
                view.tabs[0].update(cx, |_, cx| {
                    cx.emit(crate::folder::FolderEvent::OpenInWindow(location.clone()));
                });
            })
            .unwrap();
        cx.update(|cx| {
            let other_windows: Vec<_> = cx
                .windows()
                .into_iter()
                .filter_map(|handle| handle.downcast::<Explorer>())
                .filter(|handle| *handle != source)
                .collect();
            assert_eq!(other_windows.len(), 1);
            other_windows[0]
                .read_with(cx, |view, cx| {
                    assert_eq!(view.tabs.len(), 1);
                    assert_eq!(view.tabs[0].read(cx).tab_location(), &location);
                })
                .unwrap();
        });
        source
            .read_with(cx, |view, cx| {
                assert_eq!(view.tabs.len(), 1);
                assert_eq!(view.tabs[0].read(cx).tab_location(), &Location::Home);
            })
            .unwrap();
    }

    #[gpui::test]
    fn moved_folder_open_in_tab_event_reaches_only_its_current_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = test_window(cx);
        let target = test_window(cx);
        source
            .update(cx, |view, w, cx| {
                view.open_tab_at(Location::Home, w, cx);
                view.transfer_tab(1, target, 0, w, cx);
            })
            .unwrap();
        target
            .update(cx, |view, _, cx| {
                view.tabs[0].update(cx, |_, cx| {
                    cx.emit(crate::folder::FolderEvent::OpenInTab(Location::Folder(
                        std::env::temp_dir(),
                    )))
                });
            })
            .unwrap();
        assert_eq!(target.read_with(cx, |view, _| view.tabs.len()).unwrap(), 3);
        assert_eq!(source.read_with(cx, |view, _| view.tabs.len()).unwrap(), 1);
    }
}
