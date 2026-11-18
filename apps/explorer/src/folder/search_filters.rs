use super::*;
use crate::motion;
use explorer_core::name_search::{Filters, Kind, ModifiedFilter, ModifiedPeriod};
use gpui::{Bounds, canvas, deferred};
use std::{cell::Cell, rc::Rc};

mod modified;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchTier {
    Wide,
    Medium,
    Narrow,
}

impl SearchTier {
    fn for_window(window: &Window) -> Self {
        let width = window.viewport_size().width;
        if width < px(560.) {
            Self::Narrow
        } else if width < px(1150.) {
            Self::Medium
        } else {
            Self::Wide
        }
    }

    fn field_width(self) -> Pixels {
        px(match self {
            Self::Wide => 256.,
            Self::Medium => 190.,
            Self::Narrow => 76.,
        })
    }

    fn input_min_width(self) -> Pixels {
        px(match self {
            Self::Wide => 112.,
            Self::Medium => 56.,
            Self::Narrow => 39.,
        })
    }
}

const KINDS: [(Kind, &str); 3] = [
    (Kind::Any, "すべて"),
    (Kind::File, "ファイル"),
    (Kind::Folder, "フォルダー"),
];

const MODIFIED_PERIODS: [(Option<ModifiedPeriod>, &str); 4] = [
    (None, "指定なし"),
    (Some(ModifiedPeriod::Today), "今日"),
    (Some(ModifiedPeriod::LastSevenDays), "過去7日間"),
    (Some(ModifiedPeriod::LastThirtyDays), "過去30日間"),
];

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("down", gpui::NoAction, Some("SearchConditionsTrigger")),
        gpui::KeyBinding::new("left", gpui::NoAction, Some("SearchConditionsOptions")),
        gpui::KeyBinding::new("right", gpui::NoAction, Some("SearchConditionsOptions")),
        gpui::KeyBinding::new("backspace", gpui::NoAction, Some("SearchConditionsControl")),
    ]);
}

pub(super) struct SearchFilters {
    pub(super) criteria: Filters,
    pub(super) open: bool,
    pub(super) presence: motion::Presence<Filters>,
    field_anchor: Rc<Cell<Bounds<Pixels>>>,
    trigger_anchor: Rc<Cell<Bounds<Pixels>>>,
    clear: FocusHandle,
    clear_all: FocusHandle,
    trigger: FocusHandle,
    kinds: [FocusHandle; 3],
    modified: [FocusHandle; 4],
    extension: Entity<TextInput>,
}

impl SearchFilters {
    pub(super) fn new(cx: &mut Context<FolderView>) -> Self {
        let extension = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.placeholder = "例: .pdf".into();
            input
        });
        cx.observe(&extension, |view, input, cx| {
            let input = input.read(cx);
            if !input.is_composing() {
                let criteria = view
                    .search_filters
                    .criteria
                    .clone()
                    .with_extension(&input.content);
                view.apply_search_filters(criteria, cx);
            }
        })
        .detach();
        Self {
            criteria: Filters::default(),
            open: false,
            presence: Default::default(),
            field_anchor: Default::default(),
            trigger_anchor: Default::default(),
            clear: cx.focus_handle(),
            clear_all: cx.focus_handle(),
            trigger: cx.focus_handle(),
            kinds: std::array::from_fn(|_| cx.focus_handle()),
            modified: std::array::from_fn(|_| cx.focus_handle()),
            extension,
        }
    }
}

impl FolderView {
    pub(super) fn apply_search_filters(&mut self, mut criteria: Filters, cx: &mut Context<Self>) {
        if self.path().is_none() {
            return;
        }
        if let Some(updated) = rebased_modified_filter(&criteria) {
            criteria = criteria.with_modified(Some(updated));
        }
        if self.search_filters.criteria == criteria {
            return;
        }
        self.search_filters.criteria = criteria;
        self.context_menu = None;
        let keep = self.selected_entry().map(|entry| entry.path.clone());
        self.rebuild_visible(keep.as_deref());
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.query_search_changed(self.search.read(cx).is_composing(), cx);
        cx.notify();
    }

    fn set_search_kind(&mut self, kind: Kind, window: &Window, cx: &mut Context<Self>) {
        if kind == Kind::Folder {
            self.search_filters
                .extension
                .update(cx, |input, cx| input.reset(window, cx));
        }
        self.apply_search_filters(self.search_filters.criteria.clone().with_kind(kind), cx);
    }

    fn set_search_modified_period(
        &mut self,
        period: Option<ModifiedPeriod>,
        cx: &mut Context<Self>,
    ) {
        let filter = period.and_then(|period| modified::filter_at(period, chrono::Local::now()));
        if period.is_none() || filter.is_some() {
            self.apply_search_filters(
                self.search_filters.criteria.clone().with_modified(filter),
                cx,
            );
        }
    }

    pub(super) fn rebase_search_modified_period(&mut self) -> bool {
        let Some(updated) = rebased_modified_filter(&self.search_filters.criteria) else {
            return false;
        };
        self.search_filters.criteria =
            std::mem::take(&mut self.search_filters.criteria).with_modified(Some(updated));
        true
    }

