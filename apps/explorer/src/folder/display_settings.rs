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
        self.display_settings.active_row = 0;
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
        div()
            .id("display-settings-trigger")
            .debug_selector(|| "display-settings-trigger".into())
            .track_focus(&self.display_settings.trigger_focus)
            .tab_index(0)
            .key_context("DisplaySettingsTrigger")
            .relative()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(34.))
            .px(px(10.))
            .flex_shrink_0()
            .rounded(px(6.))
            .border_1()
            .border_color(t.c(if self.display_settings.open {
                "accent.fg"
            } else {
                "border.subtle"
            }))
            .bg(t.c(if self.display_settings.open {
                "surface.chip-selected"
            } else {
                "surface.chrome"
            }))
            .text_color(t.c("accent.fg"))
            .text_size(px(11.))
            .cursor_pointer()
            .focus(|s| s.border_color(t.c("accent.fg")))
            .hover(|s| s.bg(t.c("state.hover")))
            .child(div().flex().flex_col().child("表示設定 ▾").child(
                div().text_size(px(9.)).child(if self.gallery {
                    "ギャラリー"
                } else {
                    "詳細"
                }),
            ))
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
        let row = self.display_settings.active_row;
        match key {
            "escape" => self.dismiss_display_settings(w, cx),
            "tab" if modifiers.shift && row == 0 => self.dismiss_display_settings(w, cx),
            "tab" if !modifiers.shift && row == 5 => {
                self.close_display_settings(cx);
                w.focus(&self.focus, cx);
            }
            "tab" | "up" | "down" => {
                let previous = key == "up" || (key == "tab" && modifiers.shift);
                self.display_settings.active_row = if previous {
                    row.saturating_sub(1)
                } else {
                    (row + 1).min(5)
                };
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
        // Row indices include the three scope headings in the scrolling content.
        self.display_settings
            .scroll
            .scroll_to_item([1, 2, 3, 5, 7, 8][self.display_settings.active_row]);
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
        enabled: bool,
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
            .opacity(if enabled { 1. } else { 0.4 })
            .text_size(px(11.))
            .when(enabled, |d| {
                d.cursor_pointer().hover(|s| s.bg(t.c("state.hover")))
            })
            .child(label)
            .on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                if enabled {
                    this.display_settings.active_row = row;
                    w.focus(&this.display_settings.focus, cx);
                    edit(this, w, cx);
                    cx.notify();
                }
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
            .px(px(3.))
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
        let height = px(446.)
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
        let scope = |text: &'static str| {
            div()
                .h(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_size(px(10.))
                .text_color(t.c("text.muted"))
                .child(text)
        };
        let section = |label: &'static str, active: bool| {
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap(px(5.))
                .py(px(4.))
                .border_l_2()
                .border_color(if active {
                    t.c("accent.fg").opacity(0.55)
                } else {
                    t.c("accent.fg").opacity(0.)
                })
                .pl(px(3.))
                .child(div().text_size(px(12.)).child(label))
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
                available,
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
            .px(px(12.))
            .pb(px(10.))
            .child(scope("このタブ"))
            .child(
                section("表示形式", row == 0)
                    .child(
                        pair()
                            .child(self.display_option(
                                "display-details",
                                "≡ 詳細",
                                !self.gallery,
                                available && !self.is_archive(),
                                0,
                                t,
                                cx,
                                |this, _, cx| this.set_gallery(false, cx),
                            ))
                            .child(self.display_option(
                                "display-gallery",
                                "▦ ギャラリー",
                                self.gallery,
                                available && !self.is_archive(),
                                0,
                                t,
                                cx,
                                |this, _, cx| this.set_gallery(true, cx),
                            )),
                    )
                    .when(self.is_archive(), |d| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(t.c("text.muted"))
                                .child("圧縮ファイル内は詳細表示です"),
                        )
                    }),
            )
            .child(
                section(
                    if self.grouped {
                        "並べ替え（日付グループ）"
                    } else {
                        "並べ替え"
                    },
                    row == 1,
                )
                .child(sorts),
            )
            .child(
                div()
                    .border_l_2()
                    .border_color(if row == 2 {
                        t.c("accent.fg").opacity(0.55)
                    } else {
                        t.c("accent.fg").opacity(0.)
                    })
                    .pl(px(3.))
                    .pb(px(6.))
                    .child(
                        pair()
                            .child(self.display_option(
                                "display-sort-ascending",
                                "↑ 昇順",
                                self.sort.ascending,
                                available,
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
                                available,
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
            .child(scope("このウィンドウ"))
            .child(self.display_switch("display-pane", "詳細ペイン", self.pane, 3, t, cx))
            .child(scope("すべてのウィンドウ"))
            .child(self.display_switch(
                "display-hidden",
                "隠しファイル",
                self.show_hidden,
                4,
                t,
                cx,
            ))
            .child(
                section("表示密度", row == 5).child(
                    pair()
                        .child(self.display_option(
                            "display-density-standard",
                            "標準",
                            !self.compact,
                            true,
                            5,
                            t,
                            cx,
                            |this, _, cx| this.set_display_density(false, cx),
                        ))
                        .child(self.display_option(
                            "display-density-compact",
                            "コンパクト",
                            self.compact,
                            true,
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
