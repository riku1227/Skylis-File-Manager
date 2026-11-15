use super::*;
use crate::file_operations::{self, TransferKind, TransferOutcome};
use std::{cell::OnceCell, collections::BTreeSet, rc::Rc};

/// 弱参照だけを保持し、終了したドラッグの選択や画像を残さない。
struct DragFeedback {
    source: gpui::EntityId,
    payload: std::rc::Weak<OnceCell<ResolvedDrag>>,
}
impl gpui::Global for DragFeedback {}
impl DragFeedback {
    fn contains(&self, source: gpui::EntityId, path: &Path, active: bool) -> bool {
        active
            && self.source == source
            && self
                .payload
                .upgrade()
                .is_some_and(|payload| payload.get().is_some_and(|payload| payload.contains(path)))
    }
}

/// 表示済みパスだけで明らかに無効な対象を除外する。実パスの検証はワーカーで再実施。
fn valid_drop_paths(paths: &[PathBuf], destination: &Path) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            destination != path
                && !destination.starts_with(path)
                && path.parent() != Some(destination)
        })
}

/// Cheap per-row descriptor. The selection is frozen once when a drag actually starts,
/// rather than cloning a potentially huge selection for every visible row on every frame.
#[derive(Clone)]
pub(super) struct FileDrag {
    source: gpui::WeakEntity<FolderView>,
    item: PathBuf,
    is_dir: bool,
    mode: ThemeMode,
    payload: Rc<OnceCell<ResolvedDrag>>,
}

struct ResolvedDrag {
    paths: Vec<PathBuf>,
    directories: BTreeSet<PathBuf>,
    sorted_paths: Vec<usize>,
}
impl ResolvedDrag {
    fn new(paths: Vec<PathBuf>, directories: BTreeSet<PathBuf>) -> Self {
        // パス本体を複製せず、可視行の半透明判定をO(log n)にする。
        let mut sorted_paths: Vec<_> = (0..paths.len()).collect();
        sorted_paths.sort_unstable_by(|&a, &b| paths[a].cmp(&paths[b]));
        Self {
            paths,
            directories,
            sorted_paths,
        }
    }
    fn contains(&self, path: &Path) -> bool {
        self.sorted_paths
            .binary_search_by(|&i| self.paths[i].as_path().cmp(path))
            .is_ok()
    }
}

impl FileDrag {
    /// Call in the on_drag constructor before creating the drag view, outside a source
    /// entity update. Later drop callbacks may update that same entity, so they must read
    /// the frozen payload rather than re-enter the source to obtain its current selection.
    pub(super) fn resolve(&self, cx: &mut App) {
        self.payload.get_or_init(|| {
            let Some(source) = self.source.upgrade() else {
                return self.fallback();
            };
            let source = source.read(cx);
            let paths = if source.multi_selection.contains(&self.item) {
                source.selected_paths()
            } else {
                vec![self.item.clone()]
            };
            let selected: BTreeSet<_> = paths.iter().collect();
            let mut directories: BTreeSet<_> = source
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Dir && selected.contains(&entry.path))
                .map(|entry| entry.path.clone())
                .collect();
            if self.is_dir && selected.contains(&self.item) {
                directories.insert(self.item.clone());
            }
            ResolvedDrag::new(paths, directories)
        });
        cx.set_global(DragFeedback {
            source: self.source.entity_id(),
            payload: Rc::downgrade(&self.payload),
        });
    }

    fn fallback(&self) -> ResolvedDrag {
        let paths = if self.item.as_os_str().is_empty() {
            Vec::new()
        } else {
            vec![self.item.clone()]
        };
        let directories = if self.is_dir {
            paths.iter().cloned().collect()
        } else {
            BTreeSet::new()
        };
        ResolvedDrag::new(paths, directories)
    }

    fn payload(&self) -> &ResolvedDrag {
        // Defensive fallback also avoids borrowing a source view from its own drop handler.
        self.payload.get_or_init(|| self.fallback())
    }

    pub(super) fn paths(&self, _cx: &App) -> Vec<PathBuf> {
        self.payload().paths.clone()
    }

    pub(super) fn external_payload(&self, _cx: &App) -> Option<gpui::ExternalDragPayload> {
        let payload = self.payload();
        if payload.paths.is_empty() {
            return None;
        }
        Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new(
            payload
                .paths
                .iter()
                .map(|path| (path.clone(), payload.directories.contains(path))),
        )))
    }
}

