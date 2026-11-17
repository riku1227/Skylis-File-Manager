use super::*;
use gpui::{
    Bounds, ElementInputHandler, EntityInputHandler, Point, UTF16Selection, canvas, point, size,
};
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

gpui::actions!(
    explorer,
    [
        ExtendUp,
        ExtendDown,
        ExtendLeft,
        ExtendRight,
        ExtendHome,
        ExtendEnd,
        ExtendPageUp,
        ExtendPageDown
    ]
);
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("shift-up", ExtendUp, Some("Explorer")),
        gpui::KeyBinding::new("shift-down", ExtendDown, Some("Explorer")),
        gpui::KeyBinding::new("shift-left", ExtendLeft, Some("Explorer")),
        gpui::KeyBinding::new("shift-right", ExtendRight, Some("Explorer")),
        gpui::KeyBinding::new("shift-home", ExtendHome, Some("Explorer")),
        gpui::KeyBinding::new("shift-end", ExtendEnd, Some("Explorer")),
        gpui::KeyBinding::new("shift-pageup", ExtendPageUp, Some("Explorer")),
        gpui::KeyBinding::new("shift-pagedown", ExtendPageDown, Some("Explorer")),
    ]);
}

#[derive(Default)]
pub(super) struct Typeahead {
    prefix: String,
    last: Option<Instant>,
    pub(super) composition: String,
    selection: Range<usize>,
}
impl Typeahead {
    fn push(&mut self, text: &str, now: Instant) -> Option<(String, bool)> {
        if text.is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
            return None;
        }
        let text = text.to_lowercase();
        if text.len() > 256 {
            return None;
        }
        let expired = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(800));
        let cycle = expired || (self.prefix == text && text.graphemes(true).count() == 1);
        if expired {
            self.prefix.clear();
        }
        if !cycle && self.prefix.len() + text.len() > 256 {
            return None;
        }
        if cycle {
            self.prefix = text;
        } else {
            self.prefix.push_str(&text);
        }
        self.last = Some(now);
        Some((self.prefix.clone(), cycle))
    }
    fn replacement(&self, range: Option<Range<usize>>, text: &str) -> Option<String> {
        if text.len() > 1024 || text.chars().any(char::is_control) {
            return None;
        }
        let range = range.unwrap_or(0..self.composition.encode_utf16().count());
        let start = crate::search::utf16_to_utf8(&self.composition, range.start);
        let end = crate::search::utf16_to_utf8(&self.composition, range.end).max(start);
        if self.composition.len() - (end - start) + text.len() > 1024 {
            return None;
        }
        Some(format!(
            "{}{}{}",
            &self.composition[..start],
            text,
            &self.composition[end..]
        ))
    }
}
pub(super) enum Direction {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl FolderView {
    pub(super) fn reset_blocked_typeahead(&mut self, w: &Window, cx: &mut Context<Self>) {
        if !self.typing_target(w, cx)
            && (!self.typeahead.composition.is_empty() || !self.typeahead.prefix.is_empty())
        {
            self.typeahead = Default::default();
            w.notify_text_input_changed(&self.focus);
            cx.notify();
        }
    }
    fn typing_target(&self, w: &Window, cx: &App) -> bool {
        cx.active_window() == Some(w.window_handle())
            && self.focus.is_focused(w)
            && !self.is_home()
            && !self.preview_open
            && self.context_menu.is_none()
            && !self.display_settings.open
            && self.name_editor.is_none()
            && self.archive_dialog.is_none()
            && !self.marquee_in_progress()
            && !cx.has_active_drag()
    }
    fn commit_typeahead(&mut self, text: &str, w: &mut Window, cx: &mut Context<Self>) {
        if !self.typing_target(w, cx) {
            return;
        }
        let Some((prefix, cycle)) = self.typeahead.push(text, cx.background_executor().now())
        else {
            return;
        };
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        let start = if cycle {
            self.selected.map_or(0, |i| (i + 1) % len)
        } else {
            self.selected.unwrap_or(0).min(len - 1)
        };
        let found = (0..len).map(|i| (start + i) % len).find(|&i| {
            let mut name = self.entries[self.visible[i]]
                .name
                .chars()
                .flat_map(char::to_lowercase);
            prefix.chars().all(|c| name.next() == Some(c))
        });
        if let Some(index) = found {
            let state = std::mem::take(&mut self.typeahead);
            self.cancel_scroll_motion();
            self.select(index, cx);
            self.typeahead = state;
        }
    }
    pub(super) fn extend_selection_to(
        &mut self,
        target: usize,
        w: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focused(w, cx)
            || self.is_home()
            || self.preview_open
            || self.marquee_in_progress()
            || self.context_menu.is_some()
            || self.visible.is_empty()
        {
            return;
        }
        let target = target.min(self.visible.len() - 1);
        let anchor = self
            .selection_anchor
            .as_ref()
            .and_then(|p| {
                self.visible
                    .iter()
                    .position(|&i| self.entries[i].path == *p)
            })
            .unwrap_or(self.selected.unwrap_or(target).min(self.visible.len() - 1));
        self.typeahead = Default::default();
        self.cancel_scroll_motion();
        self.preview_scope.cancel();
        self.selection_anchor = Some(self.entries[self.visible[anchor]].path.clone());
        self.multi_selection = (anchor.min(target)..=anchor.max(target))
            .map(|i| self.entries[self.visible[i]].path.clone())
            .collect();
        self.selected = Some(target);
        self.scroll_to_entry(target, ScrollStrategy::Nearest);
        cx.notify();
    }
    pub(super) fn extend_key(&mut self, direction: Direction, w: &Window, cx: &mut Context<Self>) {
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        let current = self.selected.unwrap_or(0).min(len - 1);
        let step = if self.gallery {
            self.gallery_columns.max(1)
        } else {
            1
        };
        let target = if self.selected.is_none() {
            match direction {
                Direction::Up | Direction::Left | Direction::End | Direction::PageUp => len - 1,
                _ => 0,
            }
        } else {
            match direction {
                Direction::Up => current.saturating_sub(step),
                Direction::Down => current.saturating_add(step).min(len - 1),
                Direction::Left => current.saturating_sub(1),
                Direction::Right => current.saturating_add(1).min(len - 1),
                Direction::Home => 0,
                Direction::End => len - 1,
                Direction::PageUp => current.saturating_sub(self.page_rows() as usize),
                Direction::PageDown => current
                    .saturating_add(self.page_rows() as usize)
                    .min(len - 1),
            }
        };
        self.extend_selection_to(target, w, cx);
    }
    pub(super) fn typeahead_input(&self, w: &Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.typing_target(w, cx) {
            return div().into_any_element();
        }
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, w, cx| {
                        w.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(!self.typeahead.composition.is_empty(), |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(220.))
                        .bottom(px(35.))
                        .px(px(8.))
                        .py(px(4.))
                        .bg(self.theme(w).c("surface.raised"))
                        .child(self.typeahead.composition.clone()),
                )
            })
            .into_any_element()
    }
}