    /// Start a refresh or traversal with the same date range in both sections.
    pub(super) fn refresh_search_modified_period(&mut self, cx: &mut Context<Self>) {
        if self.rebase_search_modified_period() {
            let keep = self.selected_entry().map(|entry| entry.path.clone());
            self.rebuild_visible(keep.as_deref());
            cx.notify();
        }
    }

    pub(super) fn reset_search_filters(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.search_filters.open = false;
        self.search_filters.criteria = Filters::default();
        self.search_filters
            .extension
            .update(cx, |input, cx| input.reset(window, cx));
    }

    fn clear_search_conditions(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.apply_search_filters(Filters::default(), cx);
        self.search_filters
            .extension
            .update(cx, |input, cx| input.reset(window, cx));
    }

    pub(super) fn filter_popup_focus(&self, window: &Window, cx: &App) -> bool {
        let f = &self.search_filters;
        f.open
            && (f
                .kinds
                .iter()
                .chain(&f.modified)
                .any(|focus| focus.is_focused(window))
                || f.extension.focus_handle(cx).is_focused(window)
                || f.clear.is_focused(window)
                || f.clear_all.is_focused(window))
    }

    pub(super) fn search_filter_focus(&self, window: &Window, cx: &App) -> bool {
        self.search_filters.trigger.is_focused(window) || self.filter_popup_focus(window, cx)
    }

    fn search_filter_context(&self, window: &Window, cx: &App) -> bool {
        self.search.focus_handle(cx).is_focused(window)
            || self.search_filter_focus(window, cx)
            || self.search_filters.open
    }

    pub(super) fn search_input_composing(&self, window: &Window, cx: &App) -> bool {
        let search = self.search.read(cx);
        let ext = self.search_filters.extension.read(cx);
        (search.focus_handle(cx).is_focused(window) && search.is_composing())
            || (ext.focus_handle(cx).is_focused(window) && ext.is_composing())
    }

    pub(crate) fn close_search_filters(&mut self, cx: &mut Context<Self>) -> bool {
        if !std::mem::take(&mut self.search_filters.open) {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn dismiss_search_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_search_filters(cx);
        window.focus(&self.search_filters.trigger, cx);
    }

    pub(super) fn observe_search_filter_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let f = &self.search_filters;
        let handles = [
            self.search.focus_handle(cx),
            f.trigger.clone(),
            f.extension.focus_handle(cx),
            f.clear.clone(),
            f.clear_all.clone(),
        ];
        for handle in handles
            .into_iter()
            .chain(f.kinds.iter().chain(&f.modified).cloned())
        {
            self._window_subscriptions
                .push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            self._window_subscriptions
                .push(cx.on_blur(&handle, window, |view, w, cx| {
                    if !view.search.focus_handle(cx).is_focused(w)
                        && !view.search_filter_focus(w, cx)
                    {
                        view.close_search_filters(cx);
                    }
                    cx.notify();
                }));
        }
        self._window_subscriptions
            .push(cx.observe_window_bounds(window, |view, w, cx| {
                let f = &view.search_filters;
                if f.open
                    && f.clear_all.is_focused(w)
                    && SearchTier::for_window(w) != SearchTier::Narrow
                {
                    w.focus(&f.clear, cx);
                }
            }));
    }

