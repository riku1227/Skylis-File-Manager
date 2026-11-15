use super::*;
use gpui::{Bounds, KeyDownEvent, Point, anchored, canvas, deferred, point};
use std::{cell::Cell, rc::Rc};

const KEYS: [SortKey; 4] = [
    SortKey::Name,
    SortKey::Modified,
    SortKey::Kind,
    SortKey::Size,
];

pub(super) struct DisplaySettings {
    pub open: bool,
    pub trigger_focus: FocusHandle,
    focus: FocusHandle,
    anchor: Rc<Cell<Bounds<Pixels>>>,
    active_row: usize,
    scroll: gpui::ScrollHandle,
}

impl DisplaySettings {
    pub fn new(cx: &mut App) -> Self {
        Self {
            open: false,
            trigger_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            anchor: Default::default(),
            active_row: 0,
            scroll: gpui::ScrollHandle::new(),
        }
    }
}

pub(crate) fn bind_keys(cx: &mut App) {
    // Let the popup's raw key handler own these keys instead of Explorer's file actions.
    for context in ["DisplaySettings", "DisplaySettingsTrigger"] {
        cx.bind_keys(
            [
                "up",
                "down",
                "left",
                "right",
                "home",
                "end",
                "enter",
                "space",
                "escape",
                "tab",
                "shift-tab",
            ]
            .map(|key| gpui::KeyBinding::new(key, gpui::NoAction, Some(context))),
        );
    }
    cx.bind_keys([gpui::KeyBinding::new(
        "ctrl-h",
        gpui::NoAction,
        Some("DisplaySettings"),
    )]);
}

impl FolderView {
    pub(crate) fn close_display_settings(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = std::mem::replace(&mut self.display_settings.open, false);
        if was_open {
            cx.notify();
        }
        was_open
    }