impl Render for FileDrag {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::resolve(self.mode, window.appearance());
        div()
            .p(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.c("border.subtle"))
            .bg(theme.c("surface.raised"))
            .text_color(theme.c("text.primary"))
            .opacity(0.88)
            .child(format!(
                "{} 項目 · Ctrlでコピー / Shiftで移動",
                self.payload().paths.len()
            ))
    }
}

/// Completion belongs to the location where the action began. Navigation cancels its
/// transient notice, not the actual user-requested transfer running in the native worker.
struct TransferContext {
    origin: Location,
    destination: PathBuf,
    sources: BTreeSet<PathBuf>,
    listing: CancelToken,
}

impl TransferContext {
    fn affected_directories(&self, kind: TransferKind) -> BTreeSet<PathBuf> {
        let mut directories = BTreeSet::from([self.destination.clone()]);
        if kind != TransferKind::Copy {
            directories.extend(self.sources.iter().cloned());
        }
        directories
    }
}

/// One application-wide event reaches matching tabs and windows, including when the
/// initiating tab has closed. A view subscribes once when created, not when reparented.
struct FileTransferEvents;
impl gpui::EventEmitter<FileTransferCompleted> for FileTransferEvents {}
struct SharedFileTransferEvents(Entity<FileTransferEvents>);
impl gpui::Global for SharedFileTransferEvents {}

struct FileTransferCompleted {
    directories: BTreeSet<PathBuf>,
    notice: Option<TransferNotice>,
}

struct TransferNotice {
    initiator: gpui::EntityId,
    origin: Location,
    listing: CancelToken,
    text: String,
}

impl TransferNotice {
    fn applies_to(&self, view: &FolderView, id: gpui::EntityId) -> bool {
        self.initiator == id
            && !self.listing.is_cancelled()
            && *view.history.current() == self.origin
    }
}

fn file_transfer_events(cx: &mut App) -> Entity<FileTransferEvents> {
    if let Some(shared) = cx.try_global::<SharedFileTransferEvents>() {
        shared.0.clone()
    } else {
        let events = cx.new(|_| FileTransferEvents);
        cx.set_global(SharedFileTransferEvents(events.clone()));
        events
    }
}

fn cut_sequence_to_clear(
    kind: TransferKind,
    result: &Result<TransferOutcome, String>,
    sequence: Option<u32>,
) -> Option<u32> {
    if kind == TransferKind::Move && matches!(result, Ok(TransferOutcome::Completed)) {
        sequence
    } else {
        None
    }
}

