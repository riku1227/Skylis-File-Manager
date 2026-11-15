//! Refresh a visible directory from a complete worker snapshot, without dismantling the
//! current view. Native notifications and in-flight snapshots share a navigation lifetime.
use super::*;
use crate::directory_watch::{self, WatchEvent, WatchHandle};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static REFRESH_WORKERS: AtomicUsize = AtomicUsize::new(0);
const MAX_REFRESH_WORKERS: usize = 4;

#[derive(Default)]
pub(super) struct WatchState {
    path: Option<PathBuf>,
    navigation: CancelToken,
    handle: Option<WatchHandle>,
    events: Option<Task<()>>,
    dirty: bool,
    refresh_cancel: CancelToken,
    refresh_task: Option<Task<()>>,
    completed: Option<Result<Snapshot, std::io::Error>>,
    last_error_notice: Option<String>,
    retry: Option<Task<()>>,
}

impl Drop for WatchState {
    fn drop(&mut self) {
        self.navigation.cancel();
        self.refresh_cancel.cancel();
    }
}

#[cfg(test)]
impl WatchState {
    pub(super) fn for_directory(path: PathBuf) -> Self {
        let mut state = Self::default();
        state.path = Some(path);
        state
    }

    pub(super) fn refresh_pending(&self) -> bool {
        self.dirty || self.refresh_task.is_some() || self.completed.is_some()
    }
}

struct Snapshot {
    entries: Vec<Entry>,
    stats: ListingStats,
    sort: SortOrder,
}

struct RefreshSlot;
impl RefreshSlot {
    fn acquire() -> Option<Self> {
        let mut active = REFRESH_WORKERS.load(Ordering::Acquire);
        loop {
            if active >= MAX_REFRESH_WORKERS {
                return None;
            }
            match REFRESH_WORKERS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self),
                Err(current) => active = current,
            }
        }
    }
}
impl Drop for RefreshSlot {
    fn drop(&mut self) {
        REFRESH_WORKERS.fetch_sub(1, Ordering::Release);
    }
}

