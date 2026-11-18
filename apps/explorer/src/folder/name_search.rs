use super::*;
use crate::name_search::{self as service, Key, Message, Mode};
use explorer_core::name_search::{End, Event, Hit, Limit, Locality, Progress};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct SearchState {
    epoch: u64,
    generation: u64,
    pub(super) composing: bool,
    locality: Option<Locality>,
    pub hits: Vec<Arc<Hit>>,
    pub selected: Option<usize>,
    pub status: String,
    pub progress: Progress,
    ticket: Option<service::Ticket>,
    debounce: Option<Task<()>>,
    receive: Option<Task<()>>,
    pub stopped: bool,
    needs_start: bool,
    active: bool,
    visible: bool,
    pub(super) open_cancel: CancelToken,
    pub(super) open_pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::EntityInputHandler;
    fn fixture(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
        let w = super::super::tests::workflow_window(cx);
        w.update(cx, |v, _, _| {
            v.query = "report".into();
            v.loading = false;
            v.entries = vec![Entry::virtual_entry(
                "report.txt".into(),
                v.path().unwrap().join("report.txt"),
                EntryKind::File,
                None,
                None,
            )];
            v.gallery = true;
            v.rebuild_visible(None);
        })
        .unwrap();
        w
    }
    fn hit(v: &FolderView) -> Arc<Hit> {
        Arc::new(Hit {
            path: v.path().unwrap().join("sub/report.txt"),
            name: "report.txt".into(),
            parent: "sub".into(),
            kind: EntryKind::File,
        })
    }
    #[gpui::test]
    fn imports_append_without_rebuilding_direct_rows_and_stale_keys_cannot_import(
        cx: &mut gpui::TestAppContext,
    ) {
        let w = fixture(cx);
        w.update(cx, |v, _, cx| {
            let key = v.search_key(cx);
            let (ticket, tx) = service::test_ticket(key);
            v.descendants.ticket = Some(ticket);
            v.descendants.active = true;
            v.select(0, cx);
            let direct = v.display_rows.len();
            let offset = v.scroll.0.borrow().base_handle.offset();
            tx.try_send(Message::Search(Event::Batch(vec![hit(v)])))
                .unwrap();
            assert!(v.drain_name_search(key, cx));
            assert_eq!(v.descendants.hits.len(), 1);
            assert_eq!(v.display_rows.len(), direct);
            assert_eq!(v.scroll.0.borrow().base_handle.offset(), offset);
            assert_eq!(v.selected, Some(0));
            assert!(!v.list_gallery());
            assert!(v.gallery);
            v.reset_name_search();
            assert!(tx.is_closed());
            assert!(!v.drain_name_search(key, cx));
            assert!(v.descendants.hits.is_empty());
        })
        .unwrap();
    }
    #[gpui::test]
    fn descendant_selection_never_becomes_entry_indices_or_root_operations(
        cx: &mut gpui::TestAppContext,
    ) {
        let w = fixture(cx);
        w.update(cx, |v, w, cx| {
            v.descendants.hits.push(hit(v));
            v.select(0, cx);
            v.begin_name_edit(
                name_operations::NameOperation::Rename(v.entries[0].path.clone()),
                w,
                cx,
            );
            assert!(v.name_editor.is_some());
            v.name_editor = None;
            v.select_descendant(0, cx);
            assert!(v.selected.is_none());
            assert!(v.selected_paths().is_empty());
            assert!(v.selected_entry().is_none());
            v.select_all(&SelectAllFiles, w, cx);
            assert!(v.selected.is_none());
            v.begin_name_edit(
                name_operations::NameOperation::Folder(v.path().unwrap().into()),
                w,
                cx,
            );
            assert!(v.name_editor.is_none());
            v.extend_selection_to(0, w, cx);
            assert!(v.multi_selection.is_empty());
            v.move_selection(-1, cx);
            assert_eq!(v.selected, Some(0));
            assert!(v.descendants.selected.is_none());
            assert_eq!(v.selected_paths().len(), 1);
            v.move_selection(1, cx);
            assert_eq!(v.descendants.selected, Some(0));
        })
        .unwrap();
    }
    #[gpui::test]
    fn changing_a_filter_invalidates_the_old_request_without_editing_the_name(
        cx: &mut gpui::TestAppContext,
    ) {
        use explorer_core::name_search::{Filters, Kind};

        let w = fixture(cx);
        w.update(cx, |view, _, cx| {
            view.apply_search_filters(Filters::from_input(Kind::Any, ".txt"), cx);
            let key = view.search_key(cx);
            let (ticket, tx) = service::test_ticket(key);
            view.descendants.ticket = Some(ticket);
            view.descendants.active = true;
            view.descendants.hits.push(hit(view));

            view.apply_search_filters(Filters::from_input(Kind::Any, "＊．ＴＸＴ"), cx);
            assert!(!tx.is_closed());

            view.apply_search_filters(Filters::from_input(Kind::Any, "pdf"), cx);
            assert_eq!(view.query, "report");
            assert!(tx.is_closed());
            assert!(view.descendants.hits.is_empty());
            assert!(view.descendants.debounce.is_some());
            assert!(!view.drain_name_search(key, cx));
        })
        .unwrap();
    }
    #[gpui::test]
    fn stopping_search_in_visible_tab_still_allows_new_conditions(cx: &mut gpui::TestAppContext) {
        let w = fixture(cx);
        w.update(cx, |view, _, cx| {
            let key = view.search_key(cx);
            let (ticket, tx) = service::test_ticket(key);
            view.descendants.ticket = Some(ticket);
            view.descendants.active = true;
            view.stop_name_search(cx);
            assert!(tx.is_closed());
            assert!(view.descendants.visible);
            assert!(!view.drain_name_search(key, cx));
            view.query_search_changed(false, cx);
            assert!(view.descendants.debounce.is_some());
        })
        .unwrap();
    }
    #[gpui::test]
    fn hidden_tab_listing_completion_does_not_restart_search(cx: &mut gpui::TestAppContext) {
        let w = fixture(cx);
        w.update(cx, |v, _, cx| {
            v.loading = true;
            v.pause_name_search(cx);
            v.on_listing_event(
                ListingEvent::Done(ListingStats {
                    total: 1,
                    first_batch: None,
                    elapsed: Duration::ZERO,
                    cancelled: false,
                }),
                cx,
            );
            assert!(!v.descendants.active);
            assert!(v.descendants.debounce.is_none());
            v.activate_name_search();
            assert!(v.descendants.debounce.is_none());
        })
        .unwrap();
    }
    #[gpui::test]
    fn unchanged_ime_commit_starts_debounce_but_marked_text_does_not(
        cx: &mut gpui::TestAppContext,
    ) {
        let w = fixture(cx);
        w.update(cx, |v, w, cx| {
            v.search.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "日本語", Some(3..3), w, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        w.update(cx, |v, w, cx| {
            assert_eq!(v.query, "日本語");
            assert!(v.descendants.composing);
            assert!(v.descendants.debounce.is_none());
            v.search.update(cx, |input, cx| input.unmark_text(w, cx));
        })
        .unwrap();
        // Observers run at the end of the update, before the 250ms timer fires.
        w.update(cx, |v, _, cx| {
            assert!(!v.descendants.composing);
            assert!(v.descendants.debounce.is_some());
            v.pause_name_search(cx);
        })
        .unwrap();
    }
    #[gpui::test]
    fn descendant_open_authority_is_cancelled_without_cancelling_direct_open(
        cx: &mut gpui::TestAppContext,
    ) {
        let w = fixture(cx);
        w.update(cx, |v, _, cx| {
            let direct = v.filesystem_open_cancel();
            v.cancel_name_search(false);
            assert!(!direct.is_cancelled());
            v.descendants.hits.push(hit(v));
            v.select_descendant(0, cx);
            let descendant = v.filesystem_open_cancel();
            v.pause_name_search(cx);
            assert!(descendant.is_cancelled());
        })
        .unwrap();
    }
    #[gpui::test]
    fn dropping_tab_entity_cancels_descendant_open_authority(cx: &mut gpui::TestAppContext) {
        let w = fixture(cx);
        let (tab, token) = w
            .update(cx, |v, w, cx| {
                let tab = cx.new(|cx| {
                    FolderView::new_deferred(Location::Home, v.mode, v.icons.clone(), w, cx)
                });
                let token = tab.update(cx, |v, _| {
                    v.descendants.selected = Some(0);
                    v.filesystem_open_cancel()
                });
                (tab, token)
            })
            .unwrap();
        drop(tab);
        cx.update(|_| {});
        assert!(token.is_cancelled());
    }
    #[gpui::test]
    fn descendant_icons_request_only_the_rendered_type_not_the_real_path(
        cx: &mut gpui::TestAppContext,
    ) {
        let w = fixture(cx);
        w.update(cx, |v, w, cx| {
            v.icons = cx.new(ShellIcons::new);
            for i in 0..200 {
                v.descendants.hits.push(Arc::new(Hit {
                    path: PathBuf::from(format!("C:/private/sub/item_{i}.ext{i}")),
                    name: format!("item_{i}.ext{i}"),
                    parent: "sub".into(),
                    kind: EntryKind::File,
                }));
            }
            let key = v.query.to_lowercase();
            let matcher = explorer_core::entry::NameMatcher::new(&key);
            let _ = v.render_descendant(0, v.theme(w), &matcher, w, cx);
            v.icons.read(cx).assert_type_requests(&["file.ext0"]);
        })
        .unwrap();
    }
    #[gpui::test]
    fn direct_and_descendant_names_share_the_same_left_edge(cx: &mut gpui::TestAppContext) {
        let w = fixture(cx);
        w.update(cx, |v, _, cx| {
            v.descendants.hits.push(hit(v));
            cx.notify();
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(w.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let direct = visual.debug_bounds("entry-name-0").unwrap();
        let descendant = visual.debug_bounds("descendant-name-0").unwrap();
        assert!((f32::from(direct.left() - descendant.left())).abs() < 0.1);
        assert_eq!(direct.size.height, descendant.size.height);
    }
    #[gpui::test]
    fn rendered_descendant_and_section_padding_are_not_root_targets(cx: &mut gpui::TestAppContext) {
        let w = fixture(cx);
        w.update(cx, |v, _, cx| {
            v.descendants.hits.push(hit(v));
            cx.notify();
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(w.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let row = visual.debug_bounds("descendant-0").unwrap();
        let header = visual.debug_bounds("search-heading-direct").unwrap();
        w.read_with(&visual, |v, _| {
            assert!(v.descendant_area(row.center()));
            assert!(v.descendant_area(header.center()));
        })
        .unwrap();
        visual.simulate_click(row.center(), gpui::Modifiers::none());
        w.read_with(&visual, |v, _| {
            assert_eq!(v.descendants.selected, Some(0));
            assert!(v.selected.is_none());
        })
        .unwrap();
    }
}
impl SearchState {
    pub(super) fn new(visible: bool) -> Self {
        Self {
            visible,
            ..Self::default()
        }
    }
}
pub(super) enum SearchAnchor {
    Direct(PathBuf, f32),
    Descendant(usize, f32),
}
impl FolderView {
    #[cfg(test)]
    pub(crate) fn hold_name_search_for_test(&mut self, cx: &mut Context<Self>) {
        self.descendants.visible = true;
        let (ticket, _) = service::test_ticket(self.search_key(cx));
        self.descendants.ticket = Some(ticket);
        self.descendants.active = true;
    }
    #[cfg(test)]
    pub(crate) fn name_search_running_for_test(&self) -> bool {
        self.descendants.active
    }
    pub(super) fn filesystem_open_cancel(&mut self) -> CancelToken {
        self.descendants.open_pending = self.descendants.selected.is_some();
        if self.descendants.open_pending {
            self.descendants.open_cancel.clone()
        } else {
            self.cancel.clone()
        }
    }
    pub(super) fn capture_search_anchor(&self) -> Option<SearchAnchor> {
        if !self.recursive_search() {
            return None;
        }
        let height = if self.compact { 24. } else { 28. };
        let offset = -f32::from(self.scroll.0.borrow().base_handle.offset().y);
        let row = ((offset - 2.).max(0.) / height).floor() as usize;
        let pixel = offset - 2. - row as f32 * height;
        if row >= self.display_rows.len() {
            return self
                .descendants
                .hits
                .get(row - self.display_rows.len())
                .map(|_| SearchAnchor::Descendant(row - self.display_rows.len(), pixel));
        }
        match self.display_rows.get(row) {
            Some(DisplayRow::Entry(ix)) => Some(SearchAnchor::Direct(
                self.entries[*self.visible.get(*ix)?].path.clone(),
                pixel,
            )),
            _ => None,
        }
    }
    pub(super) fn restore_search_anchor(&mut self, anchor: Option<SearchAnchor>) {
        let (row, pixel) = match anchor {
            Some(SearchAnchor::Direct(path, pixel)) => {
                let Some(i) = self
                    .visible
                    .iter()
                    .position(|&i| self.entries[i].path == path)
                else {
                    return;
                };
                (self.entry_rows[i], pixel)
            }
            Some(SearchAnchor::Descendant(index, pixel)) if index < self.descendants.hits.len() => {
                (self.display_rows.len() + index, pixel)
            }
            _ => return,
        };
        let handle = self.scroll.0.borrow().base_handle.clone();
        handle.set_offset(gpui::point(
            handle.offset().x,
            px(-(row as f32 * if self.compact { 24. } else { 28. } + 2. + pixel)),
        ));
    }
    pub(crate) fn activate_name_search(&mut self) {
        self.descendants.visible = true;
    }
    /// Includes descendant header, row padding and blank tail, preventing root fallback.
    pub(super) fn descendant_area(&self, position: gpui::Point<Pixels>) -> bool {
        if !self.recursive_search() {
            return false;
        }
        let handle = self.scroll.0.borrow();
        let bounds = handle.base_handle.bounds();
        let y = f32::from(position.y - bounds.origin.y - handle.base_handle.offset().y) - 2.;
        let height = if self.compact { 24. } else { 28. };
        bounds.contains(&position)
            && (y < height || y >= self.display_rows.len().saturating_sub(1) as f32 * height)
    }
    pub(super) fn search_active(&self) -> bool {
        !self.query.is_empty() || !self.search_filters.criteria.is_empty()
    }
    pub(super) fn recursive_search(&self) -> bool {
        self.search_active() && matches!(self.history.current(), Location::Folder(_))
    }
    pub(super) fn list_gallery(&self) -> bool {
        self.gallery && !self.recursive_search()
    }
    pub(super) fn search_key(&self, cx: &Context<Self>) -> Key {
        Key {
            owner: cx.entity_id().as_u64(),
            epoch: self.descendants.epoch,
            generation: self.descendants.generation,
        }
    }
    pub(crate) fn pause_name_search(&mut self, cx: &mut Context<Self>) {
        if !self.descendants.visible {
            return;
        }
        self.descendants.visible = false;
        let running = self.descendants.active || self.descendants.debounce.is_some();
        self.cancel_name_search(false);
        if running {
            self.descendants.stopped = true;
            self.descendants.status = "停止中 · タブを戻しても自動では再開しません".into();
            cx.notify();
        }
    }
    fn stop_name_search(&mut self, cx: &mut Context<Self>) {
        self.cancel_name_search(false);
        self.descendants.stopped = true;
        self.descendants.status = "停止中 · 再検索で更新します".into();
        cx.notify();
    }
    pub(super) fn cancel_name_search(&mut self, clear: bool) {
        self.descendants.generation = self.descendants.generation.wrapping_add(1);
        self.descendants.debounce = None;
        self.descendants.receive = None;
        self.descendants.ticket = None;
        self.descendants.active = false;
        self.descendants.open_cancel.cancel();
        self.descendants.open_cancel = CancelToken::new();
        if self.descendants.open_pending {
            self._open_request = None;
            self.descendants.open_pending = false;
        }
        if clear {
            self.descendants.hits.clear();
            self.descendants.selected = None;
            self.descendants.progress = Progress::default();
        }
    }
    pub(super) fn reset_name_search(&mut self) {
        self.cancel_name_search(true);
        self.descendants.epoch = self.descendants.epoch.wrapping_add(1);
        self.descendants.locality = None;
        self.descendants.status.clear();
        self.descendants.stopped = false;
    }
    pub(super) fn query_search_changed(&mut self, composing: bool, cx: &mut Context<Self>) {
        self.cancel_name_search(true);
        self.descendants.composing = composing;
        self.descendants.stopped = false;
        self.descendants.needs_start = false;
        if !self.recursive_search() {
            return;
        }
        if !self.descendants.visible {
            self.descendants.stopped = true;
            self.descendants.status = "停止中 · 再検索で更新します".into();
            return;
        }
        if composing {
            self.descendants.status = "変換の確定後に配下を検索します".into();
            return;
        }
        if self.query.to_lowercase().len() > 4096 || self.search_filters.criteria.is_too_long() {
            self.descendants.status = "検索条件が長すぎるため、直下のみ表示しています".into();
            return;
        }
        if self.loading {
            self.descendants.status = "直下を読み込み中…".into();
            return;
        }
        self.descendants.status = "入力待ち…".into();
        let key = self.search_key(cx);
        self.descendants.debounce = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.search_key(cx) == key {
                    view.begin_name_search(false, cx);
                }
            });
        }));
    }
    fn begin_name_search(&mut self, explicit: bool, cx: &mut Context<Self>) {
        if !self.descendants.visible
            || !self.recursive_search()
            || self.descendants.composing
            || self.query.to_lowercase().len() > 4096
            || self.search_filters.criteria.is_too_long()
        {
            return;
        }
        self.cancel_name_search(true);
        self.descendants.stopped = false;
        if !explicit && self.descendants.locality == Some(Locality::NeedsStart) {
            self.descendants.needs_start = true;
            self.descendants.status =
                "ネットワーク・未確認の場所です。配下の検索は開始ボタンから".into();
            cx.notify();
            return;
        }
        let root = self.path().unwrap().to_path_buf();
        let key = self.search_key(cx);
        let mode = if explicit {
            Mode::Explicit
        } else if self.descendants.locality == Some(Locality::Local) {
            Mode::Local
        } else {
            Mode::Classify
        };
        match service::request(
            key,
            root,
            self.query.clone(),
            self.search_filters.criteria.clone(),
            self.show_hidden,
            mode,
        ) {
            Err(e) => {
                self.descendants.status = e;
                self.descendants.stopped = true;
            }
            Ok(ticket) => {
                self.descendants.ticket = Some(ticket);
                self.descendants.active = true;
                self.descendants.needs_start = false;
                self.descendants.status = "検索待ち · 配下の結果はスナップショットです".into();
                self.descendants.receive = Some(cx.spawn(async move |view, cx| {
                    loop {
                        let again = view
                            .update(cx, |view, cx| view.drain_name_search(key, cx))
                            .unwrap_or(false);
                        if !again {
                            break;
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(16))
                            .await;
                    }
                }));
            }
        }
        cx.notify();
    }
    fn drain_name_search(&mut self, key: Key, cx: &mut Context<Self>) -> bool {
        if self.search_key(cx) != key || !self.descendants.active {
            return false;
        }
        if self
            .descendants
            .ticket
            .as_ref()
            .is_none_or(|t| Instant::now() >= t.deadline)
        {
            self.descendants.ticket = None;
            self.descendants.active = false;
            self.descendants.stopped = true;
            self.descendants.status = "30秒で停止しました · 表示済みの結果のみ".into();
            cx.notify();
            return false;
        }
        let receiver = &self.descendants.ticket.as_ref().unwrap().receiver;
        if receiver.is_empty() && !receiver.is_closed() {
            return true;
        }
        if !service::drain_available(key) {
            return true;
        }
        let started = Instant::now();
        // One channel event contains at most 64 hits; never rebuild direct rows here.
        let event = self
            .descendants
            .ticket
            .as_ref()
            .unwrap()
            .receiver
            .try_recv();
        if let Ok(message) = event {
            match message {
                Message::Classified(locality) => {
                    self.descendants.locality = Some(locality);
                    if locality == Locality::NeedsStart {
                        self.descendants.active = false;
                        self.descendants.ticket = None;
                        self.descendants.needs_start = true;
                        self.descendants.status =
                            "ネットワーク・未確認の場所です。配下の検索は開始ボタンから".into();
                    }
                }
                Message::Search(Event::Batch(hits)) => {
                    self.descendants.hits.extend(hits);
                    self.descendants.status = "検索中 · 配下の結果はスナップショットです".into();
                }
                Message::Search(Event::Progress(p)) => {
                    self.descendants.progress = p;
                    self.descendants.status = "検索中 · 配下の結果はスナップショットです".into();
                }
                Message::Search(Event::Done(p, end)) => {
                    self.descendants.progress = p;
                    self.descendants.active = false;
                    self.descendants.ticket = None;
                    self.descendants.stopped = true;
                    self.descendants.status = match end {
                        End::Complete => if p.errors + p.links_or_cloud + p.depth_or_volume > 0 {
                            "検索完了 · 一部をスキップ · スナップショット"
                        } else {
                            "検索完了 · スナップショット"
                        }
                        .into(),
                        End::Limited(reason) => {
                            let reason = match reason {
                                Limit::Entries => "確認件数",
                                Limit::Hits => "検索結果件数",
                                Limit::Bytes => "結果の容量",
                                Limit::Depth => "フォルダーの深さ",
                                Limit::Deadline => "検索時間",
                                Limit::Path => "パスの長さ",
                                Limit::Frontier => "探索経路の容量",
                            };
                            format!("{reason}の上限で停止しました · 表示済みの結果のみ")
                        }
                        End::Cancelled => "停止しました".into(),
                    };
                }
                Message::Failed(e) => {
                    self.descendants.status = format!("配下を検索できません: {e}");
                    self.descendants.ticket = None;
                    self.descendants.active = false;
                    self.descendants.stopped = true;
                }
            }
            cx.notify();
        } else if matches!(event, Err(async_channel::TryRecvError::Closed)) {
            self.descendants.ticket = None;
            self.descendants.active = false;
            self.descendants.stopped = true;
            self.descendants.status = "検索処理が終了しました · 表示済みの結果のみ".into();
            cx.notify();
        }
        service::drain_charge(
            key,
            started.elapsed(),
            self.descendants
                .ticket
                .as_ref()
                .is_some_and(|t| !t.receiver.is_empty()),
        );
        self.descendants.active
    }
    pub(super) fn select_descendant(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.descendants.hits.len() {
            return;
        }
        self.cancel_marquee();
        self.close_preview(cx);
        self.typeahead = Default::default();
        self.context_menu = None;
        self.selected = None;
        self.multi_selection.clear();
        self.selection_anchor = None;
        self.descendants.selected = Some(index);
        self.scroll
            .scroll_to_item(self.display_rows.len() + index, ScrollStrategy::Nearest);
        cx.notify();
    }
    pub(super) fn descendant(&self) -> Option<&Arc<Hit>> {
        self.descendants.hits.get(self.descendants.selected?)
    }
    pub(super) fn open_descendant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.descendant().cloned() else {
            return;
        };
        self.open_filesystem_target(hit.path.clone(), hit.kind, window, cx);
    }
    fn reveal_descendant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.descendant().cloned() else {
            return;
        };
        let Some(parent) = hit.path.parent() else {
            return;
        };
        self.navigate_to(parent.to_path_buf(), window, cx);
        self.pending_select_path = Some(hit.path.clone());
    }
    pub(super) fn search_row_count(&self) -> usize {
        self.display_rows.len()
            + if self.recursive_search() {
                self.descendants.hits.len()
            } else {
                0
            }
    }
    pub(super) fn render_search_header(&self, label: String, t: Theme) -> AnyElement {
        let direct = label.starts_with("このフォルダー");
        div()
            .id(("search-heading", usize::from(direct)))
            .debug_selector(move || {
                if direct {
                    "search-heading-direct".into()
                } else {
                    "search-heading-descendant".into()
                }
            })
            .h(px(if self.compact { 24. } else { 28. }))
            .w_full()
            .px(px(16.))
            .flex()
            .items_center()
            .text_size(px(11.))
            .text_color(t.c("text.muted"))
            .child(label)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .into_any_element()
    }
    pub(super) fn render_descendant(
        &self,
        index: usize,
        t: Theme,
        matcher: &explorer_core::entry::NameMatcher<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(hit) = self.descendants.hits.get(index) else {
            return div().into_any_element();
        };
        let selected = self.descendants.selected == Some(index);
        let focused = self.focus.is_focused(window);
        let name_color = if selected && focused {
            t.c("selection.fg")
        } else {
            t.c("text.primary")
        };
        let meta_color = if selected && focused {
            t.c("selection.fg-secondary")
        } else {
            t.c("text.muted")
        };
        div()
            .id(("descendant", index))
            .debug_selector(move || format!("descendant-{index}").into())
            .h(px(if self.compact { 24. } else { 28. }))
            .px(px(12.))
            .w_full()
            .child(
                row_surface(t, selected, focused)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .flex_1()
                            .min_w_0()
                            .child(self.type_icon(
                                &hit.path,
                                hit.kind == EntryKind::Dir,
                                18.,
                                t,
                                window,
                                cx,
                            ))
                            .child(
                                div()
                                    .id(("descendant-name", index))
                                    .debug_selector(move || {
                                        format!("descendant-name-{index}").into()
                                    })
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(name_color)
                                    .when(selected, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                    .child(search_presentation::name_text(
                                        &hit.name,
                                        matcher.ranges_for_name(&hit.name),
                                        t,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .w(px(240.))
                            .flex_shrink_0()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(meta_color)
                            .child(hit.parent.clone()),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, _, w, cx| {
                    cx.stop_propagation();
                    w.focus(&v.focus, cx);
                    v.select_descendant(index, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, _, w, cx| {
                    cx.stop_propagation();
                    w.focus(&v.focus, cx);
                    v.select_descendant(index, cx);
                }),
            )
            .on_click(cx.listener(move |v, e: &ClickEvent, w, cx| {
                if e.click_count() == 2 {
                    v.open_descendant(w, cx);
                }
            }))
            .into_any_element()
    }
    pub(super) fn render_search_bar(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let p = self.descendants.progress;
        let skipped = p.errors + p.links_or_cloud + p.depth_or_volume;
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(6.))
            .text_size(px(11.))
            .text_color(t.c("text.muted"))
            .when(self.descendants.selected.is_some(), |d| {
                d.child(
                    div()
                        .id("search-parent")
                        .cursor_pointer()
                        .text_color(t.c("accent.fg"))
                        .child("親で表示")
                        .on_click(cx.listener(|v, _, w, cx| v.reveal_descendant(w, cx))),
                )
                .child(
                    div()
                        .id("search-copy-path")
                        .cursor_pointer()
                        .text_color(t.c("accent.fg"))
                        .child("パスをコピー")
                        .on_click(cx.listener(|v, _, _, cx| {
                            if let Some(h) = v.descendant() {
                                v.copy_path_list(&[h.path.clone()], cx);
                            }
                        })),
                )
                .when(
                    self.descendant().is_some_and(|h| h.kind == EntryKind::Dir),
                    |d| {
                        d.child(
                            div()
                                .id("search-new-tab")
                                .cursor_pointer()
                                .text_color(t.c("accent.fg"))
                                .child("新しいタブ")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    if let Some(h) = v.descendant() {
                                        cx.emit(FolderEvent::OpenInTab(h.path.clone()));
                                    }
                                })),
                        )
                    },
                )
            })
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "{} · {} 件を確認{}",
                self.descendants.status,
                p.visited,
                if skipped > 0 {
                    format!(" · {skipped} 件のリンク／クラウド／未確認領域をスキップ")
                } else {
                    String::new()
                }
            )))
            .child(
                div()
                    .id("search-control")
                    .cursor_pointer()
                    .text_color(t.c("accent.fg"))
                    .child(if self.descendants.active {
                        "停止"
                    } else if self.descendants.needs_start || self.descendants.locality.is_none() {
                        "配下の検索を開始"
                    } else {
                        "再検索"
                    })
                    .on_click(cx.listener(|v, _, _, cx| {
                        if v.descendants.active {
                            v.stop_name_search(cx);
                        } else {
                            v.begin_name_search(
                                v.descendants.locality != Some(Locality::Local),
                                cx,
                            );
                        }
                    })),
            )
            .into_any_element()
    }
    pub(super) fn render_descendant_inspector(
        &self,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hit = self.descendant().unwrap();
        div()
            .w(px(280.))
            .h_full()
            .flex_shrink_0()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_size(px(12.))
            .text_color(t.c("text.primary"))
            .child(hit.name.clone())
            .child(
                div()
                    .text_color(t.c("text.muted"))
                    .child(hit.path.display().to_string()),
            )
            .child("配下の検索結果 · スナップショット")
            .child(
                div()
                    .id("hit-open")
                    .cursor_pointer()
                    .child("開く")
                    .on_click(cx.listener(|v, _, w, cx| v.open_descendant(w, cx))),
            )
            .when(hit.kind == EntryKind::Dir, |d| {
                d.child(
                    div()
                        .id("hit-tab")
                        .cursor_pointer()
                        .child("新しいタブで開く")
                        .on_click(cx.listener(|v, _, _, cx| {
                            if let Some(h) = v.descendant() {
                                cx.emit(FolderEvent::OpenInTab(h.path.clone()));
                            }
                        })),
                )
            })
            .child(
                div()
                    .id("hit-parent")
                    .cursor_pointer()
                    .child("親フォルダーで表示")
                    .on_click(cx.listener(|v, _, w, cx| v.reveal_descendant(w, cx))),
            )
            .child(
                div()
                    .id("hit-copy-path")
                    .cursor_pointer()
                    .child("パスをコピー")
                    .on_click(cx.listener(|v, _, _, cx| {
                        if let Some(h) = v.descendant() {
                            v.copy_path_list(&[h.path.clone()], cx);
                        }
                    })),
            )
            .into_any_element()
    }
}