#[cfg(windows)]
static NATIVE_DRAG_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static NATIVE_DROP_ACCEPTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
struct NativeDragLease;
#[cfg(windows)]
impl NativeDragLease {
    fn acquire() -> Option<Self> {
        use std::sync::atomic::Ordering;
        NATIVE_DRAG_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| {
                NATIVE_DROP_ACCEPTED.store(false, Ordering::Release);
                Self
            })
    }
}
#[cfg(windows)]
impl Drop for NativeDragLease {
    fn drop(&mut self) {
        NATIVE_DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl FolderView {
    pub(super) fn is_drag_source(&self, path: &Path, cx: &Context<Self>) -> bool {
        let active = cx.has_active_drag();
        #[cfg(windows)]
        let active = active || NATIVE_DRAG_ACTIVE.load(std::sync::atomic::Ordering::Acquire);
        cx.try_global::<DragFeedback>()
            .is_some_and(|feedback| feedback.contains(cx.entity_id(), path, active))
    }

    /// Bind only a known folder destination. Home, toolbar gaps and other non-folder
    /// surfaces deliberately have no fallback to the currently displayed directory.
    /// Paths come from existing view data; validation remains on the transfer worker.
    pub(super) fn file_drop_target<E: gpui::InteractiveElement>(
        element: E,
        destination: Option<PathBuf>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> E {
        Self::file_drop_receiver(element, destination, cx)
            .drag_over::<FileDrag>(move |style, _, _, _| style.bg(theme.c("accent.soft")))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| {
                style.bg(theme.c("accent.soft"))
            })
    }

    /// 背景にもドロップできるが、一覧全体を青く塗ることはしない。
    pub(super) fn file_drop_receiver<E: gpui::InteractiveElement>(
        element: E,
        destination: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> E {
        let Some(destination) = destination else {
            return element;
        };
        let external_destination = destination.clone();
        let hover_destination = destination.clone();
        element
            .can_drop(move |drag, _, _| {
                if let Some(drag) = drag.downcast_ref::<FileDrag>() {
                    valid_drop_paths(&drag.payload().paths, &hover_destination)
                } else if let Some(drag) = drag.downcast_ref::<gpui::ExternalPaths>() {
                    valid_drop_paths(drag.paths(), &hover_destination)
                } else {
                    false
                }
            })
            .on_drop(cx.listener(move |this, drag: &FileDrag, window, cx| {
                cx.stop_propagation();
                let modifiers = window.modifiers();
                let kind = TransferKind::from_drop_modifiers(modifiers.control, modifiers.shift);
                this.receive_files(drag.paths(cx), destination.clone(), kind, window, cx);
            }))
            .on_drop(
                cx.listener(move |this, drag: &gpui::ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let modifiers = window.modifiers();
                    this.receive_files(
                        drag.paths().to_vec(),
                        external_destination.clone(),
                        TransferKind::from_drop_modifiers(modifiers.control, modifiers.shift),
                        window,
                        cx,
                    );
                }),
            )
    }

    pub(super) fn observe_file_transfers(&mut self, cx: &mut Context<Self>) {
        let events = file_transfer_events(cx);
        cx.subscribe(&events, |view, _, event: &FileTransferCompleted, cx| {
            // Evaluate before load cancels the old listing generation.
            let notice = event
                .notice
                .as_ref()
                .filter(|notice| notice.applies_to(view, cx.entity_id()));
            let refresh = match view.history.current() {
                Location::Folder(path) => event.directories.contains(path),
                _ => false,
            };
            if refresh {
                let selected = view.selected_name();
                view.load(view.history.current().clone(), selected, cx);
            }
            if let Some(notice) = notice {
                view.notice = Some(notice.text.clone());
            }
            if refresh || notice.is_some() {
                cx.notify();
            }
        })
        .detach();
    }

    pub(super) fn file_drag(&self, ix: usize, cx: &Context<Self>) -> FileDrag {
        let entry = self
            .visible
            .get(ix)
            .and_then(|&index| self.entries.get(index));
        FileDrag {
            source: cx.weak_entity(),
            item: entry.map(|entry| entry.path.clone()).unwrap_or_default(),
            is_dir: entry.is_some_and(|entry| entry.kind == EntryKind::Dir),
            mode: self.mode,
            payload: Rc::new(OnceCell::new()),
        }
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        self.visible
            .iter()
            .map(|&index| &self.entries[index].path)
            .filter(|path| self.multi_selection.contains(*path))
            .cloned()
            .collect()
    }

    pub(super) fn pointer_select(
        &mut self,
        ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(&index) = self.visible.get(ix) else {
            return;
        };
        cx.stop_propagation();
        let path = self.entries[index].path.clone();
        window.focus(&self.focus, cx);
        if event.modifiers.shift {
            let anchor = self
                .selection_anchor
                .as_ref()
                .and_then(|path| {
                    self.visible
                        .iter()
                        .position(|&index| self.entries[index].path == *path)
                })
                .unwrap_or(self.selected.unwrap_or(ix))
                .min(self.visible.len() - 1);
            if !event.modifiers.control && !event.modifiers.platform {
                self.multi_selection.clear();
            }
            for position in anchor.min(ix)..=anchor.max(ix) {
                self.multi_selection
                    .insert(self.entries[self.visible[position]].path.clone());
            }
            self.selected = Some(ix);
        } else if event.modifiers.control || event.modifiers.platform {
            if self.multi_selection.remove(&path) {
                self.selected = self
                    .visible
                    .iter()
                    .position(|&index| self.multi_selection.contains(&self.entries[index].path));
            } else {
                self.multi_selection.insert(path.clone());
                self.selected = Some(ix);
            }
            self.selection_anchor = Some(path);
        } else {
            // Preserve a multi-selection on mouse-down so its whole payload can be dragged.
            if !self.multi_selection.contains(&path) {
                self.multi_selection.clear();
                self.multi_selection.insert(path.clone());
            }
            self.selected = Some(ix);
            self.selection_anchor = Some(path);
        }
        self.preview_scope.cancel();
        cx.notify();
    }

    pub(super) fn select_all(
        &mut self,
        _: &SelectAllFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focused(window, cx) || self.is_home() || self.marquee_in_progress() {
            return;
        }
        self.multi_selection = self
            .visible
            .iter()
            .map(|&index| self.entries[index].path.clone())
            .collect();
        self.selected = self
            .selected
            .or_else(|| (!self.visible.is_empty()).then_some(0));
        self.selection_anchor = self
            .selected
            .and_then(|ix| self.visible.get(ix))
            .map(|&index| self.entries[index].path.clone());
        cx.notify();
    }

    /// Collapse a preserved multi-selection only after a plain click completes. GPUI does
    /// not emit the row click after a drag, so mouse-down can retain the full drag payload.
    pub(super) fn finish_pointer_click(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        if !event.standard_click() || modifiers.control || modifiers.platform || modifiers.shift {
            return;
        }
        let Some(&index) = self.visible.get(ix) else {
            return;
        };
        self.multi_selection.clear();
        self.multi_selection
            .insert(self.entries[index].path.clone());
        self.selected = Some(ix);
        self.selection_anchor = Some(self.entries[index].path.clone());
        cx.notify();
    }

    pub(super) fn receive_files(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        kind: TransferKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_transfer(paths, destination, kind, None, window, cx);
    }

    fn start_transfer(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        kind: TransferKind,
        cut_sequence: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = TransferContext {
            origin: self.history.current().clone(),
            destination: destination.clone(),
            sources: paths
                .iter()
                .filter_map(|path| path.parent().map(Path::to_path_buf))
                .collect(),
            listing: self.cancel.clone(),
        };
        let initiator = cx.entity_id();
        let events = file_transfer_events(cx);
        match file_operations::transfer(paths, destination, kind, window) {
            Err(error) => self.notice = Some(error.to_string()),
            Ok(receiver) => {
                #[cfg(windows)]
                if NATIVE_DRAG_ACTIVE.load(std::sync::atomic::Ordering::Acquire) {
                    // A native drag dropped back into Skylis. Its native result arrives
                    // before our worker completes; publish only that worker's completion.
                    NATIVE_DROP_ACCEPTED.store(true, std::sync::atomic::Ordering::Release);
                }
                self.notice = Some(
                    match kind {
                        TransferKind::Copy => "コピーしています…",
                        TransferKind::Move => "移動しています…",
                        TransferKind::Automatic => "ファイルを転送しています…",
                    }
                    .into(),
                );
                cx.spawn(async move |_, cx| {
                    let result = receiver.recv().await.unwrap_or_else(|_| Err("ファイル操作の結果を取得できませんでした。元と保存先を確認してください".into()));
                    let mut clipboard_error = None;
                    #[cfg(windows)]
                    if let Some(sequence) = cut_sequence_to_clear(kind, &result, cut_sequence) {
                        // Independent of the folder entity: a completed move may outlive its tab.
                        // A changed clipboard is deliberately preserved by the native sequence check.
                        if let Err(error) = file_operations::clear_file_clipboard(sequence) {
                            clipboard_error = Some(format!("移動は完了しましたが、切り取り情報を消去できませんでした: {error}"));
                        }
                    }
                    #[cfg(not(windows))]
                    let _ = cut_sequence;
                    let text = clipboard_error.unwrap_or_else(|| match result {
                                Ok(TransferOutcome::Completed) => match kind { TransferKind::Copy => "ファイルをコピーしました", TransferKind::Move => "ファイルを移動しました", TransferKind::Automatic => "ファイルを転送しました" }.into(),
                                Ok(TransferOutcome::Aborted) => "ファイル操作を中止しました。一部は完了している場合があります".into(),
                                Err(error) => error,
                            });
                    let completion = FileTransferCompleted {
                        directories: context.affected_directories(kind),
                        notice: Some(TransferNotice { initiator, origin: context.origin, listing: context.listing, text }),
                    };
                    let _ = events.update(cx, |_, cx| {
                        cx.emit(completion);
                    });
                }).detach();
            }
        }
        cx.notify();
    }

    pub(super) fn copy_files(
        &mut self,
        _: &CopyFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clipboard_write(TransferKind::Copy, window, cx);
    }

    pub(super) fn cut_files(&mut self, _: &CutFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.clipboard_write(TransferKind::Move, window, cx);
    }

    fn clipboard_write(&mut self, kind: TransferKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        #[cfg(windows)]
        let result = file_operations::write_file_clipboard(&paths, kind, window)
            .map_err(|error| error.to_string());
        #[cfg(target_os = "macos")]
        let result = if kind == TransferKind::Move {
            Err("macOSでの移動にはShiftを押しながらドラッグしてください".into())
        } else {
            crate::media::macos_actions::copy_files(&paths)
        };
        #[cfg(not(any(windows, target_os = "macos")))]
        let result: Result<(), String> =
            Err("このOSのファイルクリップボードには対応していません".into());
        self.notice = Some(match result {
            Ok(()) => if kind == TransferKind::Copy {
                "ファイルをコピーしました。保存先で貼り付けできます"
            } else {
                "移動するファイルを選びました。保存先で貼り付けてください"
            }
            .into(),
            Err(error) => error,
        });
        cx.notify();
    }

    pub(super) fn paste_files(
        &mut self,
        _: &PasteFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focused(window, cx) {
            return;
        }
        let Some(destination) = self.path().map(Path::to_path_buf) else {
            return;
        };
        self.paste_into(destination, window, cx);
    }

    pub(super) fn paste_into(
        &mut self,
        destination: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(windows)]
        match file_operations::read_file_clipboard() {
            Ok(Some(files)) => self.start_transfer(
                files.paths,
                destination,
                files.kind,
                Some(files.sequence),
                window,
                cx,
            ),
            Ok(None) => self.notice = Some("クリップボードにファイルがありません".into()),
            Err(error) => self.notice = Some(error.to_string()),
        }
        #[cfg(not(windows))]
        {
            let paths = cx
                .read_from_clipboard()
                .map(|item| {
                    item.entries()
                        .iter()
                        .filter_map(|entry| {
                            if let gpui::ClipboardEntry::ExternalPaths(paths) = entry {
                                Some(paths.paths().to_vec())
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if paths.is_empty() {
                self.notice = Some("クリップボードにファイルがありません".into());
            } else {
                self.start_transfer(paths, destination, TransferKind::Copy, None, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn promote_file_drag(
        &mut self,
        event: &gpui::DragMoveEvent<FileDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(windows)]
        {
            if event.event.pressed_button != Some(MouseButton::Left)
                || gpui::Bounds::new(gpui::point(px(0.), px(0.)), window.viewport_size())
                    .contains(&event.event.position)
            {
                return;
            }
            let Some(lease) = NativeDragLease::acquire() else {
                return;
            };
            let Some(drag) = event.dragged_item().downcast_ref::<FileDrag>() else {
                return;
            };
            let paths = drag.paths(cx);
            let directories: BTreeSet<PathBuf> = paths
                .iter()
                .filter_map(|path| path.parent().map(Path::to_path_buf))
                .collect();
            let request = file_operations::NativeFileDrag::new(paths, window);
            let payload = drag.payload.clone();
            cx.stop_active_drag(window);
            match request {
                Err(error) => {
                    self.notice = Some(error.to_string());
                    cx.notify();
                }
                Ok(request) => {
                    let origin = self.history.current().clone();
                    let listing = self.cancel.clone();
                    let initiator = cx.entity_id();
                    let events = file_transfer_events(cx);
                    cx.spawn(async move |_, cx| {
                        let _lease = lease;
                        let _payload = payload;
                        // Foreground future, outside any GPUI entity/window/app update borrow.
                        // SHDoDragDrop's nested event loop may deliver a drop into another Skylis view.
                        let result = request.run();
                        drop(_lease);
                        drop(_payload);
                        // Escapeや外へのドロップでも、元ウィンドウの半透明表示を解除する。
                        let _ = cx.update(|cx| cx.refresh_windows());
                        let completion = match result {
                            Ok(Some(_))
                                if !NATIVE_DROP_ACCEPTED
                                    .load(std::sync::atomic::Ordering::Acquire) =>
                            {
                                FileTransferCompleted {
                                    directories,
                                    notice: None,
                                }
                            }
                            Ok(_) => return, // Cancelled, or an internal worker will publish later.
                            Err(error) => FileTransferCompleted {
                                directories: BTreeSet::new(),
                                notice: Some(TransferNotice {
                                    initiator,
                                    origin,
                                    listing,
                                    text: error.to_string(),
                                }),
                            },
                        };
                        let _ = events.update(cx, |_, cx| {
                            cx.emit(completion);
                        });
                    })
                    .detach();
                }
            }
        }
        #[cfg(not(windows))]
        let _ = (event, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn native_drop_samples_modifiers_at_release_without_keyboard_events(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::InputEvent;
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let handle = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        let platform = cx.test_window(handle.into());
        let position = gpui::point(px(10.), px(10.));
        let steps = [
            (
                gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
                gpui::FileDropEvent::Entered {
                    position,
                    paths: gpui::ExternalPaths(
                        [PathBuf::from("C:\\source.txt")].into_iter().collect(),
                    ),
                },
            ),
            (
                gpui::Modifiers {
                    control: true,
                    ..Default::default()
                },
                gpui::FileDropEvent::Pending { position },
            ),
            (
                gpui::Modifiers::default(),
                gpui::FileDropEvent::Submit { position },
            ),
        ];
        for (modifiers, event) in steps {
            platform.set_modifiers(modifiers);
            handle
                .update(cx, |_, window, cx| {
                    window.dispatch_event(event.to_platform_input(), cx);
                    assert_eq!(window.modifiers(), modifiers);
                })
                .unwrap();
        }
    }

    #[test]
    fn drop_feedback_rejects_self_children_and_same_parent() {
        let source = PathBuf::from("root/source");
        let paths = vec![source.clone()];
        assert!(!valid_drop_paths(&paths, &source));
        assert!(!valid_drop_paths(&paths, &source.join("child")));
        assert!(!valid_drop_paths(&paths, Path::new("root")));
        assert!(valid_drop_paths(&paths, Path::new("other")));
        assert!(!valid_drop_paths(&[], Path::new("other")));
    }

    #[gpui::test]
    fn drag_payload_is_lazy_and_freezes_selection_and_directory_metadata(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        let mut entries = Vec::new();
        list_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            &CancelToken::new(),
            |event| {
                if let ListingEvent::Batch(batch) = event {
                    entries.extend(batch);
                }
                true
            },
        );
        assert!(entries.iter().any(|entry| entry.kind == EntryKind::Dir));
        assert!(entries.iter().any(|entry| entry.kind == EntryKind::File));
        let expected: Vec<_> = entries
            .iter()
            .map(|entry| (entry.path.clone(), entry.kind == EntryKind::Dir))
            .collect();
        let drag = window
            .update(cx, |view, _, cx| {
                view.entries = entries;
                view.visible = (0..view.entries.len()).collect();
                view.multi_selection = view
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                view.file_drag(0, cx)
            })
            .unwrap();
        assert!(
            drag.payload.get().is_none(),
            "rendering a row must not clone the selection"
        );
        cx.update(|cx| drag.resolve(cx));
        let clone = drag.clone();
        assert!(Rc::ptr_eq(&drag.payload, &clone.payload));
        window
            .update(cx, |view, _, _| {
                view.multi_selection.clear();
                view.selected = None;
                view.entries.clear();
                view.visible.clear();
            })
            .unwrap();
        cx.read(|cx| {
            assert_eq!(
                drag.paths(cx),
                expected
                    .iter()
                    .map(|(path, _)| path.clone())
                    .collect::<Vec<_>>()
            );
            let Some(gpui::ExternalDragPayload::Files(files)) = drag.external_payload(cx) else {
                panic!("missing native file payload");
            };
            assert_eq!(files.entries(), expected);
            let feedback = cx.global::<DragFeedback>();
            assert!(feedback.contains(drag.source.entity_id(), &expected[0].0, true));
            assert!(
                !feedback.contains(drag.source.entity_id(), &expected[0].0, false),
                "drop/cancel clears feedback even before the drag view is released"
            );
        });
        let payload = Rc::downgrade(&drag.payload);
        drop(drag);
        drop(clone);
        assert!(
            payload.upgrade().is_none(),
            "feedback must not retain completed payloads"
        );
    }

    fn folder(name: &str) -> Location {
        Location::Folder(PathBuf::from(name))
    }

    #[test]
    fn completion_refreshes_only_affected_directories() {
        let context = TransferContext {
            origin: folder("source"),
            destination: PathBuf::from("target"),
            sources: [PathBuf::from("source")].into(),
            listing: CancelToken::new(),
        };
        assert_eq!(
            context.affected_directories(TransferKind::Copy),
            BTreeSet::from([PathBuf::from("target")])
        );
        assert_eq!(
            context.affected_directories(TransferKind::Move),
            BTreeSet::from([PathBuf::from("source"), PathBuf::from("target")])
        );
    }

    #[gpui::test]
    fn completion_refreshes_all_matching_windows_without_stale_notices(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let windows: Vec<_> = (0..4)
            .map(|_| {
                cx.add_window(|window, cx| {
                    let icons = cx.new(ShellIcons::new);
                    FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
                })
            })
            .collect();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let destination = source.join("src");
        let locations = [
            source.clone(),
            source.clone(),
            destination.clone(),
            source.join("assets"),
        ];
        let mut tokens = Vec::new();
        for (window, location) in windows.iter().zip(&locations) {
            tokens.push(
                window
                    .update(cx, |view, _, _| {
                        view.history = History::new(Location::Folder(location.clone()));
                        view.notice = Some("existing notice".into());
                        view.cancel.clone()
                    })
                    .unwrap(),
            );
        }
        let initiator = windows[0].update(cx, |_, _, cx| cx.entity_id()).unwrap();
        cx.update(|cx| {
            let events = file_transfer_events(cx);
            events.update(cx, |_, cx| {
                cx.emit(FileTransferCompleted {
                    directories: BTreeSet::from([source.clone(), destination.clone()]),
                    notice: Some(TransferNotice {
                        initiator,
                        origin: Location::Folder(source.clone()),
                        listing: tokens[0].clone(),
                        text: "completed".into(),
                    }),
                })
            });
        });
        for (index, token) in tokens.iter().enumerate() {
            assert_eq!(
                token.is_cancelled(),
                index < 3,
                "only source and destination views refresh"
            );
        }
        windows[0]
            .update(cx, |view, _, _| {
                assert_eq!(view.notice.as_deref(), Some("completed"))
            })
            .unwrap();
        windows[3]
            .update(cx, |view, _, _| {
                assert_eq!(view.notice.as_deref(), Some("existing notice"))
            })
            .unwrap();
        // Returning to the same path still has a different generation; old notices must
        // not overwrite current UI. Refresh is nevertheless delivered to every match.
        let replacement = windows[0]
            .update(cx, |view, _, _| view.cancel.clone())
            .unwrap();
        cx.update(|cx| {
            let events = file_transfer_events(cx);
            events.update(cx, |_, cx| {
                cx.emit(FileTransferCompleted {
                    directories: BTreeSet::from([source.clone()]),
                    notice: Some(TransferNotice {
                        initiator,
                        origin: Location::Folder(source),
                        listing: tokens[0].clone(),
                        text: "stale".into(),
                    }),
                })
            });
        });
        assert!(replacement.is_cancelled());
        windows[0]
            .update(cx, |view, _, _| {
                assert_ne!(view.notice.as_deref(), Some("stale"))
            })
            .unwrap();
        let survivor = windows[2]
            .update(cx, |view, _, _| view.cancel.clone())
            .unwrap();
        windows[0]
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.update(|cx| {
            let events = file_transfer_events(cx);
            events.update(cx, |_, cx| {
                cx.emit(FileTransferCompleted {
                    directories: BTreeSet::from([destination]),
                    notice: None,
                })
            });
        });
        assert!(
            survivor.is_cancelled(),
            "completion outlives its initiating window"
        );
    }

    #[test]
    fn cut_clipboard_is_cleared_only_after_completed_move() {
        assert_eq!(
            cut_sequence_to_clear(
                TransferKind::Move,
                &Ok(TransferOutcome::Completed),
                Some(42)
            ),
            Some(42)
        );
        assert_eq!(
            cut_sequence_to_clear(
                TransferKind::Copy,
                &Ok(TransferOutcome::Completed),
                Some(42)
            ),
            None
        );
        assert_eq!(
            cut_sequence_to_clear(TransferKind::Move, &Ok(TransferOutcome::Aborted), Some(42)),
            None
        );
        assert_eq!(
            cut_sequence_to_clear(TransferKind::Move, &Err("failed".into()), Some(42)),
            None
        );
        assert_eq!(
            cut_sequence_to_clear(TransferKind::Move, &Ok(TransferOutcome::Completed), None),
            None
        );
    }

    #[gpui::test]
    fn control_click_can_remove_the_last_selected_file(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        let mut entries = Vec::new();
        list_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            &CancelToken::new(),
            |event| {
                if let ListingEvent::Batch(batch) = event {
                    entries.extend(batch);
                }
                true
            },
        );
        assert!(entries.len() >= 2);
        window
            .update(cx, |view, window, cx| {
                view.entries = entries;
                view.visible = (0..view.entries.len()).collect();
                let selected = view.entries[0].path.clone();
                view.multi_selection.insert(selected.clone());
                view.selected = Some(0);
                let control_click = MouseDownEvent {
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                view.pointer_select(0, &control_click, window, cx);
                assert!(view.multi_selection.is_empty());
                assert!(view.selected.is_none());
                assert!(view.selected_paths().is_empty());
                // A focus cursor is not sufficient to reselect a deselected file for Copy/Drag.
                view.selected = Some(0);
                assert!(view.selected_paths().is_empty());
                view.pointer_select(0, &control_click, window, cx);
                assert_eq!(view.selected_paths(), [selected.clone()]);
                view.multi_selection.insert(view.entries[1].path.clone());
                let down = MouseDownEvent {
                    button: MouseButton::Left,
                    ..Default::default()
                };
                view.pointer_select(0, &down, window, cx);
                assert_eq!(
                    view.selected_paths().len(),
                    2,
                    "mouse-down preserves the drag selection"
                );
                view.finish_pointer_click(
                    0,
                    &ClickEvent::Mouse(gpui::MouseClickEvent {
                        down,
                        up: gpui::MouseUpEvent {
                            button: MouseButton::Left,
                            ..Default::default()
                        },
                    }),
                    cx,
                );
                assert_eq!(
                    view.selected_paths(),
                    [selected.clone()],
                    "a completed plain click collapses it"
                );
                // Reorder the listing as a streamed batch/sort would. Shift must follow
                // the anchor file rather than its old visible row number.
                assert_eq!(view.selection_anchor.as_ref(), Some(&selected));
                view.visible.swap(0, 1);
                let shift = MouseDownEvent {
                    modifiers: gpui::Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                view.pointer_select(1, &shift, window, cx);
                assert_eq!(view.selected_paths(), [selected]);
            })
            .unwrap();
    }
}