    fn dismiss_display_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_display_settings(cx);
        window.focus(&self.display_settings.trigger_focus, cx);
    }

    pub(crate) fn open_display_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_scroll_motion();
        self.context_menu = None;
        self.display_settings.open = true;
        self.display_settings.active_row = self.display_rows()[0];
        self.display_settings
            .scroll
            .set_offset(point(px(0.), px(0.)));
        window.focus(&self.display_settings.focus, cx);
        cx.notify();
    }

    pub(super) fn request_pane(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.pane = enabled;
        cx.emit(FolderEvent::SetPane(enabled));
        cx.notify();
    }

    fn set_display_density(&mut self, compact: bool, cx: &mut Context<Self>) {
        if let Some(preferences) = cx
            .try_global::<crate::settings::SharedPreferences>()
            .map(|p| p.0.clone())
        {
            if preferences.read(cx).value.compact != compact {
                preferences.update(cx, |p, cx| p.change(|s| s.compact = compact, cx));
            }
        }
        self.compact = compact;
        cx.notify();
    }

    pub(super) fn render_display_trigger(
        &self,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let anchor = self.display_settings.anchor.clone();
        icon_button(
            "display-settings-trigger",
            "icons/view-settings.svg",
            true,
            t,
        )
        .debug_selector(|| "display-settings-trigger".into())
        .track_focus(&self.display_settings.trigger_focus)
        .tab_index(0)
        .key_context("DisplaySettingsTrigger")
        .relative()
        .border_1()
        .border_color(
            t.c("accent.fg")
                .opacity(if self.display_settings.open { 1. } else { 0. }),
        )
        .when(self.display_settings.open, |d| d.bg(t.c("accent.soft")))
        .focus(|s| s.border_color(t.c("accent.fg")))
        .child(
            canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
        .on_click(cx.listener(|this, _, w, cx| {
            cx.stop_propagation();
            if this.display_settings.open {
                this.dismiss_display_settings(w, cx);
            } else {
                this.open_display_settings(w, cx);
            }
        }))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
            if event.keystroke.modifiers.control
                || event.keystroke.modifiers.platform
                || event.keystroke.modifiers.alt
            {
                return;
            }
            match event.keystroke.key.as_str() {
                "enter" | "space" | "down" => {
                    this.open_display_settings(w, cx);
                    cx.stop_propagation();
                }
                "tab" => {
                    w.focus(&this.focus, cx);
                    cx.stop_propagation();
                }
                _ => (),
            }
        }))
    }

    // Stable row IDs let mouse and keyboard share the same operations. Omit
    // unavailable rows entirely, including their keyboard stops and headings.
    fn display_rows(&self) -> &'static [usize] {
        if self.is_home() {
            &[4, 5]
        } else if self.is_archive() {
            &[1, 2, 3, 4, 5]
        } else {
            &[0, 1, 2, 3, 4, 5]
        }
    }

    fn display_key(&mut self, event: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control && !modifiers.alt && !modifiers.platform && key == "h" {
            self.toggle_hidden(&ToggleHidden, w, cx);
            cx.stop_propagation();
            return;
        }
        if modifiers.control || modifiers.platform || modifiers.alt {
            return;
        }
        let rows = self.display_rows();
        let row = self.display_settings.active_row;
        let index = rows.iter().position(|r| *r == row).unwrap_or(0);
        match key {
            "escape" => self.dismiss_display_settings(w, cx),
            "tab" if modifiers.shift && index == 0 => self.dismiss_display_settings(w, cx),
            "tab" if !modifiers.shift && index == rows.len() - 1 => {
                self.close_display_settings(cx);
                w.focus(&self.focus, cx);
            }
            "tab" | "up" | "down" => {
                let previous = key == "up" || (key == "tab" && modifiers.shift);
                self.display_settings.active_row = rows[if previous {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(rows.len() - 1)
                }];
            }
            "left" | "right" | "home" | "end" | "enter" | "space" => {
                let last = matches!(key, "right" | "end");
                let activate = matches!(key, "enter" | "space");
                match row {
                    0 if !self.is_home() && !self.is_archive() => {
                        self.set_gallery(if activate { !self.gallery } else { last }, cx)
                    }
                    1 if !self.is_home() => {
                        let current = KEYS.iter().position(|k| *k == self.sort.key).unwrap_or(0);
                        let index = match key {
                            "home" => 0,
                            "end" => 3,
                            "left" => current.saturating_sub(1),
                            "right" => (current + 1).min(3),
                            _ => current,
                        };
                        self.choose_display_sort(KEYS[index], cx);
                    }
                    2 if !self.is_home() => self.apply_sort(
                        SortOrder {
                            key: self.sort.key,
                            ascending: if activate {
                                !self.sort.ascending
                            } else {
                                !last
                            },
                        },
                        cx,
                    ),
                    3 => self.request_pane(if activate { !self.pane } else { last }, cx),
                    4 => {
                        let enabled = if activate { !self.show_hidden } else { last };
                        if enabled != self.show_hidden {
                            self.toggle_hidden(&ToggleHidden, w, cx);
                        }
                    }
                    5 => self.set_display_density(if activate { !self.compact } else { last }, cx),
                    _ => (),
                }
            }
            _ => return,
        }
        // The scroll container also has a heading at each visible scope boundary.
        let mut item = 0;
        let mut previous_scope = None;
        for &row in rows {
            let scope = if row < 3 {
                0
            } else if row == 3 {
                1
            } else {
                2
            };
            if previous_scope != Some(scope) {
                item += 1;
            }
            if row == self.display_settings.active_row {
                break;
            }
            previous_scope = Some(scope);
            item += 1;
        }
        self.display_settings.scroll.scroll_to_item(item);
        cx.stop_propagation();
        cx.notify();
    }

    fn choose_display_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let ascending = self.sort.key != key || self.sort.ascending;
        self.apply_sort(SortOrder { key, ascending }, cx);
    }

    fn display_option(
        &self,
        id: &'static str,
        label: &'static str,
        selected: bool,
        row: usize,
        t: Theme,
        cx: &mut Context<Self>,
        edit: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if selected {
                t.c("accent.fg")
            } else {
                t.c("accent.fg").opacity(0.)
            })
            .when(selected, |d| {
                d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
            })
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|s| s.bg(t.c("state.hover")))
            .child(label)
            .on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                this.display_settings.active_row = row;
                w.focus(&this.display_settings.focus, cx);
                edit(this, w, cx);
                cx.notify();
            }))
    }

    fn display_switch(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        row: usize,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .items_center()
            .justify_between()
            .h(px(34.))
            .flex_shrink_0()
            .border_l_2()
            .border_color(
                t.c("accent.fg")
                    .opacity(if self.display_settings.active_row == row {
                        0.55
                    } else {
                        0.
                    }),
            )
            .cursor_pointer()
            .rounded(px(4.))
            .px(px(15.))
            .bg(t.c(if row == 3 {
                "accent.soft"
            } else {
                "surface.field"
            }))
            .hover(|s| s.bg(t.c("state.hover")))
            .child(label)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(t.c("text.muted"))
                            .child(if enabled { "表示" } else { "非表示" }),
                    )
                    .child(
                        div()
                            .w(px(30.))
                            .h(px(18.))
                            .p(px(3.))
                            .rounded(px(9.))
                            .flex()
                            .items_center()
                            .bg(t.c(if enabled {
                                "button.primary.bg"
                            } else {
                                "surface.field"
                            }))
                            .border_1()
                            .border_color(t.c("border.default"))
                            .when(enabled, |d| d.justify_end())
                            .child(div().size(px(10.)).rounded(px(5.)).bg(t.c(if enabled {
                                "text.on-accent"
                            } else {
                                "text.secondary"
                            }))),
                    ),
            )
            .on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                this.display_settings.active_row = row;
                w.focus(&this.display_settings.focus, cx);
                if row == 3 {
                    this.request_pane(!this.pane, cx);
                } else {
                    this.toggle_hidden(&ToggleHidden, w, cx);
                }
            }))
    }

    pub(super) fn render_display_settings(
        &self,
        t: Theme,
        w: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let anchor = self.display_settings.anchor.get();
        let viewport = w.viewport_size();
        let width = px(306.).min((viewport.width - px(16.)).max(px(1.)));
        let below = (viewport.height - anchor.bottom() - px(14.)).max(px(0.));
        let above = (anchor.origin.y - px(14.)).max(px(0.));
        let natural_height = if self.is_home() {
            191.
        } else if self.is_archive() {
            424.
        } else {
            489.
        };
        let height = px(natural_height)
            .min(below.max(above).max(px(80.)))
            .min((viewport.height - px(16.)).max(px(1.)));
        let x =
            (anchor.right() - width).clamp(px(8.), (viewport.width - width - px(8.)).max(px(8.)));
        let y = if below >= above {
            anchor.bottom() + px(6.)
        } else {
            anchor.origin.y - height - px(6.)
        }
        .clamp(px(8.), (viewport.height - height - px(8.)).max(px(8.)));
        let scope = |text: &'static str,
                     hint: &'static str,
                     icon: &'static str,
                     window: bool,
                     global: bool| {
            div()
                .h(px(44.))
                .flex_shrink_0()
                .px(px(15.))
                .border_t_1()
                .border_color(t.c("border.default"))
                .bg(t.c(if window {
                    "accent.soft"
                } else if global {
                    "surface.field"
                } else {
                    "surface.raised"
                }))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(t.c(if window {
                            "accent.fg"
                        } else {
                            "text.secondary"
                        }))
                        .child(svg().path(icon).size(px(13.)).text_color(t.c(if window {
                            "accent.fg"
                        } else {
                            "text.secondary"
                        })))
                        .child(text),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .text_color(t.c("text.muted"))
                        .child(hint),
                )
        };
        let section = |label: &'static str, row: usize| {
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap(px(5.))
                .py(px(4.))
                .px(px(12.))
                .bg(t.c(if row == 5 {
                    "surface.field"
                } else {
                    "surface.raised"
                }))
                .child(
                    div()
                        .border_l_2()
                        .pl(px(3.))
                        .border_color(t.c("accent.fg").opacity(
                            if self.display_settings.active_row == row {
                                0.8
                            } else {
                                0.
                            },
                        ))
                        .text_size(px(12.))
                        .child(label),
                )
        };
        let pair = || {
            div()
                .flex()
                .gap(px(4.))
                .p(px(3.))
                .bg(t.c("surface.field"))
                .rounded(px(5.))
        };
        let row = self.display_settings.active_row;
        let available = !self.is_home();
        let modes_available = available && !self.is_archive();
        let mut sorts = pair();
        for (key, id, label) in [
            (SortKey::Name, "display-sort-name", "名前"),
            (SortKey::Modified, "display-sort-modified", "更新日"),
            (SortKey::Kind, "display-sort-kind", "種類"),
            (SortKey::Size, "display-sort-size", "サイズ"),
        ] {
            sorts = sorts.child(self.display_option(
                id,
                label,
                self.sort.key == key,
                1,
                t,
                cx,
                move |this, _, cx| this.choose_display_sort(key, cx),
            ));
        }
        let mut body = div()
            .id("display-settings-scroll")
            .track_scroll(&self.display_settings.scroll)
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .pb(px(10.))
            .when(available, |d| {
                d.child(scope(
                    "このタブ",
                    "開いているタブに適用",
                    "icons/documents.svg",
                    false,
                    false,
                ))
            })
            .when(modes_available, |d| {
                d.child(
                    section("表示形式", 0).child(
                        pair()
                            .child(self.display_option(
                                "display-details",
                                "≡ 詳細",
                                !self.gallery,
                                0,
                                t,
                                cx,
                                |this, _, cx| this.set_gallery(false, cx),
                            ))
                            .child(self.display_option(
                                "display-gallery",
                                "▦ ギャラリー",
                                self.gallery,
                                0,
                                t,
                                cx,
                                |this, _, cx| this.set_gallery(true, cx),
                            )),
                    ),
                )
            })
            .when(available, |d| {
                d.child(
                    section(
                        if self.grouped {
                            "並べ替え（日付グループ）"
                        } else {
                            "並べ替え"
                        },
                        1,
                    )
                    .child(sorts),
                )
            })
            .when(available, |d| {
                d.child(
                    div()
                        .border_l_2()
                        .border_color(if row == 2 {
                            t.c("accent.fg").opacity(0.55)
                        } else {
                            t.c("accent.fg").opacity(0.)
                        })
                        .px(px(12.))
                        .pb(px(6.))
                        .child(
                            pair()
                                .child(self.display_option(
                                    "display-sort-ascending",
                                    "↑ 昇順",
                                    self.sort.ascending,
                                    2,
                                    t,
                                    cx,
                                    |this, _, cx| {
                                        this.apply_sort(
                                            SortOrder {
                                                key: this.sort.key,
                                                ascending: true,
                                            },
                                            cx,
                                        )
                                    },
                                ))
                                .child(self.display_option(
                                    "display-sort-descending",
                                    "↓ 降順",
                                    !self.sort.ascending,
                                    2,
                                    t,
                                    cx,
                                    |this, _, cx| {
                                        this.apply_sort(
                                            SortOrder {
                                                key: this.sort.key,
                                                ascending: false,
                                            },
                                            cx,
                                        )
                                    },
                                )),
                        ),
                )
            })
            .when(available, |d| {
                d.child(scope(
                    "このウィンドウ",
                    "このウィンドウの全タブに適用",
                    "icons/panel.svg",
                    true,
                    false,
                ))
                .child(self.display_switch(
                    "display-pane",
                    "詳細ペイン",
                    self.pane,
                    3,
                    t,
                    cx,
                ))
            })
            .child(scope(
                "すべてのウィンドウ",
                "他のウィンドウにも反映",
                "icons/restore.svg",
                false,
                true,
            ))
            .child(self.display_switch(
                "display-hidden",
                "隠しファイル",
                self.show_hidden,
                4,
                t,
                cx,
            ))
            .child(
                section("表示密度", 5).child(
                    pair()
                        .child(self.display_option(
                            "display-density-standard",
                            "標準",
                            !self.compact,
                            5,
                            t,
                            cx,
                            |this, _, cx| this.set_display_density(false, cx),
                        ))
                        .child(self.display_option(
                            "display-density-compact",
                            "コンパクト",
                            self.compact,
                            5,
                            t,
                            cx,
                            |this, _, cx| this.set_display_density(true, cx),
                        )),
                ),
            );
        if let Some(error) = cx
            .try_global::<crate::settings::SharedPreferences>()
            .and_then(|p| p.0.read(cx).error.clone())
        {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .text_color(t.c("danger.fg"))
                    .child(error),
            );
        }
        deferred(
            anchored()
                .position(Point::new(x, y))
                .snap_to_window()
                .child(
                    div()
                        .id("display-settings-panel")
                        .debug_selector(|| "display-settings-panel".into())
                        .key_context("DisplaySettings")
                        .track_focus(&self.display_settings.focus)
                        .occlude()
                        .w(width)
                        .h(height)
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded(px(8.))
                        .bg(t.c("surface.raised"))
                        .border_1()
                        .border_color(t.c("border.dialog"))
                        .shadow_lg()
                        .text_color(t.c("text.primary"))
                        .text_size(px(12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, w, cx| {
                            // The trigger handles its own toggle; don't close on down then reopen on click.
                            if !this.display_settings.anchor.get().contains(&event.position) {
                                this.close_display_settings(cx);
                                // Give blank areas a live dispatch path. A clicked input can
                                // still take focus in its own subsequent mouse-down handler.
                                w.focus(&this.focus, cx);
                            }
                        }))
                        .on_key_down(cx.listener(Self::display_key))
                        .child(
                            div()
                                .h(px(38.))
                                .flex_shrink_0()
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("表示設定")
                                .child(
                                    div()
                                        .id("display-settings-close")
                                        .debug_selector(|| "display-settings-close".into())
                                        .cursor_pointer()
                                        .p(px(5.))
                                        .child("×")
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.dismiss_display_settings(w, cx)
                                        })),
                                ),
                        )
                        .child(body),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}