impl FolderView {
    /// Call after load resets its notice, including when leaving for Home/an archive.
    /// A same-directory explicit load replaces snapshots but keeps the native watch.
    pub(super) fn update_watch(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        self.watch_state.refresh_cancel.cancel();
        self.watch_state.refresh_task = None;
        self.watch_state.completed = None;
        self.watch_state.dirty = false;
        self.watch_state.retry = None;
        if self.watch_state.path.as_deref() == path {
            return;
        }
        self.watch_state = WatchState::default();
        self.watch_state.path = path.map(Path::to_path_buf);
        let Some(path) = self.watch_state.path.clone() else {
            return;
        };
        let navigation = self.watch_state.navigation.clone();
        match directory_watch::watch(path, navigation.clone()) {
            Ok((handle, receiver)) => {
                self.watch_state.handle = Some(handle);
                self.watch_state.events = Some(cx.spawn(async move |view, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if navigation.is_cancelled() {
                            break;
                        }
                        let done = view.update(cx, |view, cx| {
                            if navigation.is_cancelled() {
                                return true;
                            }
                            match event {
                                WatchEvent::Changed => view.request_refresh(cx),
                                WatchEvent::Unavailable(message) => {
                                    view.notice = Some(message);
                                    cx.notify();
                                    return true;
                                }
                            }
                            false
                        });
                        if !matches!(done, Ok(false)) {
                            break;
                        }
                    }
                }));
            }
            Err(message) => self.notice = Some(message),
        }
    }

    /// Also used after app-initiated mutations. Coalesces with a watch notification and
    /// keeps an existing selection/menu/editor alive until its interaction finishes.
    pub(super) fn request_refresh(&mut self, cx: &mut Context<Self>) {
        if self.path().is_none() || self.path() != self.watch_state.path.as_deref() {
            return;
        }
        self.watch_state.dirty = true;
        self.flush_directory_refresh(cx);
    }

    /// Called at the beginning of render as well as on notification/completion. There is
    /// no idle timer: a deferred interaction already causes a render when it finishes.
    pub(super) fn flush_directory_refresh(&mut self, cx: &mut Context<Self>) {
        if self.watch_state.path.is_none()
            || self.path() != self.watch_state.path.as_deref()
            || self.loading
            || self.name_editor.is_some()
            || self.archive_dialog.is_some()
            || self.context_menu.is_some()
            || self.marquee_in_progress()
            || self.scrollbar_drag.is_some()
            || self.motion.trajectory.active()
        {
            return;
        }
        if let Some(result) = self.watch_state.completed.take() {
            match result {
                Ok(snapshot) if snapshot.sort == self.sort => self.apply_snapshot(snapshot, cx),
                Ok(_) => self.watch_state.dirty = true,
                Err(error) => {
                    // Retain the last valid rows on permission/disconnection errors.
                    self.permission_observation.record(Err(error.kind()), false);
                    self.notice = Some(format!("一覧を更新できませんでした: {error}"));
                    self.watch_state.last_error_notice = self.notice.clone();
                    cx.notify();
                }
            }
        }
        if !self.watch_state.dirty || self.watch_state.refresh_task.is_some() {
            return;
        }
        let Some(slot) = RefreshSlot::acquire() else {
            self.retry_directory_refresh(cx);
            return;
        };
        self.watch_state.retry = None;
        self.watch_state.dirty = false;
        self.watch_state.refresh_cancel.cancel();
        self.watch_state.refresh_cancel = CancelToken::new();
        let token = self.watch_state.refresh_cancel.clone();
        let worker_token = token.clone();
        let path = self.watch_state.path.clone().unwrap();
        let sort = self.sort;
        let (sender, receiver) = async_channel::bounded(1);
        let worker = std::thread::Builder::new()
            .name("refresh-directory".into())
            .spawn(move || {
                let _slot = slot;
                let result = collect_snapshot(&path, sort, &worker_token);
                if !worker_token.is_cancelled() {
                    let _ = sender.try_send(result);
                }
            });
        if let Err(error) = worker {
            self.notice = Some(format!("一覧の更新を開始できません: {error}"));
            cx.notify();
            return;
        }
        self.watch_state.refresh_task = Some(cx.spawn(async move |view, cx| {
            let result = receiver
                .recv()
                .await
                .unwrap_or_else(|_| Err(std::io::Error::other("更新処理が終了しました")));
            let _ = view.update(cx, |view, cx| {
                if token.is_cancelled() {
                    return;
                }
                view.watch_state.refresh_task = None;
                view.watch_state.completed = Some(result);
                view.flush_directory_refresh(cx);
            });
        }));
    }

    fn retry_directory_refresh(&mut self, cx: &mut Context<Self>) {
        if self.watch_state.retry.is_some() {
            return;
        }
        let navigation = self.watch_state.navigation.clone();
        self.watch_state.retry = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let _ = view.update(cx, |view, cx| {
                if !navigation.is_cancelled() {
                    view.watch_state.retry = None;
                    view.flush_directory_refresh(cx);
                }
            });
        }));
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.stats = Some(snapshot.stats);
        self.permission_observation
            .record(Ok(()), snapshot.stats.cancelled);
        if self.watch_state.last_error_notice.is_some()
            && self.notice == self.watch_state.last_error_notice
        {
            self.notice = None;
        }
        self.watch_state.last_error_notice = None;
        cx.notify();
        if same_entries(&self.entries, &snapshot.entries) {
            if self.error.take().is_some() {
                cx.notify();
            }
            return;
        }
        let selected = self.selected_entry().map(|entry| entry.path.clone());
        self.cancel_scroll_motion();
        let scroll = self.scroll.0.borrow().base_handle.clone();
        let offset = scroll.offset();
        self.entries = snapshot.entries;
        let existing: HashSet<&Path> = self
            .entries
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();
        self.multi_selection
            .retain(|path| existing.contains(path.as_path()));
        if self
            .selection_anchor
            .as_ref()
            .is_some_and(|path| !existing.contains(path.as_path()))
        {
            self.selection_anchor = None;
        }
        self.group_date = chrono::Local::now().date_naive();
        self.rebuild_visible(selected.as_deref());
        if self.selected.is_none() {
            self.selected = self
                .visible
                .iter()
                .position(|&index| self.multi_selection.contains(&self.entries[index].path));
        }
        scroll.set_offset(offset);
        self.error = None;
        self.retain_visible_directory_sizes();
        cx.notify();
    }
}