impl EntityInputHandler for FolderView {
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.typeahead.composition.encode_utf16().count())
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = crate::search::utf16_to_utf8(&self.typeahead.composition, range.start);
        let end = crate::search::utf16_to_utf8(&self.typeahead.composition, range.end).max(start);
        self.typeahead.selection = crate::search::utf8_to_utf16(&self.typeahead.composition, start)
            ..crate::search::utf8_to_utf16(&self.typeahead.composition, end);
        w.notify_text_selection_changed(&self.focus);
        cx.notify();
    }
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = crate::search::utf16_to_utf8(&self.typeahead.composition, range.start);
        let end = crate::search::utf16_to_utf8(&self.typeahead.composition, range.end).max(start);
        *actual = Some(
            crate::search::utf8_to_utf16(&self.typeahead.composition, start)
                ..crate::search::utf8_to_utf16(&self.typeahead.composition, end),
        );
        Some(self.typeahead.composition[start..end].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.typeahead.selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.typeahead.composition.is_empty())
            .then(|| 0..self.typeahead.composition.encode_utf16().count())
    }
    fn unmark_text(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.typeahead.composition);
        self.typeahead.selection = 0..0;
        self.commit_typeahead(&text, w, cx);
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.typing_target(w, cx) {
            self.reset_blocked_typeahead(w, cx);
            return;
        }
        let Some(text) = self.typeahead.replacement(range, text) else {
            return;
        };
        self.typeahead.composition.clear();
        self.typeahead.selection = 0..0;
        self.commit_typeahead(&text, w, cx);
        w.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.typing_target(w, cx) {
            self.reset_blocked_typeahead(w, cx);
            return;
        }
        let insertion = range.as_ref().map_or(0, |r| {
            r.start
                .min(self.typeahead.composition.encode_utf16().count())
        });
        let inserted_len = text.encode_utf16().count();
        let Some(text) = self.typeahead.replacement(range, text) else {
            return;
        };
        let len = text.encode_utf16().count();
        self.typeahead.composition = text;
        let selection = selection.unwrap_or(inserted_len..inserted_len);
        self.typeahead.selection = (insertion + selection.start.min(inserted_len)).min(len)
            ..(insertion + selection.end.min(inserted_len)).min(len);
        w.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(
            point(bounds.left() + px(220.), bounds.bottom() - px(35.)),
            size(px(1.), px(20.)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.typeahead.composition.encode_utf16().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn preview_transition_discards_stale_composition_during_cancel_and_paint(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.replace_and_mark_text_in_range(None, "未確定", None, w, cx);
                assert!(!v.typeahead.composition.is_empty());
                v.preview_open = true;
                v.replace_text_in_range(None, "", w, cx);
                assert!(v.typeahead.composition.is_empty());
                v.preview_open = false;
                v.replace_and_mark_text_in_range(None, "再変換", None, w, cx);
                v.preview_open = true;
                cx.notify();
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        window
            .update(&mut visual, |v, w, cx| {
                assert!(v.typeahead.composition.is_empty());
                assert_eq!(v.typeahead.selection, 0..0);
                assert!(!v.input_focused(w, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn real_input_handler_and_shift_bindings_jump_and_extend(cx: &mut gpui::TestAppContext) {
        cx.update(bind_keys);
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |v, w, cx| {
                v.entries = super::super::tests::workflow_entries(&[
                    "file_open.rs",
                    "file_operations.rs",
                    "folder.rs",
                ]);
                v.visible = vec![0, 1, 2];
                v.rebuild_display_rows();
                v.select(0, cx);
                w.focus(&v.focus, cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_input("fo");
        window
            .read_with(&visual, |v, _| assert_eq!(v.selected, Some(2)))
            .unwrap();
        visual.simulate_keystrokes("shift-up shift-up shift-down");
        window
            .read_with(&visual, |v, _| {
                assert_eq!(v.selected, Some(1));
                assert_eq!(v.selected_paths().len(), 2);
            })
            .unwrap();
    }

    #[gpui::test]
    fn japanese_and_surrogate_composition_cancel_and_commit(cx: &mut gpui::TestAppContext) {
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |v, w, cx| {
                let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("日本😀.txt");
                v.entries = vec![Entry::virtual_entry(
                    "日本😀.txt".into(),
                    path,
                    EntryKind::File,
                    Some(0),
                    None,
                )];
                v.visible = vec![0];
                w.focus(&v.focus, cx);
                let destination =
                    Location::Folder(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("other"));
                v.history.push(destination.clone(), None);
                v.replace_and_mark_text_in_range(None, "日本😀", Some(4..4), w, cx);
                v.go_back(&GoBack, w, cx);
                assert_eq!(v.history.current(), &destination);
                assert_eq!(v.text_length_utf16(w, cx), Some(4));
                assert_eq!(v.selected, None);
                v.replace_and_mark_text_in_range(None, "", None, w, cx);
                v.unmark_text(w, cx);
                assert_eq!(v.selected, None);
                v.replace_text_in_range(None, "日本😀", w, cx);
                assert_eq!(v.selected, Some(0));
            })
            .unwrap();
    }

    #[test]
    fn prefixes_extend_repeat_and_expire_without_unbounded_input() {
        let now = std::time::Instant::now();
        let mut state = Typeahead::default();
        assert_eq!(state.push("R", now), Some(("r".into(), true)));
        assert_eq!(state.push("r", now), Some(("r".into(), true)));
        assert_eq!(state.push("e", now), Some(("re".into(), false)));
        assert_eq!(
            state.push("日", now + std::time::Duration::from_secs(1)),
            Some(("日".into(), true))
        );
        assert_eq!(state.push(&"a".repeat(300), now), None);
        assert_eq!(state.push(&"İ".repeat(100), now), None);
        assert_eq!(state.push("\n", now), None);
    }

    #[gpui::test]
    fn range_selection_grows_shrinks_and_keeps_the_anchor(cx: &mut gpui::TestAppContext) {
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |v, w, cx| {
                v.history =
                    History::new(Location::Folder(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
                v.entries = super::super::tests::workflow_entries(&[
                    "file_open.rs",
                    "file_operations.rs",
                    "folder.rs",
                ]);
                v.visible = vec![0, 1, 2];
                v.selected = None;
                v.extend_key(Direction::Down, w, cx);
                assert_eq!(v.selected, Some(0));
                v.select(0, cx);
                v.extend_selection_to(2, w, cx);
                assert_eq!(v.selected_paths().len(), 3);
                v.extend_selection_to(1, w, cx);
                assert_eq!(v.selected_paths().len(), 2);
                assert_eq!(v.selected, Some(1));
                v.extend_selection_to(0, w, cx);
                assert_eq!(v.selected_paths().len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_only_jumps_after_commit_and_never_filters_the_list(cx: &mut gpui::TestAppContext) {
        use gpui::EntityInputHandler;
        let window = super::super::tests::workflow_window(cx);
        window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.history =
                    History::new(Location::Folder(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
                v.entries = super::super::tests::workflow_entries(&[
                    "file_open.rs",
                    "file_operations.rs",
                    "folder.rs",
                ]);
                v.visible = vec![0, 1, 2];
                v.select(0, cx);
                v.replace_and_mark_text_in_range(None, "folder", None, w, cx);
                assert_eq!(v.selected, Some(0));
                v.replace_text_in_range(None, "folder", w, cx);
                assert_eq!(v.selected, Some(2));
                assert_eq!(v.visible.len(), 3);
                w.focus(&v.search.focus_handle(cx), cx);
                v.replace_text_in_range(None, "file", w, cx);
                assert_eq!(v.selected, Some(2));
            })
            .unwrap();
    }
}