    fn toggle_search_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_filters.open {
            self.dismiss_search_filters(window, cx);
            return;
        }
        self.context_menu = None;
        self.close_display_settings(cx);
        self.cancel_scroll_motion();
        self.search_filters.open = true;
        let index = match self.search_filters.criteria.kind() {
            Kind::Any => 0,
            Kind::File => 1,
            Kind::Folder => 2,
        };
        window.focus(&self.search_filters.kinds[index], cx);
        cx.notify();
    }

    pub(super) fn activate_search_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.search_input_composing(window, cx) {
            return true;
        }
        let f = &self.search_filters;
        if f.trigger.is_focused(window) {
            self.toggle_search_filters(window, cx);
        } else if self.filter_popup_focus(window, cx) {
            if let Some(index) = f.kinds.iter().position(|focus| focus.is_focused(window)) {
                self.set_search_kind(KINDS[index].0, window, cx);
            } else if let Some(index) = f.modified.iter().position(|focus| focus.is_focused(window))
            {
                self.set_search_modified_period(MODIFIED_PERIODS[index].0, cx);
            } else if f.clear.is_focused(window) {
                self.clear_search_conditions(window, cx);
            } else if f.clear_all.is_focused(window) {
                self.clear_search(window, cx);
            } else {
                self.close_search_filters(cx);
                window.focus(&self.focus, cx);
            }
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn tab_search_filters(
        &mut self,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.path().is_none() {
            return false;
        }
        if self.search_input_composing(window, cx) {
            return true;
        }
        let f = &self.search_filters;
        let search = self.search.focus_handle(cx);
        let extension = f.extension.focus_handle(cx);
        let order = [
            Some(&search),
            Some(&f.trigger),
            f.open.then_some(&f.kinds[0]),
            f.open.then_some(&f.kinds[1]),
            f.open.then_some(&f.kinds[2]),
            (f.open && f.criteria.kind() != Kind::Folder).then_some(&extension),
            f.open.then_some(&f.modified[0]),
            f.open.then_some(&f.modified[1]),
            f.open.then_some(&f.modified[2]),
            f.open.then_some(&f.modified[3]),
            (f.open && (!f.criteria.is_empty() || f.clear.is_focused(window))).then_some(&f.clear),
            (f.open && SearchTier::for_window(window) == SearchTier::Narrow)
                .then_some(&f.clear_all),
        ];
        let Some(index) = order
            .iter()
            .position(|focus| focus.is_some_and(|focus| focus.is_focused(window)))
        else {
            return false;
        };
        if backward {
            let Some(previous) = order[..index].iter().rev().flatten().next() else {
                return false;
            };
            window.focus(previous, cx);
        } else if let Some(next) = order[index + 1..].iter().flatten().next() {
            window.focus(next, cx);
        } else {
            self.close_search_filters(cx);
            window.focus(&self.display_settings.trigger_focus, cx);
        }
        true
    }

    pub(super) fn render_name_search_field(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tier = SearchTier::for_window(window);
        let narrow = tier == SearchTier::Narrow;
        let focused = self.search.focus_handle(cx).is_focused(window);
        let editing = self.search_filter_context(window, cx);
        let f = &self.search_filters;
        let active = !f.criteria.is_empty();
        // GPUI focuses the trigger on mouse-down. Leave its hitbox in place
        // until the click completes, even when keyboard focus reaches it.
        let chip = active && !focused && !f.open;
        let trigger = self.path().is_some() && (editing || active);
        let leading_trigger = narrow && trigger;
        let show_icon = !(leading_trigger || tier == SearchTier::Medium && chip);
        let anchor = self.search_filters.field_anchor.clone();
        let popup_mounted = self.search_filters.presence.mounted();
        let field = div()
            .id("name-search-field")
            .debug_selector(|| "name-search-field".into())
            .relative()
            .flex()
            .items_center()
            .gap(px(if narrow { 3. } else { 4. }))
            .w(tier.field_width())
            .flex_shrink_0()
            .h(px(36.))
            .px(px(if narrow { 4. } else { 12. }))
            .rounded(px(18.))
            .bg(t.c("surface.field"))
            .border_1()
            .border_color(t.c(if focused {
                "focus.ring"
            } else {
                "border.subtle"
            }))
            .on_click(cx.listener(|view, _, w, cx| {
                w.focus(&view.search.focus_handle(cx), cx);
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        // The popup uses the last prepaint bounds. Reanchor after a resize
                        // even when reduced motion has settled the popup on the first frame.
                        if anchor.replace(bounds) != bounds && popup_mounted {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(show_icon, |d| {
                d.child(
                    svg()
                        .path("icons/search.svg")
                        .size(px(14.))
                        .flex_shrink_0()
                        .text_color(t.c("icon.default")),
                )
            });
        let mut field = if leading_trigger {
            field.child(self.render_search_filter_trigger(t, tier, chip, cx))
        } else {
            field
        };
        // A compact name-only field still has room for its clear button.
        let show_clear = !self.query.is_empty() && !(narrow && (active || leading_trigger));
        let min_input = if narrow && show_clear {
            px(26.)
        } else {
            tier.input_min_width()
        };
        field = field.child(
            div()
                .debug_selector(|| "search-input".into())
                .flex_1()
                .min_w(min_input)
                .child(self.search.clone()),
        );
        if trigger && !leading_trigger {
            field = field.child(self.render_search_filter_trigger(t, tier, chip, cx));
        }
        field.when(show_clear, |d| {
            d.child(
                icon_button("clear-search", "icons/x.svg", true, t)
                    .debug_selector(|| "clear-search".into())
                    .size(px(20.))
                    .flex_shrink_0()
                    .on_click(cx.listener(|view, _, w, cx| {
                        cx.stop_propagation();
                        view.clear_search(w, cx);
                    })),
            )
        })
    }

    fn render_search_filter_trigger(
        &self,
        t: Theme,
        tier: SearchTier,
        chip: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let f = &self.search_filters;
        let count = f.criteria.count();
        let active = count > 0;
        let has_name = !self.query.is_empty();
        let show_remove = chip && tier == SearchTier::Wide;
        let width = px(match (tier, chip, has_name) {
            (SearchTier::Wide, false, _) => 60.,
            (SearchTier::Wide, true, true) => 72.,
            (SearchTier::Wide, true, false) => 96.,
            (SearchTier::Medium, false, _) => 44.,
            (SearchTier::Medium, true, _) => 72.,
            (SearchTier::Narrow, _, _) => 24.,
        });
        let label = match (tier, chip, count) {
            (SearchTier::Narrow, _, 0) => String::new(),
            (SearchTier::Narrow, _, n) => n.to_string(),
            (SearchTier::Medium, true, n) if n > 1 => format!("条件{n}件"),
            (SearchTier::Wide, true, 3) => "条件3件".into(),
            (_, true, _) => condition_summary(&f.criteria, 24),
            (SearchTier::Wide, false, n) if n > 0 => format!("条件{n}"),
            _ => "条件".into(),
        };
        let tooltip = if active {
            condition_summary(&f.criteria, 128)
        } else {
            "検索条件".into()
        };
        let anchor = f.trigger_anchor.clone();
        div()
            .relative()
            .flex()
            .items_center()
            .w(width)
            .h(px(24.))
            .flex_shrink_0()
            .min_w_0()
            .rounded(px(6.))
            .when(active || f.open, |d| {
                d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
            })
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .id("search-filters")
                    .debug_selector(|| "search-filters".into())
                    .track_focus(&f.trigger)
                    .key_context("SearchConditionsTrigger SearchConditionsControl")
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(if tier == SearchTier::Narrow { 1. } else { 3. }))
                    .gap(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .text_size(px(11.))
                    .hover(|s| s.bg(t.c("state.hover")))
                    .focus(|s| s.border_color(t.c("focus.ring")))
                    .tooltip(move |_, cx| {
                        cx.new(|_| Tip {
                            text: tooltip.clone().into(),
                            theme: t,
                        })
                        .into()
                    })
                    .when(!label.is_empty(), |d| {
                        d.child(div().truncate().min_w_0().child(label))
                    })
                    .when(!show_remove, |d| {
                        d.child(
                            svg()
                                .path("icons/chevron-down.svg")
                                .size(px(10.))
                                .flex_shrink_0(),
                        )
                    })
                    .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, w, cx| {
                        let modifiers = event.keystroke.modifiers;
                        if !modifiers.control
                            && !modifiers.platform
                            && !modifiers.alt
                            && matches!(event.keystroke.key.as_str(), "space" | "down")
                        {
                            view.toggle_search_filters(w, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_click(cx.listener(|view, _, w, cx| {
                        cx.stop_propagation();
                        view.toggle_search_filters(w, cx);
                    })),
            )
            .when(show_remove, |d| {
                d.child(
                    icon_button("clear-search-conditions", "icons/x.svg", true, t)
                        .debug_selector(|| "clear-search-conditions".into())
                        .size(px(16.))
                        .flex_shrink_0()
                        .on_click(cx.listener(|view, _, w, cx| {
                            cx.stop_propagation();
                            view.clear_search_conditions(w, cx);
                        })),
                )
            })
    }

    pub(super) fn render_search_filter_popover(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use gpui::anchored;

        let f = &self.search_filters;
        let shown = f.presence.shown()?;
        let (progress, live) = (shown.progress, shown.live);
        let criteria = shown.content;
        let narrow = SearchTier::for_window(window) == SearchTier::Narrow;
        let viewport = window.viewport_size();
        let anchor = f.field_anchor.get();
        let width = px(280.).min((viewport.width - px(16.)).max(px(1.)));
        let (position, height, below) =
            super::display_settings::popover_placement(anchor, width, px(240.), viewport);
        let mut kinds = div().flex().items_center().gap(px(4.));
        for (index, (kind, label)) in KINDS.into_iter().enumerate() {
            kinds = kinds.child(
                filter_choice(("search-kind", index), label, t)
                    .debug_selector(move || format!("search-kind-{index}").into())
                    .when(criteria.kind() == kind, |d| {
                        d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
                    })
                    .when(live, |d| {
                        d.track_focus(&f.kinds[index])
                            .key_context("SearchConditionsControl SearchConditionsOptions")
                            .on_click(cx.listener(move |view, _, w, cx| {
                                w.focus(&view.search_filters.kinds[index], cx);
                                view.set_search_kind(kind, w, cx);
                            }))
                            .on_key_down(cx.listener(
                                move |view, event: &gpui::KeyDownEvent, w, cx| {
                                    let Some(next) = choice_key(event, index, KINDS.len()) else {
                                        return;
                                    };
                                    w.focus(&view.search_filters.kinds[next], cx);
                                    view.set_search_kind(KINDS[next].0, w, cx);
                                    cx.stop_propagation();
                                },
                            ))
                    }),
            );
        }
        let modified = self.render_modified_choices(criteria, live, t, cx);
        let extension_enabled = criteria.kind() != Kind::Folder;
        let extension = div()
            .id("search-extension")
            .debug_selector(|| "search-extension".into())
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .flex()
            .items_center()
            .px(px(8.))
            .rounded(px(6.))
            .bg(t.c("surface.field"))
            .border_1()
            .border_color(
                if live && extension_enabled && f.extension.focus_handle(cx).is_focused(window) {
                    t.c("focus.ring")
                } else {
                    t.c("border.subtle")
                },
            )
            .when(live && extension_enabled, |d| d.child(f.extension.clone()))
            .when(!live || !extension_enabled, |d| {
                let ext = criteria.extension();
                d.text_color(t.c("text.disabled"))
                    .child(if !extension_enabled {
                        "指定できません".into()
                    } else if ext.is_empty() {
                        "例: .pdf".into()
                    } else {
                        format!(".{}", bounded_extension(ext, 40))
                    })
            });
        let clear_enabled = !criteria.is_empty();
        let clear = filter_choice("search-conditions-reset", "条件をクリア", t)
            .debug_selector(|| "search-conditions-reset".into())
            .when(!clear_enabled, |d| d.text_color(t.c("text.disabled")))
            .when(live, |d| {
                d.track_focus(&f.clear).when(clear_enabled, |d| {
                    d.on_click(cx.listener(|view, _, w, cx| {
                        w.focus(&view.search_filters.clear, cx);
                        view.clear_search_conditions(w, cx);
                    }))
                })
            });
        let body = div()
            .id("search-conditions-body")
            .overflow_y_scroll()
            .size_full()
            .px(px(10.))
            .py(px(7.))
            .child(
                div()
                    .h(px(24.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("検索条件"),
            )
            .child(
                div()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("種類")
                    .child(kinds),
            )
            .child(
                div()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child("拡張子")
                    .child(extension),
            )
            .child(modified)
            .child(
                div()
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .child(clear)
                    .when(narrow, |d| {
                        d.child(
                            filter_choice("search-all-reset", "すべて解除", t)
                                .debug_selector(|| "search-all-reset".into())
                                .when(live, |d| {
                                    d.track_focus(&f.clear_all).on_click(cx.listener(
                                        |view, _, w, cx| {
                                            view.clear_search(w, cx);
                                        },
                                    ))
                                }),
                        )
                    }),
            );
        Some(
            deferred(
                anchored().position(position).snap_to_window().child(
                    div()
                        .id("search-conditions-popover")
                        .debug_selector(|| "search-conditions-popover".into())
                        .occlude()
                        .w(width)
                        .h(height)
                        .overflow_hidden()
                        .rounded(px(8.))
                        .bg(t.c("surface.raised"))
                        .border_1()
                        .border_color(t.c("border.dialog"))
                        .shadow_lg()
                        .text_size(px(12.))
                        .when(live, |d| {
                            d.on_mouse_down_out(cx.listener(
                                |view, event: &MouseDownEvent, w, cx| {
                                    if !view
                                        .search_filters
                                        .trigger_anchor
                                        .get()
                                        .contains(&event.position)
                                        && view.close_search_filters(cx)
                                    {
                                        w.focus(&view.focus, cx);
                                    }
                                },
                            ))
                        })
                        .child(body)
                        .when(!live, |d| d.child(motion::ghost_shield()))
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
            .into_any_element(),
        )
    }

    fn render_modified_choices(
        &self,
        criteria: &Filters,
        live: bool,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let selected = criteria.modified().map(ModifiedFilter::period);
        let mut choices = div().flex().flex_wrap().justify_between().gap(px(4.));
        for (index, (period, label)) in MODIFIED_PERIODS.into_iter().enumerate() {
            choices = choices.child(
                filter_choice(("search-modified", index), label, t)
                    .debug_selector(move || format!("search-modified-{index}").into())
                    .w(gpui::relative(0.49))
                    .min_w_0()
                    .px(px(4.))
                    .text_center()
                    .truncate()
                    .when(selected == period, |d| {
                        d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
                    })
                    .when(live, |d| {
                        d.track_focus(&self.search_filters.modified[index])
                            .key_context("SearchConditionsControl SearchConditionsOptions")
                            .on_click(cx.listener(move |view, _, w, cx| {
                                w.focus(&view.search_filters.modified[index], cx);
                                view.set_search_modified_period(period, cx);
                            }))
                            .on_key_down(cx.listener(
                                move |view, event: &gpui::KeyDownEvent, w, cx| {
                                    let Some(next) =
                                        choice_key(event, index, MODIFIED_PERIODS.len())
                                    else {
                                        return;
                                    };
                                    w.focus(&view.search_filters.modified[next], cx);
                                    view.set_search_modified_period(MODIFIED_PERIODS[next].0, cx);
                                    cx.stop_propagation();
                                },
                            ))
                    }),
            );
        }
        div()
            .h(px(84.))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(4.))
            .child("更新日時")
            .child(choices)
    }

    pub(super) fn theme_search_filters(&self, t: Theme, cx: &mut Context<Self>) {
        if self.search_filters.open {
            self.search_filters
                .extension
                .update(cx, |input, _| input.theme = t);
        }
    }
}

fn rebased_modified_filter(criteria: &Filters) -> Option<ModifiedFilter> {
    let old = criteria.modified()?;
    modified::filter_at(old.period(), chrono::Local::now()).filter(|updated| updated != old)
}

fn choice_key(event: &gpui::KeyDownEvent, index: usize, count: usize) -> Option<usize> {
    let modifiers = event.keystroke.modifiers;
    if modifiers.control || modifiers.platform || modifiers.alt || modifiers.shift {
        return None;
    }
    let offset = match event.keystroke.key.as_str() {
        "left" => -1,
        "right" => 1,
        "space" => 0,
        _ => return None,
    };
    Some(index.saturating_add_signed(offset).min(count - 1))
}

fn filter_choice(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .key_context("SearchConditionsControl")
        .px(px(7.))
        .py(px(4.))
        .rounded(px(6.))
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .hover(|s| s.bg(t.c("state.hover")))
        .focus(|s| s.border_color(t.c("focus.ring")))
        .child(label)
}

fn bounded_extension(extension: &str, max_chars: usize) -> String {
    if let Some((end, _)) = extension.char_indices().nth(max_chars) {
        format!("{}…", &extension[..end])
    } else {
        extension.into()
    }
}

fn condition_summary(criteria: &Filters, max_chars: usize) -> String {
    let kind = match criteria.kind() {
        Kind::Any => "",
        Kind::File => "ファイル",
        Kind::Folder => "フォルダー",
    };
    let mut summary = kind.to_owned();
    if !criteria.extension().is_empty() {
        if !summary.is_empty() {
            summary.push('・');
        }
        summary.push('.');
        summary.push_str(&bounded_extension(criteria.extension(), max_chars));
    }
    if let Some(filter) = criteria.modified() {
        let (_, label) = MODIFIED_PERIODS
            .iter()
            .find(|(period, _)| *period == Some(filter.period()))
            .expect("every modified preset has a label");
        if !summary.is_empty() {
            summary.push('・');
        }
        summary.push_str(label);
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, Modifiers};

    fn redraw(visual: &mut gpui::VisualTestContext) {
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }

    fn click(visual: &mut gpui::VisualTestContext, id: &'static str) {
        redraw(visual);
        let point = visual.debug_bounds(id).unwrap().center();
        visual.simulate_click(point, Modifiers::none());
    }

    #[gpui::test]
    fn search_conditions_are_disclosed_only_in_search_context(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bind_keys(cx);
            cx.bind_keys([gpui::KeyBinding::new("down", SelectNext, Some("Explorer"))]);
        });
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| w.focus(&view.focus, cx))
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("search-filters").is_none());
        let field = visual.debug_bounds("name-search-field").unwrap();
        visual.simulate_click(
            gpui::point(field.left() + px(6.), field.center().y),
            Modifiers::none(),
        );
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("search-filters").is_some());
        assert!(visual.debug_bounds("search-extension").is_none());
        visual.simulate_keystrokes("tab down");
        window
            .read_with(&visual, |view, _| assert!(view.search_filters.open))
            .unwrap();
    }

    #[gpui::test]
    fn filter_controls_update_direct_results_and_clear_the_search(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            bind_keys(cx);
            cx.bind_keys([
                gpui::KeyBinding::new("enter", OpenSelected, Some("Explorer")),
                gpui::KeyBinding::new("escape", ClearSearch, Some("Explorer")),
            ]);
        });
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| {
                let root = view.path().unwrap();
                view.entries = [
                    ("report.PDF", EntryKind::File),
                    ("report.txt", EntryKind::File),
                    ("draft.pdf", EntryKind::Dir),
                ]
                .into_iter()
                .map(|(name, kind)| {
                    Entry::virtual_entry(name.into(), root.join(name), kind, None, None)
                })
                .collect();
                // Leave the descendant timer unscheduled. Its worker has a separate fixture.
                view.loading = true;
                view.rebuild_visible(None);
                w.focus(&view.search.focus_handle(cx), cx);
                view.search.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "日本語", Some(3..3), w, cx);
                });
                cx.notify();
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("tab");
        window
            .update(&mut visual, |view, w, cx| {
                assert!(view.search.focus_handle(cx).is_focused(w));
                assert!(!view.search_filters.open);
                view.search.update(cx, |input, cx| input.reset(w, cx));
            })
            .unwrap();
        visual.simulate_keystrokes("tab enter tab space right left");
        window
            .read_with(&visual, |view, _| {
                assert_eq!(view.search_filters.criteria.kind(), Kind::File);
            })
            .unwrap();
        click(&mut visual, "search-extension");
        visual.simulate_keystrokes("p d f");
        window
            .update(&mut visual, |view, w, cx| {
                assert!(view.recursive_search());
                assert_eq!(view.visible, [0]);
                assert!(matches!(
                    view.display_rows.first(),
                    Some(grouping::DisplayRow::SearchHeader(true))
                ));
                view.search_filters.extension.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "ｘ", Some(1..1), w, cx);
                });
            })
            .unwrap();
        visual.simulate_keystrokes("tab enter escape");
        window
            .update(&mut visual, |view, w, cx| {
                assert!(view.search_filters.extension.focus_handle(cx).is_focused(w));
                assert_eq!(view.visible, [0]);
                assert!(view.search_filters.open);
            })
            .unwrap();
        click(&mut visual, "search-kind-2");
        window
            .update(&mut visual, |view, _, cx| {
                assert_eq!(view.visible, [2]);
                assert!(view.search_filters.extension.read(cx).content.is_empty());
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        window
            .update(&mut visual, |view, w, _| {
                assert!(!view.search_filters.open);
                assert!(view.search_filters.trigger.is_focused(w));
                assert_eq!(view.visible, [2]);
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        window
            .update(&mut visual, |view, _, _| {
                assert!(!view.recursive_search());
                assert_eq!(view.visible, [0, 1, 2]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn edits_rebase_stale_dates_before_rebuilding_direct_results(cx: &mut gpui::TestAppContext) {
        let window = super::super::tests::workflow_window(cx);
        let stale = window
            .update(cx, |view, _, cx| {
                let now = std::time::SystemTime::now();
                let old = now - std::time::Duration::from_secs(9 * 86_400);
                let stale = Filters::default().with_modified(Some(ModifiedFilter::new(
                    ModifiedPeriod::LastSevenDays,
                    old..old + std::time::Duration::from_secs(7 * 86_400),
                )));
                let root = view.path().unwrap();
                view.entries = vec![Entry::virtual_entry(
                    "report.pdf".into(),
                    root.join("report.pdf"),
                    EntryKind::File,
                    None,
                    Some(now),
                )];
                view.loading = true;
                view.search_filters.criteria = stale.clone();
                view.rebuild_visible(None);
                assert!(view.visible.is_empty());
                view.apply_search_filters(stale.clone().with_extension("pdf"), cx);
                assert_eq!(view.visible, [0]);
                stale
            })
            .unwrap();
        window
            .update(cx, |view, w, cx| {
                view.search_filters.criteria = stale;
                view.query = "rep".into();
                view.rebuild_visible(None);
                assert!(view.visible.is_empty());
                view.search.update(cx, |input, cx| {
                    input.set_text("report".into(), false, w, cx)
                });
            })
            .unwrap();
        window
            .read_with(cx, |view, _| assert_eq!(view.visible, [0]))
            .unwrap();
    }

    #[gpui::test]
    fn modified_period_can_filter_without_name_and_survives_extension_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| {
                let now = std::time::SystemTime::now();
                let root = view.path().unwrap();
                view.entries = [
                    ("recent.pdf", Some(now)),
                    (
                        "old.pdf",
                        Some(now - std::time::Duration::from_secs(9 * 86_400)),
                    ),
                    ("recent.txt", Some(now)),
                    ("unknown.pdf", None),
                ]
                .into_iter()
                .map(|(name, time)| {
                    Entry::virtual_entry(name.into(), root.join(name), EntryKind::File, None, time)
                })
                .collect();
                view.loading = true;
                view.rebuild_visible(None);
                view.toggle_search_filters(w, cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        click(&mut visual, "search-modified-2");
        window
            .read_with(&visual, |view, _| {
                assert!(view.recursive_search());
                assert!(view.query.is_empty());
                assert_eq!(view.visible, [0, 2]);
            })
            .unwrap();
        click(&mut visual, "search-extension");
        visual.simulate_keystrokes("p d f");
        window
            .read_with(&visual, |view, _| {
                assert_eq!(view.visible, [0]);
                assert_eq!(view.search_filters.criteria.count(), 2);
            })
            .unwrap();
        visual.simulate_keystrokes("tab space");
        window
            .read_with(&visual, |view, _| {
                assert_eq!(view.visible, [0, 1, 3]);
                assert_eq!(view.search_filters.criteria.extension(), "pdf");
            })
            .unwrap();
    }

    #[gpui::test]
    fn applied_conditions_keep_input_room_and_clear_independently(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| {
                view.loading = true;
                view.search.update(cx, |input, cx| {
                    input.set_text("report".into(), false, w, cx)
                });
                view.apply_search_filters(Filters::from_input(Kind::File, "．ＰＤＦ"), cx);
                w.focus(&view.focus, cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for (width, min_input) in [(500., 39.), (800., 56.), (1200., 112.)] {
            visual.simulate_resize(gpui::size(px(width), px(650.)));
            redraw(&mut visual);
            assert!(visual.debug_bounds("search-input").unwrap().size.width >= px(min_input));
            let list = window
                .read_with(&visual, |view, _| {
                    view.scroll.0.borrow().base_handle.bounds()
                })
                .unwrap();
            click(&mut visual, "search-filters");
            let popup = visual.debug_bounds("search-conditions-popover").unwrap();
            assert!(popup.left() >= px(8.) && popup.right() <= px(width - 8.));
            assert!(popup.top() >= px(8.) && popup.bottom() <= px(642.));
            assert_eq!(
                window
                    .read_with(&visual, |view, _| view
                        .scroll
                        .0
                        .borrow()
                        .base_handle
                        .bounds())
                    .unwrap(),
                list
            );
            assert_eq!(visual.update(|w, cx| w.simulate_next_frame(cx)), 0);
            click(&mut visual, "search-filters");
            assert!(visual.debug_bounds("search-conditions-popover").is_none());
            window
                .update(&mut visual, |view, w, cx| w.focus(&view.focus, cx))
                .unwrap();
        }
        click(&mut visual, "clear-search-conditions");
        window
            .update(&mut visual, |view, w, _| {
                assert!(view.search_filters.criteria.is_empty());
                assert_eq!(view.query, "report");
                assert!(view.focus.is_focused(w));
            })
            .unwrap();
        click(&mut visual, "clear-search");
        window
            .read_with(&visual, |view, _| assert!(!view.search_active()))
            .unwrap();
    }

    #[gpui::test]
    fn name_free_summary_fits_and_keeps_its_hitbox_when_pressed(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| {
                view.loading = true;
                view.apply_search_filters(Filters::from_input(Kind::File, "pdf"), cx);
                w.focus(&view.focus, cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for width in [800., 1200.] {
            visual.simulate_resize(gpui::size(px(width), px(650.)));
            redraw(&mut visual);
            let before = visual.debug_bounds("search-filters").unwrap();
            let left_edge = gpui::point(before.left() + px(6.), before.center().y);
            visual.simulate_mouse_down(left_edge, MouseButton::Left, Modifiers::none());
            redraw(&mut visual);
            assert_eq!(visual.debug_bounds("search-filters"), Some(before));
            visual.simulate_mouse_up(left_edge, MouseButton::Left, Modifiers::none());
            window
                .update(&mut visual, |view, w, cx| {
                    assert!(view.search_filters.open);
                    view.close_search_filters(cx);
                    w.focus(&view.focus, cx);
                })
                .unwrap();
        }
        redraw(&mut visual);
        let field = visual.debug_bounds("name-search-field").unwrap();
        let clear = visual.debug_bounds("clear-search-conditions").unwrap();
        assert!(clear.right() <= field.right() - px(12.));
        assert!(visual.debug_bounds("search-input").unwrap().size.width >= px(112.));
    }

    #[gpui::test]
    fn window_deactivation_returns_panel_focus_to_the_trigger(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            cx.bind_keys([gpui::KeyBinding::new(
                "escape",
                ClearSearch,
                Some("Explorer"),
            )]);
        });
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |view, w, cx| {
                view.loading = true;
                view.apply_search_filters(Filters::from_input(Kind::File, "pdf"), cx);
                view.toggle_search_filters(w, cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        click(&mut visual, "search-extension");
        visual.deactivate_window();
        window
            .update(&mut visual, |view, w, _| {
                assert!(!view.search_filters.open);
                assert!(view.search_filters.trigger.is_focused(w));
                w.activate_window();
            })
            .unwrap();
        redraw(&mut visual);
        visual.simulate_keystrokes("escape");
        window
            .read_with(&visual, |view, _| assert!(!view.search_active()))
            .unwrap();
    }

    #[gpui::test]
    fn widening_the_window_keeps_focus_inside_the_conditions_panel(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = super::super::tests::workflow_window(cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(500.), px(650.)));
        window
            .update(&mut visual, |view, w, cx| {
                view.toggle_search_filters(w, cx);
                w.focus(&view.search_filters.clear_all, cx);
            })
            .unwrap();
        redraw(&mut visual);
        assert!(visual.debug_bounds("search-all-reset").is_some());
        visual.simulate_resize(gpui::size(px(800.), px(650.)));
        redraw(&mut visual);
        assert!(visual.debug_bounds("search-all-reset").is_none());
        window
            .update(&mut visual, |view, w, _| {
                assert!(view.search_filters.open);
                assert!(view.search_filters.clear.is_focused(w));
            })
            .unwrap();
        visual.simulate_keystrokes("tab");
        window
            .read_with(&visual, |view, _| assert!(!view.search_filters.open))
            .unwrap();
    }

    #[gpui::test]
    fn exiting_conditions_are_inert_and_reopen_from_painted_progress(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(false));
        let window = super::super::tests::workflow_window(cx);
        let (initial_key, applied_key) = window
            .update(cx, |view, w, cx| {
                view.loading = true;
                let before = view.search_key(cx);
                view.apply_search_filters(Filters::from_input(Kind::File, "pdf"), cx);
                w.focus(&view.search.focus_handle(cx), cx);
                (before, view.search_key(cx))
            })
            .unwrap();
        assert_ne!(initial_key, applied_key);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        click(&mut visual, "search-filters");
        fn settle(visual: &mut gpui::VisualTestContext) {
            for _ in 0..40 {
                redraw(visual);
                if visual.update(|w, cx| w.simulate_next_frame(cx)) == 0 {
                    return;
                }
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(16));
            }
            panic!("conditions popup motion did not settle");
        }
        settle(&mut visual);
        click(&mut visual, "search-filters");
        redraw(&mut visual);
        let stale = visual
            .debug_bounds("search-conditions-reset")
            .unwrap()
            .center();
        assert!(visual.debug_bounds("motion-ghost-shield").is_some());
        visual.simulate_click(stale, Modifiers::none());
        assert!(
            window
                .read_with(&visual, |view, _| !view.search_filters.criteria.is_empty())
                .unwrap()
        );
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(32));
        visual.update(|w, cx| w.simulate_next_frame(cx));
        redraw(&mut visual);
        let ghost_progress = window
            .read_with(&visual, |view, _| {
                let shown = view.search_filters.presence.shown().unwrap();
                assert!(!shown.live);
                shown.progress
            })
            .unwrap();
        assert!(ghost_progress > 0. && ghost_progress < 1.);
        click(&mut visual, "search-filters");
        redraw(&mut visual);
        window
            .update(&mut visual, |view, _, cx| {
                let shown = view.search_filters.presence.shown().unwrap();
                // The deterministic Presence tests cover the exact reversal position. This
                // visual test allows intervening paint events dispatched by the click.
                assert!(shown.live && shown.progress > 0. && shown.progress < 1.);
                assert_eq!(view.search_key(cx), applied_key);
            })
            .unwrap();
        settle(&mut visual);
        click(&mut visual, "display-settings-trigger");
        window
            .read_with(&visual, |view, _| {
                assert!(!view.search_filters.open && view.display_settings.open);
            })
            .unwrap();
        settle(&mut visual);
        assert!(visual.debug_bounds("search-conditions-popover").is_none());
    }
}
