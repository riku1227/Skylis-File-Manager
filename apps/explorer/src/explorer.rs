use crate::{
    actions::*,
    folder::{FolderView, icon_button},
    shell_icons::ShellIcons,
    theme::{Theme, ThemeMode},
};
use explorer_core::location::Location;
use gpui::{App, Context, Entity, FocusHandle, Focusable, Window, div, img, prelude::*, px, svg};
use std::path::PathBuf;
mod settings_panel;
mod tabs;
use tabs::TabDrag;

/// Each tab owns its entity and asynchronous listing. Switching never redirects a pending batch.
pub struct Explorer {
    tabs: Vec<Entity<FolderView>>,
    active: usize,
    mode: ThemeMode,
    compact: bool,
    icons: Entity<ShellIcons>,
    preferences: Entity<crate::settings::Preferences>,
    settings_open: bool,
    settings_focus: FocusHandle,
    last_saved_theme: crate::settings_data::ThemePreference,
    tab_subscriptions: std::collections::HashMap<gpui::EntityId, Vec<gpui::Subscription>>,
    tab_scroll: gpui::ScrollHandle,
    tab_strip: std::rc::Rc<std::cell::Cell<gpui::Bounds<gpui::Pixels>>>,
    dragging_tab: Option<gpui::EntityId>,
    drop_slot: Option<usize>,
    tab_menu: Option<(gpui::EntityId, gpui::Point<gpui::Pixels>)>,
    tab_error: Option<String>,
}
impl Explorer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let mut path = None;
        let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
        let mut mode = preferences.read(cx).value.theme.into();
        let compact = preferences.read(cx).value.compact;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--theme" && i + 1 < args.len() {
                i += 1;
                mode = match args[i].to_str() {
                    Some("dark") => ThemeMode::Dark,
                    Some("light") => ThemeMode::Light,
                    _ => ThemeMode::System,
                };
            } else if args[i] == "--path" && i + 1 < args.len() {
                i += 1;
                path = Some(PathBuf::from(&args[i]));
            } else if !args[i].to_string_lossy().starts_with('-') {
                path = Some(PathBuf::from(&args[i]));
            }
            i += 1;
        }
        let icons = cx.new(ShellIcons::new);
        let start = path.map(Location::Folder).unwrap_or(Location::Home);
        let folder = cx.new(|cx| FolderView::new(start, mode, icons.clone(), window, cx));
        folder.update(cx, |folder, _| folder.compact = compact);
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
        let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
        cx.observe(&preferences, |this, preferences, cx| {
            let value = preferences.read(cx).value.clone();
            if this.last_saved_theme != value.theme {
                this.mode = value.theme.into();
                this.last_saved_theme = value.theme;
            }
            this.compact = value.compact;
            for tab in &this.tabs {
                tab.update(cx, |tab, cx| {
                    tab.mode = this.mode;
                    tab.compact = this.compact;
                    cx.notify();
                });
            }
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && this.dragging_tab.take().is_some() {
                this.clear_tab_drop_indicators(window, cx);
                cx.stop_active_drag(window);
                cx.notify();
            }
        })
        .detach();
        let mut this = Self {
            tabs,
            active: 0,
            mode,
            compact,
            icons,
            last_saved_theme: preferences.read(cx).value.theme,
            preferences,
            settings_open: false,
            settings_focus: cx.focus_handle(),
            tab_subscriptions: Default::default(),
            tab_scroll: gpui::ScrollHandle::new(),
            tab_strip: Default::default(),
            dragging_tab: None,
            drop_slot: None,
            tab_menu: None,
            tab_error: None,
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
        self.tabs[self.active].update(cx, |tab, cx| tab.cancel_address(cx));
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
            let path = tab.read(cx).path().map(std::path::Path::to_path_buf);
            let image = path.as_ref().and_then(|path| {
                self.icons.update(cx, |icons, cx| {
                    icons.get(path, true, (14. * window.scale_factor()).ceil() as u32, cx)
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
                                        .path("icons/home.svg")
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
            .font_family("Segoe UI")
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
            .on_action(cx.listener(|this, _: &OpenSettings, w, cx| {
                this.settings_open = !this.settings_open;
                if this.settings_open {
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
                    .child(
                        div()
                            .id("titlebar-drag")
                            .flex_1()
                            .min_w(px(32.))
                            .h_full()
                            .window_control_area(gpui::WindowControlArea::Drag),
                    )
                    .when(cfg!(target_os = "windows"), |d| {
                        d.child(window_controls(window.is_maximized(), t))
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
            .when(self.tab_menu.is_some() && !self.settings_open, |d| {
                d.child(gpui::deferred(self.render_tab_menu(t, window, cx)).with_priority(2))
            })
            .when(self.settings_open, |d| {
                d.child(gpui::deferred(self.render_settings(t, cx)).with_priority(3))
            })
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

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
            .debug_bounds("settings-row-density")
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
            "alt-f4 ctrl-1 alt-2 shift-3 cmd-4 ctrl-5 ctrl-escape 6 x",
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
                    ["f4", "1", "2", "3", "4", "5", "escape", "6", "x", "5"]
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

    #[test]
    fn closing_tabs_preserves_active_neighbor() {
        assert_eq!(after_close(3, 0, 3), 2);
        assert_eq!(after_close(2, 2, 2), 1);
        assert_eq!(after_close(0, 2, 2), 0);
        assert_eq!(after_close(0, 0, 1), 0);
    }
}

/// Native Windows hit tests retain drag, double-click zoom, resize and Snap behavior.
/// The backend performs system-button actions; registering an on_click too would toggle twice.
fn window_controls(maximized: bool, t: Theme) -> impl IntoElement {
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
        .child(button(
            "window-minimize",
            "icons/minus.svg",
            gpui::WindowControlArea::Min,
        ))
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
