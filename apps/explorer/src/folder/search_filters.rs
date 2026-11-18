use super::*;
use explorer_core::name_search::{Filters, Kind};

const KINDS: [(Kind, &str); 3] = [
    (Kind::Any, "すべて"),
    (Kind::File, "ファイル"),
    (Kind::Folder, "フォルダー"),
];

pub(super) struct SearchFilters {
    pub(super) criteria: Filters,
    open: bool,
    trigger: FocusHandle,
    kinds: [FocusHandle; 3],
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
                let criteria =
                    Filters::from_input(view.search_filters.criteria.kind(), &input.content);
                view.apply_search_filters(criteria, cx);
            }
        })
        .detach();
        Self {
            criteria: Filters::default(),
            open: false,
            trigger: cx.focus_handle(),
            kinds: std::array::from_fn(|_| cx.focus_handle()),
            extension,
        }
    }
}

impl FolderView {
    pub(super) fn apply_search_filters(&mut self, criteria: Filters, cx: &mut Context<Self>) {
        if self.path().is_none() || self.search_filters.criteria == criteria {
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

    pub(super) fn reset_search_filters(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.search_filters.open = false;
        self.search_filters.criteria = Filters::default();
        self.search_filters
            .extension
            .update(cx, |input, cx| input.reset(window, cx));
    }

    pub(super) fn search_filter_focus(&self, window: &Window, cx: &App) -> bool {
        let filters = &self.search_filters;
        filters.trigger.is_focused(window)
            || (filters.open
                && (filters.kinds.iter().any(|focus| focus.is_focused(window))
                    || filters.extension.focus_handle(cx).is_focused(window)))
    }

    fn toggle_search_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_filters.open = !self.search_filters.open;
        let focus = if self.search_filters.open {
            let kind = self.search_filters.criteria.kind();
            let index = KINDS
                .iter()
                .position(|(candidate, _)| *candidate == kind)
                .unwrap();
            &self.search_filters.kinds[index]
        } else {
            &self.search_filters.trigger
        };
        window.focus(focus, cx);
        cx.notify();
    }

    pub(super) fn activate_search_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.search_filters.trigger.is_focused(window) {
            self.toggle_search_filters(window, cx);
        } else if self.search_filters.open {
            if let Some(index) = self
                .search_filters
                .kinds
                .iter()
                .position(|focus| focus.is_focused(window))
            {
                self.set_search_kind(KINDS[index].0, window, cx);
            } else if self
                .search_filters
                .extension
                .focus_handle(cx)
                .is_focused(window)
            {
                if !self.search_filters.extension.read(cx).is_composing() {
                    window.focus(&self.focus, cx);
                }
            } else {
                return false;
            }
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn tab_search_filters(
        &self,
        backward: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self.path().is_none() {
            return false;
        }
        let filters = &self.search_filters;
        let search = self.search.focus_handle(cx);
        let extension = filters.extension.focus_handle(cx);
        if (search.is_focused(window) && self.search.read(cx).is_composing())
            || (filters.open
                && extension.is_focused(window)
                && filters.extension.read(cx).is_composing())
        {
            // Keep the IME's focus if the platform forwards Tab to the view.
            return true;
        }
        let show_extension = filters.criteria.kind() != Kind::Folder;
        let next = if search.is_focused(window) && !backward {
            &filters.trigger
        } else if filters.trigger.is_focused(window) {
            if backward {
                &search
            } else if filters.open {
                &filters.kinds[0]
            } else {
                &self.display_settings.trigger_focus
            }
        } else if filters.open {
            if let Some(index) = filters
                .kinds
                .iter()
                .position(|focus| focus.is_focused(window))
            {
                if backward {
                    index
                        .checked_sub(1)
                        .map_or(&filters.trigger, |i| &filters.kinds[i])
                } else {
                    filters.kinds.get(index + 1).unwrap_or(if show_extension {
                        &extension
                    } else {
                        &self.display_settings.trigger_focus
                    })
                }
            } else if show_extension && extension.is_focused(window) {
                if backward {
                    &filters.kinds[2]
                } else {
                    &self.display_settings.trigger_focus
                }
            } else {
                return false;
            }
        } else {
            return false;
        };
        window.focus(next, cx);
        true
    }

    pub(super) fn render_search_filter_trigger(
        &self,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let filters = &self.search_filters;
        let active = !filters.criteria.is_empty();
        div()
            .id("search-filters")
            .debug_selector(|| "search-filters".into())
            .track_focus(&filters.trigger)
            .flex()
            .items_center()
            .h(px(32.))
            .px(px(7.))
            .flex_shrink_0()
            .rounded(px(6.))
            .border_1()
            .border_color(t.c("border.subtle"))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(t.c("state.hover")))
            .focus(|s| s.border_color(t.c("focus.ring")))
            .when(active || filters.open, |d| {
                d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
            })
            .child(if active {
                "絞り込み ✓"
            } else {
                "絞り込み"
            })
            .on_click(cx.listener(|view, _, w, cx| view.toggle_search_filters(w, cx)))
    }

    pub(super) fn render_search_filter_bar(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let filters = &self.search_filters;
        if !filters.open || self.path().is_none() {
            return None;
        }
        let mut kinds = div().flex().items_center().gap(px(4.));
        for (index, (kind, label)) in KINDS.into_iter().enumerate() {
            kinds = kinds.child(
                div()
                    .id(("search-kind", index))
                    .debug_selector(move || format!("search-kind-{index}").into())
                    .track_focus(&filters.kinds[index])
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .hover(|s| s.bg(t.c("state.hover")))
                    .focus(|s| s.border_color(t.c("focus.ring")))
                    .when(filters.criteria.kind() == kind, |d| {
                        d.bg(t.c("accent.soft")).text_color(t.c("accent.fg"))
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, w, cx| {
                        w.focus(&view.search_filters.kinds[index], cx);
                        view.set_search_kind(kind, w, cx);
                    })),
            );
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .pb(px(6.))
                .flex_shrink_0()
                .text_size(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child("種類")
                        .child(kinds),
                )
                .when(filters.criteria.kind() != Kind::Folder, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child("拡張子")
                            .child(
                                div()
                                    .id("search-extension")
                                    .debug_selector(|| "search-extension".into())
                                    .w(px(112.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(6.))
                                    .bg(t.c("surface.field"))
                                    .border_1()
                                    .border_color(
                                        if filters.extension.focus_handle(cx).is_focused(window) {
                                            t.c("focus.ring")
                                        } else {
                                            t.c("border.subtle")
                                        },
                                    )
                                    .child(filters.extension.clone()),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    pub(super) fn theme_search_filters(&self, t: Theme, cx: &mut Context<Self>) {
        if self.search_filters.open {
            self.search_filters
                .extension
                .update(cx, |input, _| input.theme = t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, Modifiers};

    #[gpui::test]
    fn filter_controls_update_direct_results_and_clear_the_search(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
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
        fn click(visual: &mut gpui::VisualTestContext, id: &'static str) {
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let point = visual.debug_bounds(id).unwrap().center();
            visual.simulate_click(point, Modifiers::none());
        }
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("tab");
        window
            .update(&mut visual, |view, w, cx| {
                assert!(view.search.focus_handle(cx).is_focused(w));
                assert!(!view.search_filters.open);
                view.search.update(cx, |input, cx| input.reset(w, cx));
            })
            .unwrap();
        visual.simulate_keystrokes("tab enter");
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
        visual.simulate_keystrokes("tab");
        window
            .update(&mut visual, |view, w, cx| {
                assert!(view.search_filters.extension.focus_handle(cx).is_focused(w));
                assert_eq!(view.visible, [0]);
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
            .update(&mut visual, |view, _, _| {
                assert!(!view.recursive_search());
                assert_eq!(view.visible, [0, 1, 2]);
            })
            .unwrap();
    }
}
