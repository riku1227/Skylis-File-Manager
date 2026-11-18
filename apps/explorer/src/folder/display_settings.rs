use super::*;
use crate::motion;
use gpui::{AnimationExt, SpringAnimation, relative};
use gpui::{Bounds, KeyDownEvent, anchored, canvas, deferred, point};
use std::{cell::Cell, rc::Rc};

const KEYS: [SortKey; 4] = [
    SortKey::Name,
    SortKey::Modified,
    SortKey::Kind,
    SortKey::Size,
];

pub(super) struct DisplaySettings {
    pub open: bool,
    pub(super) presence: motion::Presence<()>,
    pub trigger_focus: FocusHandle,
    focus: FocusHandle,
    anchor: Rc<Cell<Bounds<Pixels>>>,
    active_row: usize,
    pub(super) keyboard_mode: bool,
    scroll: gpui::ScrollHandle,
}

impl DisplaySettings {
    pub fn new(cx: &mut App) -> Self {
        Self {
            open: false,
            presence: Default::default(),
            trigger_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            anchor: Default::default(),
            active_row: 0,
            keyboard_mode: false,
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
        self.display_settings.keyboard_mode = false;
        self.close_search_filters(cx);
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
        if enabled && !self.pane {
            self.pane_generation = self.pane_generation.wrapping_add(1);
            self.pane_revealed_at = Some(cx.background_executor().now());
        }
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
        .when(!self.display_settings.open, |d| {
            d.tooltip(move |_, cx| {
                cx.new(|_| Tip {
                    text: "表示設定".into(),
                    theme: t,
                })
                .into()
            })
        })
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
                    this.display_settings.keyboard_mode = true;
                    cx.stop_propagation();
                }
                "tab" => {
                    if !event.keystroke.modifiers.shift && !this.pinned.is_empty() {
                        w.focus(&this.pin_remove_focus[0], cx);
                    } else {
                        w.focus(&this.focus, cx);
                    }
                    cx.stop_propagation();
                }
                _ => (),
            }
        }))
    }

    // Stable row IDs let mouse and keyboard share the same operations. Omit
    // unavailable rows entirely, including their keyboard stops and empty groups.
    fn display_rows(&self) -> &'static [usize] {
        if self.is_home() {
            &[4, 5]
        } else if self.is_archive() {
            &[1, 3, 4, 5]
        } else {
            &[0, 1, 3, 4, 5]
        }
    }

    fn display_key(&mut self, event: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control && !modifiers.alt && !modifiers.platform && key == "h" {
            self.display_settings.keyboard_mode = true;
            self.toggle_hidden(&ToggleHidden, w, cx);
            cx.stop_propagation();
            return;
        }
        if modifiers.control || modifiers.platform || modifiers.alt {
            return;
        }
        self.display_settings.keyboard_mode = true;
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
                        if activate {
                            self.choose_display_sort(self.sort.key, cx);
                        } else if KEYS[index] != self.sort.key {
                            self.apply_sort(
                                SortOrder {
                                    key: KEYS[index],
                                    ascending: true,
                                },
                                cx,
                            );
                        }
                    }
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
        // Separators and rows are direct scroll children.
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
            if previous_scope.is_some_and(|previous| previous != scope) {
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
        let ascending = self.sort.key != key || !self.sort.ascending;
        self.apply_sort(SortOrder { key, ascending }, cx);
    }

    fn display_label(label: &'static str, row: usize, t: Theme) -> impl IntoElement {
        div()
            .h(px(18.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .w(px(84.))
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(t.c("text.secondary"))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(t.c("text.muted"))
                    .child(match row {
                        0 | 1 => "このタブ",
                        3 => "このウィンドウ",
                        _ => "すべてのウィンドウ",
                    }),
            )
    }

    fn display_row(&self, row: usize, t: Theme) -> gpui::Div {
        div()
            .flex_shrink_0()
            .px(px(8.))
            .py(px(6.))
            .rounded(px(5.))
            .border_l_2()
            .border_color(t.c("focus.ring").opacity(
                if self.display_settings.keyboard_mode && self.display_settings.active_row == row {
                    1.
                } else {
                    0.
                },
            ))
    }

    fn display_segments(t: Theme) -> gpui::Div {
        div()
            .relative()
            .flex()
            .p(px(3.))
            .rounded(px(6.))
            .bg(t.c("surface.field"))
            .border_1()
            .border_color(t.c("border.default"))
    }

    fn display_option(
        &self,
        id: &'static str,
        label: String,
        selected: bool,
        row: usize,
        t: Theme,
        cx: &mut Context<Self>,
        edit: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .text_size(px(11.))
            .text_color(t.c("text.secondary"))
            .cursor_pointer()
            .hover(|s| s.bg(t.c("state.hover")))
            .child(
                div()
                    .relative()
                    .when(selected, |d| {
                        d.text_color(t.c("text.primary"))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .child(label),
            )
            .when(selected && row == 1, |d| {
                let text = if self.sort.ascending {
                    "もう一度押すと降順に切り替え"
                } else {
                    "もう一度押すと昇順に切り替え"
                };
                d.tooltip(move |_, cx| {
                    cx.new(|_| Tip {
                        text: text.into(),
                        theme: t,
                    })
                    .into()
                })
            })
            .on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                this.display_settings.keyboard_mode = false;
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
        self.display_row(row, t)
            .id(id)
            .debug_selector(move || id.into())
            .h(px(42.))
            .flex()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .hover(|s| s.bg(t.c("state.hover")))
            .child(Self::display_label(label, row, t))
            .child(motion::switch(
                ("display-switch", row),
                enabled,
                32.,
                t.c("meter.track"),
                t.c("button.primary.bg"),
                t.c("text.secondary"),
                t.c("text.on-accent"),
            ))
            .on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                this.display_settings.keyboard_mode = false;
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
        let Some(shown) = self.display_settings.presence.shown() else {
            return div().into_any_element();
        };
        let (progress, live) = (shown.progress, shown.live);
        let anchor = self.display_settings.anchor.get();
        let viewport = w.viewport_size();
        let width = px(306.).min((viewport.width - px(16.)).max(px(1.)));
        // Header + body padding + segment rows + switches + group separators + border.
        let rows = self.display_rows();
        let separators = rows
            .windows(2)
            .filter(|pair| (pair[0] < 3 && pair[1] >= 3) || (pair[0] == 3 && pair[1] >= 4))
            .count();
        let natural_height = 38.
            + 8.
            + 2.
            + rows
                .iter()
                .map(|row| if *row == 3 || *row == 4 { 42. } else { 74. })
                .sum::<f32>()
            + separators as f32 * 13.;
        let (position, height, below) =
            popover_placement(anchor, width, px(natural_height), viewport);
        let mut body = div()
            .id("display-settings-scroll")
            .track_scroll(&self.display_settings.scroll)
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .px(px(8.))
            .pb(px(8.));
        let mut previous_scope = None;
        for &row in self.display_rows() {
            let scope = if row < 3 {
                0
            } else if row == 3 {
                1
            } else {
                2
            };
            if previous_scope.is_some_and(|previous| previous != scope) {
                body = body.child(
                    div()
                        .h(px(13.))
                        .flex_shrink_0()
                        .pt(px(6.))
                        .child(div().h(px(1.)).bg(t.c("border.subtle"))),
                );
            }
            previous_scope = Some(scope);
            if row == 3 || row == 4 {
                body = body.child(self.display_switch(
                    if row == 3 {
                        "display-pane"
                    } else {
                        "display-hidden"
                    },
                    if row == 3 {
                        "詳細ペイン"
                    } else {
                        "隠しファイル"
                    },
                    if row == 3 {
                        self.pane
                    } else {
                        self.show_hidden
                    },
                    row,
                    t,
                    cx,
                ));
                continue;
            }
            let (selected_index, count) = match row {
                0 => (usize::from(self.gallery), 2),
                1 => (
                    KEYS.iter().position(|k| *k == self.sort.key).unwrap_or(0),
                    4,
                ),
                _ => (usize::from(self.compact), 2),
            };
            let mut segments = Self::display_segments(t).child(
                div().absolute().inset_0().m(px(4.)).child(
                    div()
                        .debug_selector(move || format!("display-segment-face-{row}"))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(relative(1. / count as f32))
                        .rounded(px(4.))
                        .bg(t.c("surface.chip-selected"))
                        .border_1()
                        .border_color(t.c("button.hover.border"))
                        .shadow_sm()
                        .with_spring(
                            ("display-segment", row),
                            SpringAnimation::new(motion::spring::BRISK)
                                .to(selected_index as f32 / count as f32),
                            |d, position| d.left(relative(position)),
                        ),
                ),
            );
            if row == 1 {
                for (key, id, label) in [
                    (SortKey::Name, "display-sort-name", "名前"),
                    (SortKey::Modified, "display-sort-modified", "更新日"),
                    (SortKey::Kind, "display-sort-kind", "種類"),
                    (SortKey::Size, "display-sort-size", "サイズ"),
                ] {
                    let selected = self.sort.key == key;
                    let label = if selected {
                        format!("{label} {}", if self.sort.ascending { "↑" } else { "↓" })
                    } else {
                        label.into()
                    };
                    segments = segments.child(self.display_option(
                        id,
                        label,
                        selected,
                        row,
                        t,
                        cx,
                        move |this, _, cx| this.choose_display_sort(key, cx),
                    ));
                }
            } else {
                let options = if row == 0 {
                    [
                        ("display-details", "詳細", !self.gallery),
                        ("display-gallery", "ギャラリー", self.gallery),
                    ]
                } else {
                    [
                        ("display-density-standard", "標準", !self.compact),
                        ("display-density-compact", "コンパクト", self.compact),
                    ]
                };
                for (index, (id, label, selected)) in options.into_iter().enumerate() {
                    segments = segments.child(self.display_option(
                        id,
                        label.into(),
                        selected,
                        row,
                        t,
                        cx,
                        move |this, _, cx| {
                            if row == 0 {
                                this.set_gallery(index == 1, cx)
                            } else {
                                this.set_display_density(index == 1, cx)
                            }
                        },
                    ));
                }
            }
            body = body.child(
                self.display_row(row, t)
                    .h(px(74.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(Self::display_label(
                        if row == 0 {
                            "表示形式"
                        } else if row == 1 {
                            "並べ替え"
                        } else {
                            "表示密度"
                        },
                        row,
                        t,
                    ))
                    .child(segments),
            );
        }
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
            anchored().position(position).snap_to_window().child(
                div()
                    .id("display-settings-panel")
                    .debug_selector(|| "display-settings-panel".into())
                    .key_context("DisplaySettings")
                    .when(live, |d| d.track_focus(&self.display_settings.focus))
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
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.display_settings.keyboard_mode = false;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, w, cx| {
                        // The trigger handles its own toggle; don't close on down then reopen on click.
                        // A ghost is already closed and must not move focus again.
                        if this.display_settings.open
                            && !this.display_settings.anchor.get().contains(&event.position)
                        {
                            this.close_display_settings(cx);
                            // Give blank areas a live dispatch path. A clicked input can
                            // still take focus in its own subsequent mouse-down handler.
                            w.focus(&this.focus, cx);
                        }
                    }))
                    .on_mouse_move(cx.listener(|this, _, _, cx| {
                        if this.display_settings.keyboard_mode {
                            this.display_settings.keyboard_mode = false;
                            cx.notify();
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
                            .child("表示設定"),
                    )
                    .child(body)
                    .when(!live, |d| d.child(motion::ghost_shield()))
                    // Arrives from, and leaves toward, the trigger's side.
                    .opacity(progress)
                    .relative()
                    .top(px(if below {
                        -motion::POPUP_OFFSET
                    } else {
                        motion::POPUP_OFFSET
                    } * (1. - progress))),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

pub(super) fn popover_placement(
    anchor: gpui::Bounds<Pixels>,
    width: Pixels,
    natural_height: Pixels,
    viewport: gpui::Size<Pixels>,
) -> (gpui::Point<Pixels>, Pixels, bool) {
    let below = (viewport.height - anchor.bottom() - px(14.)).max(px(0.));
    let above = (anchor.top() - px(14.)).max(px(0.));
    let opens_below = below >= above;
    let height = natural_height
        .min(below.max(above).max(px(80.)))
        .min((viewport.height - px(16.)).max(px(1.)));
    let x = (anchor.right() - width).clamp(px(8.), (viewport.width - width - px(8.)).max(px(8.)));
    let y = if opens_below {
        anchor.bottom() + px(6.)
    } else {
        anchor.top() - height - px(6.)
    }
    .clamp(px(8.), (viewport.height - height - px(8.)).max(px(8.)));
    (gpui::point(x, y), height, opens_below)
}