fn same_entries(previous: &[Entry], next: &[Entry]) -> bool {
    previous.len() == next.len()
        && previous.iter().zip(next).all(|(a, b)| {
            a.path == b.path
                && a.name == b.name
                && a.kind == b.kind
                && a.size == b.size
                && a.modified == b.modified
                && a.hidden == b.hidden
        })
}

fn collect_snapshot(
    path: &Path,
    sort: SortOrder,
    cancel: &CancelToken,
) -> Result<Snapshot, std::io::Error> {
    if cancel.is_cancelled() {
        return Err(std::io::Error::other("更新は取り消されました"));
    }
    let mut entries = Vec::new();
    let mut stats = None;
    let mut error = None;
    list_dir(path, cancel, |event| {
        if cancel.is_cancelled() {
            return false;
        }
        match event {
            ListingEvent::Batch(batch) => entries.extend(batch),
            ListingEvent::Done(done) => stats = Some(done),
            ListingEvent::Failed(kind, message) => error = Some(std::io::Error::new(kind, message)),
        }
        true
    });
    if let Some(error) = error {
        return Err(error);
    }
    let stats = stats.ok_or_else(|| std::io::Error::other("更新は取り消されました"))?;
    if stats.cancelled || cancel.is_cancelled() {
        return Err(std::io::Error::other("更新は取り消されました"));
    }
    entries.sort_unstable_by(|a, b| sort.compare(a, b));
    Ok(Snapshot {
        entries,
        stats,
        sort,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn explicit_refresh_cancels_an_unfinished_initial_listing(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, window, cx| {
                view.history = History::new(Location::Folder(
                    std::env::temp_dir().join("skylis-f5-model-missing"),
                ));
                view.loading = true;
                let obsolete = view.cancel.clone();
                view.refresh(&Refresh, window, cx);
                assert!(obsolete.is_cancelled());
                assert!(!view.cancel.is_cancelled());
                assert!(view.loading);
                assert!(view._drive_task.is_some());
            })
            .unwrap();
    }

    fn entry(directory: &Path, name: &str) -> Entry {
        Entry::virtual_entry(
            name.into(),
            directory.join(name),
            EntryKind::File,
            Some(1),
            None,
        )
    }

    fn snapshot(view: &FolderView, entries: Vec<Entry>) -> Snapshot {
        Snapshot {
            stats: ListingStats {
                total: entries.len(),
                first_batch: None,
                elapsed: Duration::ZERO,
                cancelled: false,
            },
            entries,
            sort: view.sort,
        }
    }

    #[gpui::test]
    fn snapshot_preserves_live_selection_query_scroll_and_drops_missing_targets(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, _, cx| {
                let directory = std::env::temp_dir().join("skylis-watch-snapshot-model");
                view.history.push(Location::Folder(directory.clone()), None);
                view.watch_state.path = Some(directory.clone());
                view.loading = false;
                view.query = "keep".into();
                view.entries = vec![
                    entry(&directory, "keep-a"),
                    entry(&directory, "keep-b"),
                    entry(&directory, "keep-c"),
                ];
                let selected = directory.join("keep-b");
                let removed = directory.join("keep-c");
                view.multi_selection = [selected.clone(), removed].into_iter().collect();
                view.selection_anchor = Some(selected.clone());
                view.rebuild_visible(Some(&selected));
                let scroll = view.scroll.0.borrow().base_handle.clone();
                let offset = gpui::point(px(0.), px(-56.));
                scroll.set_offset(offset);
                let update = snapshot(
                    view,
                    vec![
                        entry(&directory, "keep-b"),
                        entry(&directory, "unmatched-new"),
                    ],
                );
                view.apply_snapshot(update, cx);
                assert_eq!(view.query, "keep");
                assert_eq!(view.selected_entry().unwrap().path, selected);
                assert_eq!(
                    view.multi_selection,
                    [selected.clone()].into_iter().collect()
                );
                assert_eq!(view.selection_anchor, Some(selected));
                assert_eq!(view.visible.len(), 1);
                assert_eq!(scroll.offset(), offset);
            })
            .unwrap();
    }

    #[gpui::test]
    fn completed_snapshot_waits_until_inline_rename_finishes(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, window, cx| {
                let directory = std::env::temp_dir().join("skylis-watch-editor-model");
                view.history.push(Location::Folder(directory.clone()), None);
                view.watch_state.path = Some(directory.clone());
                view.loading = false;
                view.entries = vec![entry(&directory, "old.txt")];
                view.rebuild_visible(Some(&directory.join("old.txt")));
                view.begin_name_edit(
                    name_operations::NameOperation::Rename(directory.join("old.txt")),
                    window,
                    cx,
                );
                assert!(view.name_editor.is_some());
                view.watch_state.completed =
                    Some(Ok(snapshot(view, vec![entry(&directory, "new.txt")])));
                view.flush_directory_refresh(cx);
                assert_eq!(view.entries[0].name, "old.txt");
                assert!(view.watch_state.completed.is_some());
                view.name_editor = None;
                view.flush_directory_refresh(cx);
                assert_eq!(view.entries[0].name, "new.txt");
                assert!(view.watch_state.completed.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn successful_retry_clears_denial_and_refresh_notice(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|w, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, w, cx)
        });
        window
            .update(cx, |view, _, cx| {
                let path = PathBuf::from("/skylis-disposable-model");
                view.history.push(Location::Folder(path.clone()), None);
                view.watch_state.path = Some(path);
                view.loading = false;
                view.permission_observation = crate::macos_permissions::Observation::Denied;
                view.watch_state.completed = Some(Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "denied fixture",
                )));
                view.flush_directory_refresh(cx);
                assert!(view.notice.is_some());
                let update = snapshot(view, vec![]);
                view.apply_snapshot(update, cx);
                assert!(
                    view.notice.is_none(),
                    "successful retry clears the old refresh failure"
                );
                assert_eq!(
                    view.permission_observation,
                    crate::macos_permissions::Observation::Accessible
                );
            })
            .unwrap();
    }

    #[test]
    fn cancelled_snapshot_never_publishes_a_partial_directory() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(
            collect_snapshot(
                Path::new(env!("CARGO_MANIFEST_DIR")),
                SortOrder::default(),
                &cancel
            )
            .is_err()
        );
    }

    #[gpui::test]
    fn refresh_during_initial_load_queues_once_and_reload_invalidates_old_snapshot(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, _, cx| {
                let directory = std::env::temp_dir().join("skylis-watch-loading-model");
                view.history.push(Location::Folder(directory.clone()), None);
                view.watch_state.path = Some(directory.clone());
                view.entries = vec![entry(&directory, "visible.txt")];
                view.loading = true;
                let listing = view.cancel.clone();
                view.request_refresh(cx);
                view.request_refresh(cx);
                assert!(view.watch_state.dirty);
                assert!(view.watch_state.refresh_task.is_none());
                assert!(!listing.is_cancelled());
                assert_eq!(view.entries[0].name, "visible.txt");

                let stale = view.watch_state.refresh_cancel.clone();
                view.watch_state.completed =
                    Some(Ok(snapshot(view, vec![entry(&directory, "stale.txt")])));
                view.update_watch(Some(&directory), cx);
                assert!(stale.is_cancelled());
                assert!(view.watch_state.completed.is_none());
                assert!(!view.watch_state.dirty);
                assert_eq!(view.watch_state.path, Some(directory));
            })
            .unwrap();
    }
}
