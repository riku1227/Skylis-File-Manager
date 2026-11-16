use crate::{
    actions::*,
    folder::{FolderView, icon_button},
    shell_icons::ShellIcons,
    theme::{Theme, ThemeMode},
};
use explorer_core::location::Location;
use gpui::{App, Context, Entity, FocusHandle, Focusable, Window, div, img, prelude::*, px, svg};
mod settings_panel;
mod tabs;
use tabs::TabDrag;

fn operations_pending(cx: &App) -> bool {
    crate::file_operations::has_active()
        || crate::archive_jobs::shared(cx).is_some_and(|jobs| jobs.read(cx).has_active())
}

pub(crate) fn request_quit(cx: &mut App) {
    let properties_open = crate::native_shell::properties_active();
    if !operations_pending(cx) && !properties_open {
        cx.quit();
        return;
    }
    for handle in cx.windows() {
        if let Some(handle) = handle.downcast::<Explorer>() {
            let _ = handle.update(cx, |view, _, cx| {
                view.tab_error = Some(if properties_open {
                    "プロパティ画面を閉じてから終了してください。"
                } else { "ファイル操作が進行中です。完了を待つか、処理をキャンセルしてから終了してください。" }.into());
                cx.notify();
            });
        }
    }
}

/// Each tab owns its entity and asynchronous listing. Switching never redirects a pending batch.
pub struct Explorer {
    tabs: Vec<Entity<FolderView>>,
    active: usize,
    mode: ThemeMode,
    compact: bool,
    pane: bool,
    icons: Entity<ShellIcons>,
    preferences: Entity<crate::settings::Preferences>,
    settings_open: bool,
    settings_category: usize,
    settings_scroll: gpui::ScrollHandle,
    settings_focus: FocusHandle,
    last_saved_theme: crate::settings_data::ThemePreference,
    tab_subscriptions: std::collections::HashMap<gpui::EntityId, Vec<gpui::Subscription>>,
    tab_scroll: gpui::ScrollHandle,
    tab_strip: std::rc::Rc<std::cell::Cell<gpui::Bounds<gpui::Pixels>>>,
    dragging_tab: Option<gpui::EntityId>,
    drop_slot: Option<usize>,
    tab_menu: Option<(gpui::EntityId, gpui::Point<gpui::Pixels>)>,
    tab_error: Option<String>,
    display_interval: Option<std::time::Duration>,
    _display_subscription: gpui::Subscription,
    diagnostics: Entity<crate::diagnostics::PerformanceDisplay>,
}
impl Explorer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let request = crate::launch_request::current();
        let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
        let mode = match request.theme {
            Some(crate::launch_request::ThemeArgument::Light) => ThemeMode::Light,
            Some(crate::launch_request::ThemeArgument::Dark) => ThemeMode::Dark,
            Some(crate::launch_request::ThemeArgument::System) => ThemeMode::System,
            None => preferences.read(cx).value.theme.into(),
        };
        let compact = preferences.read(cx).value.compact;
        let icons = cx.new(ShellIcons::new);
        let start = request
            .path
            .clone()
            .map(Location::Folder)
            .unwrap_or(Location::Home);
        let handoff = request
            .handoff_token
            .as_deref()
            .and_then(crate::shell_handoff::PendingHandoff::from_token);
        let notice = request.error.clone().or_else(|| {
            ((request.switch_explorer || request.handoff_token.is_some()) && handoff.is_none())
                .then(|| crate::shell_handoff::RETAINED_NOTICE.into())
        });
        let folder = cx.new(|cx| FolderView::new(start, mode, icons.clone(), window, cx));
        folder.update(cx, |folder, cx| {
            folder.compact = compact;
            folder.configure_launch(request.selected_name.clone(), handoff, notice, cx);
        });
        Self::with_tabs(vec![folder], icons, mode, compact, window, cx)
    }

    fn with_tabs(
        tabs: Vec<Entity<FolderView>>,
        icons: Entity<ShellIcons>,
        mode: ThemeMode,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let last_properties_owner = crate::native_shell::properties_active()
                && cx.windows().iter().filter(|handle| handle.downcast::<Explorer>().is_some()).count() <= 1;
            if operations_pending(cx) || last_properties_owner {
                let _ = weak.update(cx, |this, cx| {
                    this.tab_error = Some(if last_properties_owner {
                        "プロパティ画面を閉じてから最後のウィンドウを閉じてください。"
                    } else { "ファイル操作が進行中です。完了を待つか、処理をキャンセルしてから閉じてください。" }.into());
                    cx.notify();
                });
                false
            } else { true }
        });
        let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
        apply_frame_limit(window, preferences.read(cx).value.high_refresh_rate);
        let diagnostics =
            cx.new(|cx| crate::diagnostics::PerformanceDisplay::new(mode, window, cx));
        diagnostics.update(cx, |view, cx| {
            view.configure(
                preferences.read(cx).value.performance_overlay,
                mode,
                window,
                cx,
            );
        });
        cx.observe_in(&preferences, window, |this, preferences, window, cx| {
            let value = preferences.read(cx).value.clone();
            apply_frame_limit(window, value.high_refresh_rate);
            if this.last_saved_theme != value.theme {
                this.mode = value.theme.into();
                this.last_saved_theme = value.theme;
            }
            this.compact = value.compact;
            this.diagnostics.update(cx, |view, cx| {
                view.configure(value.performance_overlay, this.mode, window, cx);
            });
            for tab in &this.tabs {
                tab.update(cx, |tab, cx| {
                    tab.mode = this.mode;
                    tab.compact = this.compact;
                    if !value.smooth_scroll {
                        tab.cancel_scroll_motion();
                    }
                    cx.notify();
                });
            }
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                crate::motion_preferences::refresh(cx);
                cx.notify();
            }
            if !window.is_window_active() && this.dragging_tab.take().is_some() {
                this.clear_tab_drop_indicators(window, cx);
                cx.stop_active_drag(window);
                cx.notify();
            }
        })
        .detach();
        let weak = cx.weak_entity();
        let display_subscription = window.observe_window_display(move |window, cx| {
            let interval = window.refresh_interval(cx);
            let _ = weak.update(cx, |this, cx| {
                this.display_interval = interval;
                cx.notify();
            });
        });
        let mut this = Self {
            tabs,
            active: 0,
            mode,
            compact,
            pane: true,
            icons,
            last_saved_theme: preferences.read(cx).value.theme,
            preferences,
            settings_open: false,
            settings_category: 0,
            settings_scroll: gpui::ScrollHandle::new(),
            settings_focus: cx.focus_handle(),
            tab_subscriptions: Default::default(),
            tab_scroll: gpui::ScrollHandle::new(),
            tab_strip: Default::default(),
            dragging_tab: None,
            drop_slot: None,
            tab_menu: None,
            tab_error: None,
            display_interval: window.refresh_interval(cx),
            _display_subscription: display_subscription,
            diagnostics,
        };
        for tab in this.tabs.clone() {
            this.watch_tab(&tab, window, cx);
        }
        this.switch(0, window, cx);
        this
    }
    fn switch(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[self.active].update(cx, |tab, cx| {
            tab.cancel_address(cx);
            tab.cancel_scroll_motion();
            tab.close_preview(cx);
            tab.close_display_settings(cx);
        });
        self.active = index;
        window.focus(&self.tabs[index].focus_handle(cx), cx);
        cx.notify();
    }
    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let folder =
            cx.new(|cx| FolderView::new(Location::Home, self.mode, self.icons.clone(), window, cx));
        self.watch_tab(&folder, window, cx);
        folder.update(cx, |folder, _| folder.compact = self.compact);
        self.tabs.push(folder);
        self.settings_open = false;
        self.switch(self.tabs.len() - 1, window, cx);
    }
    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 || index >= self.tabs.len() {
            return;
        }
        let removed = self.tabs.remove(index);
        removed.update(cx, |tab, _| tab.cancel_scroll_motion());
        self.tab_subscriptions.remove(&removed.entity_id());
        self.active = after_close(self.active, index, self.tabs.len());
        self.switch(self.active, window, cx);
    }
    fn close_tab(&mut self, _: &CloseTab, w: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, w, cx)
    }
    fn next_tab(&mut self, _: &NextTab, w: &mut Window, cx: &mut Context<Self>) {
        self.switch((self.active + 1) % self.tabs.len(), w, cx)
    }
    fn prev_tab(&mut self, _: &PrevTab, w: &mut Window, cx: &mut Context<Self>) {
        self.switch((self.active + self.tabs.len() - 1) % self.tabs.len(), w, cx)
    }
    fn cycle_theme(&mut self, _: &CycleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode.next();
        self.preferences
            .update(cx, |p, cx| p.change(|s| s.theme = mode.into(), cx));
    }
    fn density(&mut self, _: &ToggleDensity, _: &mut Window, cx: &mut Context<Self>) {
        let compact = !self.compact;
        self.preferences
            .update(cx, |p, cx| p.change(|s| s.compact = compact, cx));
    }
}
fn apply_frame_limit(window: &mut Window, high_refresh_rate: bool) {
    window
        .set_frame_rate_limit((!high_refresh_rate).then_some(60.0))
        .expect("60fps is a valid frame limit");
}
fn after_close(active: usize, closed: usize, remaining: usize) -> usize {
    if closed < active {
        active - 1
    } else {
        active.min(remaining - 1)
    }
}
impl Focusable for Explorer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tabs[self.active].focus_handle(cx)
    }
}
impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::resolve(self.mode, window.appearance());
        if self.tabs.is_empty() {
            return div().into_any_element();
        }
        window.set_window_title(&self.tabs[self.active].read(cx).label());
        let mut tabs = div()
            .id("tabs")
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .w(px(self.tabs.len() as f32 * 184.))
            .max_w_full()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll)
            .child(
                gpui::canvas(
                    {
                        let bounds = self.tab_strip.clone();
                        let scroll = self.tab_scroll.clone();
                        move |rect, _, _| {
                            bounds.set(gpui::Bounds::new(rect.origin - scroll.offset(), rect.size))
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        for (i, tab) in self.tabs.iter().enumerate() {
            let id = tab.entity_id();
            let owner = cx.weak_entity();
            let drag = TabDrag {
                id,
                label: tab.read(cx).label(),
                mode: self.mode,
            };
            let current = i == self.active;
            let archive = tab.read(cx).is_archive();
            let path = tab.read(cx).path().map(std::path::Path::to_path_buf);
            let image = path.as_ref().and_then(|path| {
                self.icons.update(cx, |icons, cx| {
                    icons.get_navigation(
                        path,
                        true,
                        (14. * window.scale_factor()).ceil() as u32,
                        cx,
                    )
                })
            });
            tabs = tabs.child(
                div()
                    .id(("tab", id))
                    .on_drag(drag, move |drag, _, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.dragging_tab = Some(drag.id);
                            this.tab_menu = None;
                            cx.notify();
                        });
                        cx.new(|_| drag.clone())
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Middle,
                        cx.listener(move |this, _, w, cx| {
                            cx.stop_propagation();
                            if let Some(index) =
                                this.tabs.iter().position(|tab| tab.entity_id() == id)
                            {
                                this.close(index, w, cx);
                            }
                        }),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.tab_menu = Some((id, event.position));
                            cx.notify();
                        }),
                    )
                    .h(px(28.))
                    .w(px(180.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.c(if current {
                        "tab.active.border"
                    } else {
                        "tab.border"
                    }))
                    .bg(t.c(if current { "tab.active.bg" } else { "tab.bg" }))
                    .text_color(t.c(if current { "text.primary" } else { "tab.fg" }))
                    .when(current, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                    .when(!current, |d| d.hover(|s| s.bg(t.c("tab.hover.bg"))))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .when_some(image, |d, image| d.child(img(image).size_full()))
                            .when(path.is_none(), |d| {
                                d.child(
                                    svg()
                                        .path(if archive {
                                            "icons/file.svg"
                                        } else {
                                            "icons/home.svg"
                                        })
                                        .size_full()
                                        .text_color(t.c("icon.default")),
                                )
                            }),
                    )
                    .child(div().flex_1().truncate().child(tab.read(cx).label()))
                    .child(
                        div()
                            .id(("close-tab", id))
                            .size(px(18.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .hover(|s| s.bg(t.c("state.pressed")))
                            .child(
                                svg()
                                    .path("icons/x.svg")
                                    .size(px(12.))
                                    .text_color(t.c("text.secondary")),
                            )
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                if let Some(index) =
                                    this.tabs.iter().position(|tab| tab.entity_id() == id)
                                {
                                    this.close(index, w, cx)
                                }
                            })),
                    )
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(index) = this.tabs.iter().position(|tab| tab.entity_id() == id)
                        {
                            this.switch(index, w, cx)
                        }
                    })),
            );
        }
        if let Some(slot) = self.drop_slot {
            tabs = tabs.child(
                div()
                    .absolute()
                    .left(px(slot as f32 * 184.))
                    .top(px(2.))
                    .w(px(2.))
                    .h(px(24.))
                    .bg(t.c("focus.ring")),
            );
        }
        div()
            .id("explorer")
            .relative()
            .key_context("ExplorerTabs")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.c("surface.canvas"))
            .text_color(t.c("text.primary"))
            .font_family(if cfg!(target_os = "macos") {
                ".SystemUIFont"
            } else {
                "Segoe UI"
            })
            .text_size(px(12.))
            .on_drag_move::<TabDrag>(cx.listener(
                |this, event: &gpui::DragMoveEvent<TabDrag>, w, cx| {
                    this.update_tab_drop_indicator(event.event.position, w, cx);
                },
            ))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, w, cx| {
                    this.finish_tab_drag(event.position, w, cx)
                }),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, w, cx| {
                    this.finish_tab_drag(event.position, w, cx)
                }),
            )
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                if event.keystroke.key == "escape"
                    && (this.dragging_tab.is_some() || this.tab_menu.is_some())
                {
                    this.dragging_tab = None;
                    this.tab_menu = None;
                    this.clear_tab_drop_indicators(w, cx);
                    cx.stop_active_drag(w);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.detach_tab(action, w, cx);
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.merge_windows(action, w, cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &ClearSearch, w, cx| {
                if this.dragging_tab.is_some() || this.tab_menu.is_some() {
                    this.dragging_tab = None;
                    this.tab_menu = None;
                    this.clear_tab_drop_indicators(w, cx);
                    cx.stop_active_drag(w);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::OpenPermissions, w, cx| {
                if cfg!(target_os = "macos") {
                    this.settings_category = 4;
                    this.settings_open = true;
                    this.settings_scroll.set_offset(gpui::point(px(0.), px(0.)));
                    w.focus(&this.settings_focus, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, w, cx| {
                this.settings_open = !this.settings_open;
                if this.settings_open {
                    crate::motion_preferences::refresh(cx);
                    this.tabs[this.active].update(cx, |tab, cx| {
                        tab.cancel_scroll_motion();
                        tab.close_display_settings(cx);
                    });
                    w.focus(&this.settings_focus, cx);
                } else {
                    w.focus(&this.tabs[this.active].focus_handle(cx), cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.new_tab(action, w, cx);
                }
            }))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.close_tab(action, w, cx);
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.next_tab(action, w, cx);
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.prev_tab(action, w, cx);
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.cycle_theme(action, w, cx);
                }
            }))
            .on_action(cx.listener(|this, action, w, cx| {
                if !this.settings_open {
                    this.density(action, w, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(38.))
                    .flex_shrink_0()
                    .pl(px(if cfg!(target_os = "macos") { 80. } else { 10. }))
                    .child(tabs.mt(px(6.)))
                    .child(
                        icon_button("new-tab", "icons/plus.svg", true, t)
                            .size(px(28.))
                            .mt(px(6.))
                            .on_click(cx.listener(|this, _, w, cx| this.new_tab(&NewTab, w, cx))),
                    )
                    .child(crate::window_chrome::drag_region())
                    .when(cfg!(target_os = "windows"), |d| {
                        d.child(window_controls(window.is_maximized(), true, t))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.tabs[self.active].clone()),
            )
            .when_some(self.tab_error.clone(), |d, message| {
                d.child(div().px(px(12.)).py(px(6.)).child(message))
            })
            .when(self.preferences.read(cx).value.performance_overlay, |d| {
                d.child(self.diagnostics.clone())
            })
            .when(self.tab_menu.is_some() && !self.settings_open, |d| {
                d.child(gpui::deferred(self.render_tab_menu(t, window, cx)).with_priority(2))
            })
            .when(self.settings_open, |d| {
                d.child(gpui::deferred(self.render_settings(t, window, cx)).with_priority(3))
            })
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn pane_is_shared_by_window_tabs_and_inherited_on_attach(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let explorer = window
            .read_with(cx, |probe, _| probe.explorer.clone())
            .unwrap();
        window
            .update(cx, |_, w, cx| {
                explorer.update(cx, |view, cx| {
                    view.settings_open = false;
                    view.new_tab(&NewTab, w, cx);
                    view.tabs[view.active].update(cx, |_, cx| {
                        cx.emit(crate::folder::FolderEvent::SetPane(false))
                    });
                });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, w, cx| {
                explorer.update(cx, |view, cx| {
                    assert!(!view.pane);
                    assert!(view.tabs.iter().all(|tab| !tab.read(cx).pane));
                    view.new_tab(&NewTab, w, cx);
                    assert!(!view.tabs[view.active].read(cx).pane);
                    // An arriving tab must adopt the destination window's choice.
                    let incoming = cx.new(|cx| {
                        FolderView::new(Location::Home, view.mode, view.icons.clone(), w, cx)
                    });
                    assert!(incoming.read(cx).pane);
                    view.watch_tab(&incoming, w, cx);
                    assert!(!incoming.read(cx).pane);
                });
            })
            .unwrap();
    }

    struct SettingsKeyProbe {
        explorer: Entity<Explorer>,
        bubbled_keys: Vec<gpui::Keystroke>,
    }

    impl Render for SettingsKeyProbe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, _| {
                    this.bubbled_keys.push(event.keystroke.clone());
                }))
                .child(self.explorer.clone())
        }
    }

    fn settings_key_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<SettingsKeyProbe> {
        cx.background_executor.allow_parking();
        cx.add_window(|window, cx| {
            // Exercise real preference changes without writing the user's settings.
            // The normal load-error path deliberately keeps the original file intact.
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
            let icons = cx.new(ShellIcons::new);
            let tab = cx.new(|cx| {
                FolderView::new(Location::Home, ThemeMode::Light, icons.clone(), window, cx)
            });
            let explorer = cx.new(|cx| {
                let mut explorer =
                    Explorer::with_tabs(vec![tab], icons, ThemeMode::Light, false, window, cx);
                explorer.settings_open = true;
                window.focus(&explorer.settings_focus, cx);
                explorer
            });
            SettingsKeyProbe {
                explorer,
                bubbled_keys: Vec::new(),
            }
        })
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn permission_action_opens_category_and_keeps_unknown(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        window
            .update(cx, |probe, w, cx| {
                probe.explorer.update(cx, |view, w_cx| {
                    view.close_settings(w, w_cx);
                });
                w.dispatch_action(Box::new(crate::OpenPermissions), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let status = visual.debug_bounds("permission-status").unwrap();
        let body = visual.debug_bounds("settings-body").unwrap();
        assert!(status.top() >= body.top() && status.bottom() <= body.bottom());
        window
            .read_with(cx, |probe, cx| {
                let view = probe.explorer.read(cx);
                assert!(view.settings_open);
                assert_eq!(view.settings_category, 4);
                assert_eq!(
                    view.tabs[view.active]
                        .read(cx)
                        .permission_observation
                        .observation,
                    crate::macos_permissions::Observation::Unknown
                );
                assert!(!view.preferences.read(cx).permission_busy);
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_backdrop_closes_and_restores_focus(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let outside = gpui::point(px(4.), px(80.));
        visual.simulate_mouse_down(outside, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_up(outside, gpui::MouseButton::Left, gpui::Modifiers::none());
        window
            .update(cx, |probe, w, cx| {
                let explorer = probe.explorer.read(cx);
                assert!(!explorer.settings_open, "outside click dismisses the modal");
                assert!(
                    explorer.tabs[explorer.active]
                        .focus_handle(cx)
                        .is_focused(w)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_controls_do_not_dismiss_or_leak_tab_actions(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let row = visual
            .debug_bounds("settings-density-compact")
            .unwrap()
            .center();
        visual.simulate_mouse_down(row, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_up(row, gpui::MouseButton::Left, gpui::Modifiers::none());
        let dark = visual.debug_bounds("settings-theme-dark").unwrap().center();
        visual.simulate_mouse_down(dark, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_up(dark, gpui::MouseButton::Left, gpui::Modifiers::none());
        window
            .read_with(cx, |probe, cx| {
                let explorer = probe.explorer.read(cx);
                assert!(explorer.settings_open);
                assert!(explorer.preferences.read(cx).value.compact);
                assert_eq!(
                    ThemeMode::from(explorer.preferences.read(cx).value.theme),
                    ThemeMode::Dark
                );
            })
            .unwrap();
        cx.dispatch_action(window.into(), NewTab);
        window
            .update(cx, |probe, w, cx| {
                let explorer = probe.explorer.read(cx);
                assert_eq!(
                    explorer.tabs.len(),
                    1,
                    "modal blocks background tab actions"
                );
                assert!(explorer.settings_open);
                assert!(explorer.settings_focus.is_focused(w));
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_small_window_keeps_close_control_visible(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(640.), px(360.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let panel = visual.debug_bounds("settings-panel").unwrap();
        let body = visual.debug_bounds("settings-body").unwrap();
        let done = visual.debug_bounds("settings-done").unwrap();
        assert!(panel.origin.y >= px(24.));
        assert!(
            panel.bottom() <= px(336.),
            "modal must fit the short window: {panel:?}"
        );
        assert!(body.size.height > px(40.));
        assert!(done.bottom() <= panel.bottom());
        visual.simulate_mouse_down(
            done.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        visual.simulate_mouse_up(
            done.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        window
            .read_with(cx, |probe, cx| {
                assert!(!probe.explorer.read(cx).settings_open)
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_preserve_modified_digits_and_propagate_unhandled_shortcuts(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = settings_key_window(cx);
        let before = window
            .read_with(cx, |probe, cx| {
                probe.explorer.read(cx).preferences.read(cx).value.clone()
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.simulate_keystrokes(
            window.into(),
            "alt-f4 ctrl-1 alt-2 shift-3 cmd-4 ctrl-5 ctrl-escape 9 x",
        );
        cx.dispatch_keystroke(
            window.into(),
            gpui::Keystroke {
                modifiers: gpui::Modifiers {
                    function: true,
                    ..Default::default()
                },
                key: "5".into(),
                key_char: None,
            },
        );
        window
            .read_with(cx, |probe, cx| {
                let explorer = probe.explorer.read(cx);
                assert_eq!(explorer.preferences.read(cx).value, before);
                assert!(
                    explorer.settings_open,
                    "modified Escape is not the panel's close key"
                );
                let keys: Vec<_> = probe
                    .bubbled_keys
                    .iter()
                    .map(|key| key.key.as_str())
                    .collect();
                assert_eq!(
                    keys,
                    ["f4", "1", "2", "3", "4", "5", "escape", "9", "x", "5"]
                );
                assert!(probe.bubbled_keys[0].modifiers.alt);
                assert!(probe.bubbled_keys[9].modifiers.function);
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_consume_plain_digit_and_escape_and_restore_folder_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = settings_key_window(cx);
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "2");
        window
            .read_with(cx, |probe, cx| {
                let explorer = probe.explorer.read(cx);
                assert!(explorer.preferences.read(cx).value.compact);
                assert!(explorer.settings_open);
                assert!(probe.bubbled_keys.is_empty());
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        window
            .update(cx, |probe, window, cx| {
                let explorer = probe.explorer.read(cx);
                assert!(!explorer.settings_open);
                assert!(
                    explorer.tabs[explorer.active]
                        .focus_handle(cx)
                        .is_focused(window)
                );
                assert!(probe.bubbled_keys.is_empty());
            })
            .unwrap();
    }

    fn click_setting(visual: &mut gpui::VisualTestContext, selector: &'static str) {
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let point = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_mouse_down(point, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_up(point, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }

    #[gpui::test]
    fn settings_categories_only_change_visible_content(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let before = window
            .read_with(cx, |p, cx| {
                p.explorer.read(cx).preferences.read(cx).value.clone()
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let selectors = [
            "settings-theme-system",
            "settings-row-downloads",
            "settings-row-high-refresh",
            "settings-row-performance",
        ];
        for category in [1, 2, 3, 0] {
            click_setting(
                &mut visual,
                [
                    "settings-category-0",
                    "settings-category-1",
                    "settings-category-2",
                    "settings-category-3",
                ][category],
            );
            for (index, selector) in selectors.iter().enumerate() {
                assert_eq!(
                    visual.debug_bounds(selector).is_some(),
                    index == category,
                    "{selector}"
                );
            }
            window
                .read_with(cx, |p, cx| {
                    let e = p.explorer.read(cx);
                    assert_eq!(e.settings_category, category);
                    assert_eq!(e.preferences.read(cx).value, before);
                    assert!(e.settings_open);
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn settings_explicit_density_is_idempotent(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for (selector, compact) in [
            ("settings-density-compact", true),
            ("settings-density-standard", false),
        ] {
            for _ in 0..2 {
                click_setting(&mut visual, selector);
                window
                    .read_with(cx, |p, cx| {
                        let e = p.explorer.read(cx);
                        assert_eq!(e.preferences.read(cx).value.compact, compact);
                        assert!(e.settings_open);
                        assert_eq!(e.tabs.len(), 1);
                    })
                    .unwrap();
            }
        }
    }

    #[gpui::test]
    fn settings_keyboard_categories_and_digits_show_the_target(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(640.), px(360.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let before = window
            .read_with(cx, |p, cx| {
                p.explorer.read(cx).preferences.read(cx).value.clone()
            })
            .unwrap();
        let last_category = if cfg!(any(windows, target_os = "macos")) {
            4
        } else {
            3
        };
        for (key, category) in [("up", last_category), ("down", 0), ("down", 1)] {
            cx.simulate_keystrokes(window.into(), key);
            window
                .read_with(cx, |p, cx| {
                    let e = p.explorer.read(cx);
                    assert_eq!(e.settings_category, category);
                    assert_eq!(e.preferences.read(cx).value, before);
                    assert!(p.bubbled_keys.is_empty());
                })
                .unwrap();
        }
        for (key, category, selector) in [
            ("8", 3, "settings-row-performance"),
            ("7", 2, "settings-row-smooth-scroll"),
            ("6", 2, "settings-row-high-refresh"),
            ("5", 1, "settings-row-sizes"),
            ("4", 1, "settings-row-gallery"),
            ("3", 1, "settings-row-downloads"),
            ("2", 0, "settings-density-compact"),
            ("1", 0, "settings-theme-light"),
        ] {
            cx.simulate_keystrokes(window.into(), key);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let body = visual.debug_bounds("settings-body").unwrap();
            let target = visual.debug_bounds(selector).unwrap();
            assert!(
                target.top() >= body.top() && target.top() < body.bottom(),
                "{key}: {target:?} / {body:?}"
            );
            window
                .read_with(cx, |p, cx| {
                    assert_eq!(p.explorer.read(cx).settings_category, category);
                    assert!(p.bubbled_keys.is_empty());
                })
                .unwrap();
        }
        window
            .read_with(cx, |p, cx| {
                let e = p.explorer.read(cx);
                let value = &e.preferences.read(cx).value;
                assert_eq!(value.compact, !before.compact);
                assert_eq!(value.group_downloads, !before.group_downloads);
                assert_eq!(value.auto_gallery, !before.auto_gallery);
                assert_eq!(value.automatic_sizes, !before.automatic_sizes);
                assert_eq!(value.high_refresh_rate, !before.high_refresh_rate);
                assert_eq!(value.smooth_scroll, !before.smooth_scroll);
                assert_eq!(value.performance_overlay, !before.performance_overlay);
                assert_eq!(
                    ThemeMode::from(value.theme),
                    ThemeMode::from(before.theme).next()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_layout_bounds_at_small_and_regular_sizes(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for (width, height) in [(640., 360.), (900., 600.), (480., 360.)] {
            visual.simulate_resize(gpui::size(px(width), px(height)));
            for category in 0..if cfg!(windows) { 5 } else { 4 } {
                click_setting(
                    &mut visual,
                    [
                        "settings-category-0",
                        "settings-category-1",
                        "settings-category-2",
                        "settings-category-3",
                        "settings-category-4",
                    ][category],
                );
                let panel = visual.debug_bounds("settings-panel").unwrap();
                let body = visual.debug_bounds("settings-body").unwrap();
                let done = visual.debug_bounds("settings-done").unwrap();
                assert!(panel.left() >= px(16.) && panel.right() <= px(width - 16.));
                assert!(panel.top() >= px(24.) && panel.bottom() <= px(height - 24.));
                assert!(body.size.height > px(40.));
                assert!(body.left() >= panel.left() && body.right() <= panel.right());
                assert!(body.bottom() <= done.top());
                assert!(done.bottom() <= panel.bottom() && done.right() <= panel.right());
            }
        }
    }

    #[test]
    fn closing_tabs_preserves_active_neighbor() {
        assert_eq!(after_close(3, 0, 3), 2);
        assert_eq!(after_close(2, 2, 2), 1);
        assert_eq!(after_close(0, 2, 2), 0);
        assert_eq!(after_close(0, 0, 1), 0);
    }

    #[gpui::test]
    fn motion_shortcuts_change_independent_preferences(cx: &mut gpui::TestAppContext) {
        let window = settings_key_window(cx);
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "6");
        window
            .update(cx, |probe, w, cx| {
                let explorer = probe.explorer.read(cx);
                let value = &explorer.preferences.read(cx).value;
                assert!(!value.high_refresh_rate);
                assert!(value.smooth_scroll);
                assert_eq!(w.frame_rate_limit(), Some(60.0));
                assert!(probe.bubbled_keys.is_empty());
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "7");
        window
            .read_with(cx, |probe, cx| {
                let value = &probe.explorer.read(cx).preferences.read(cx).value;
                assert!(!value.smooth_scroll);
                assert!(!value.high_refresh_rate);
                assert!(probe.bubbled_keys.is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn performance_shortcut_toggles_the_measurement_with_the_setting(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = settings_key_window(cx);
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "8");
        window
            .update(cx, |probe, window, cx| {
                assert!(
                    probe
                        .explorer
                        .read(cx)
                        .preferences
                        .read(cx)
                        .value
                        .performance_overlay
                );
                assert!(window.frame_diagnostics_count().is_some());
                assert!(probe.bubbled_keys.is_empty());
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "8");
        window
            .update(cx, |probe, window, cx| {
                assert!(
                    !probe
                        .explorer
                        .read(cx)
                        .preferences
                        .read(cx)
                        .value
                        .performance_overlay
                );
                assert_eq!(window.frame_diagnostics_count(), None);
            })
            .unwrap();
    }
}

/// Native Windows hit tests retain drag, double-click zoom, resize and Snap behavior.
/// The backend performs system-button actions; registering an on_click too would toggle twice.
pub(crate) fn window_controls(maximized: bool, minimizable: bool, t: Theme) -> impl IntoElement {
    let button = |id: &'static str, icon: &'static str, area: gpui::WindowControlArea| {
        div()
            .id(id)
            .w(px(46.))
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(area)
            .hover(|s| {
                s.bg(t.c(if area == gpui::WindowControlArea::Close {
                    "danger.solid"
                } else {
                    "state.hover"
                }))
            })
            .child(
                svg()
                    .path(icon)
                    .size(px(12.))
                    .text_color(t.c("text.primary")),
            )
    };
    div()
        .flex()
        .items_start()
        .h_full()
        .flex_shrink_0()
        .when(minimizable, |d| {
            d.child(button(
                "window-minimize",
                "icons/minus.svg",
                gpui::WindowControlArea::Min,
            ))
        })
        .child(button(
            "window-maximize",
            if maximized {
                "icons/restore.svg"
            } else {
                "icons/maximize.svg"
            },
            gpui::WindowControlArea::Max,
        ))
        .child(button(
            "window-close",
            "icons/x.svg",
            gpui::WindowControlArea::Close,
        ))
}
