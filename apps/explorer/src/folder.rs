use std::ops::Range;
#[cfg(test)]
mod tests;
use std::path::{Path, PathBuf};

mod home;
mod marquee;
mod selection_geometry;
mod transfers;
use transfers::FileDrag;
mod sizes;
use sizes::SizeState;
mod grouping;
mod media_view;
#[cfg(not(target_os = "macos"))]
mod preview_window;
mod wsl;
use crate::media::{MediaCache, MediaScope};
mod archives;
mod context_menu;
mod delete;
pub(crate) mod display_settings;
mod name_editor;
mod name_operations;
mod scroll_motion;
mod scrollbar;
mod watch;
use context_menu::ContextMenu;
use explorer_core::history::breadcrumbs;
use explorer_core::location::{Location, resolve_address};
use explorer_core::platform::{self, DriveSpace, Place};
use explorer_core::{
    CancelToken, Entry, EntryKind, History, ListingEvent, ListingStats, SortKey, SortOrder, format,
    list_dir,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, MouseButton, MouseDownEvent,
    NavigationDirection, Pixels, ScrollStrategy, Task, UniformListScrollHandle, Window, div,
    prelude::*, px, svg, uniform_list,
};
use grouping::DisplayRow;
use std::collections::HashMap;

use crate::actions::*;
use crate::search::TextInput;
use crate::shell_icons::ShellIcons;
use crate::theme::{Theme, ThemeMode};
use gpui::{ClipboardItem, Entity, img};

const COL_MODIFIED: Pixels = px(144.);
const COL_KIND: Pixels = px(136.);
const COL_SIZE: Pixels = px(92.);
const SIDEBAR_WIDTH: Pixels = px(200.);

pub enum FolderEvent {
    OpenInTab(PathBuf),
    SetPane(bool),
}
impl gpui::EventEmitter<FolderEvent> for FolderView {}

pub struct FolderView {
    display_settings: display_settings::DisplaySettings,
    marquee: marquee::MarqueeState,
    motion: scroll_motion::FolderMotion,
    multi_selection: std::collections::BTreeSet<PathBuf>,
    selection_anchor: Option<PathBuf>,
    sizes: SizeState,
    media: Entity<MediaCache>,
    media_scope: MediaScope,
    preview_scope: MediaScope,
    inspector_image: std::cell::RefCell<Option<(Entry, std::sync::Arc<gpui::RenderImage>)>>,
    preview_open: bool,
    #[cfg(not(target_os = "macos"))]
    preview_window: Option<gpui::WindowHandle<preview_window::PreviewWindow>>,
    gallery: bool,
    gallery_override: Option<bool>,
    gallery_columns: usize,
    gallery_rows: Vec<(Option<explorer_core::date_group::DateGroup>, Vec<usize>)>,
    context_menu: Option<ContextMenu>,
    name_editor: Option<name_editor::NameEditor>,
    archive_index: Option<std::sync::Arc<explorer_core::archive::ArchiveIndex>>,
    archive_dialog: Option<archives::ArchiveDialog>,
    archive_jobs: Entity<crate::archive_jobs::ArchiveJobs>,
    transfer_progress: Entity<transfers::TransferProgress>,
    menu_generation: usize,
    pane_generation: usize,
    pane_revealed_at: Option<std::time::Instant>,
    menu_focus: FocusHandle,
    _shell_menu: Option<Task<()>>,
    _open_request: Option<Task<()>>,
    pub mode: ThemeMode,
    pub pane: bool,
    pub compact: bool,
    search: Entity<TextInput>,
    query: String,
    icons: Entity<ShellIcons>,
    focus: FocusHandle,
    /// Explicit host: GPUI's entity/window mapping advances only after a paint, whereas
    /// async results can arrive immediately after a tab has moved between windows.
    owner_window: gpui::AnyWindowHandle,
    _window_subscriptions: Vec<gpui::Subscription>,
    history: History<Location>,
    address: Entity<TextInput>,
    editing_address: bool,
    address_error: Option<String>,
    address_pending: bool,
    address_generation: u64,
    _address_validation: Option<Task<()>>,
    _drive_task: Option<Task<()>>,
    _wsl_task: Option<Task<()>>,
    wsl: Vec<explorer_core::wsl::WslDistribution>,
    /// 現在のフォルダーの全項目（隠しファイル含む）、常にソート済み
    entries: Vec<Entry>,
    /// 表示対象の entries インデックス（隠しファイルのフィルター後）
    visible: Vec<usize>,
    display_rows: Vec<DisplayRow>,
    entry_rows: Vec<usize>,
    grouped: bool,
    group_date: chrono::NaiveDate,
    group_downloads: bool,
    /// visible 上の選択位置
    selected: Option<usize>,
    /// 列挙中に現れたら選択したい項目名（戻る/上へ で元のフォルダーを選択するため）
    pending_select: Option<String>,
    startup_handoff: Option<crate::shell_handoff::PendingHandoff>,
    launch_selection: bool,
    launch_reveal: Option<String>,
    sort: SortOrder,
    show_hidden: bool,
    scroll: UniformListScrollHandle,
    scrollbar_drag: Option<f32>,
    scrollbar_hovered: bool,
    loading: bool,
    stats: Option<ListingStats>,
    error: Option<String>,
    pub(crate) permission_observation: crate::macos_permissions::Notice,
    notice: Option<String>,
    cancel: CancelToken,
    _listing: Option<Task<()>>,
    watch_state: watch::WatchState,
    favorites: Vec<Place>,
    pinned: Vec<Place>,
    // Bounded by the settings pin limit; stable across paints and keyboard traversal.
    pin_remove_focus: Vec<FocusHandle>,
    drives: Vec<Place>,
    drive_space: HashMap<PathBuf, Option<DriveSpace>>,
    title_dirty: bool,
}

impl FolderView {
    pub fn new(
        start: Location,
        mode: ThemeMode,
        icons: Entity<ShellIcons>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let search = cx.new(TextInput::new);
        cx.observe(&search, |this, search, cx| {
            let query = search.read(cx).content.to_string();
            if query != this.query {
                this.query = query;
                let keep = this.selected_entry().map(|e| e.path.clone());
                this.rebuild_visible(keep.as_deref());
                this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        })
        .detach();
        let address = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.placeholder = "フォルダーのパスを入力…".into();
            input
        });
        cx.observe(&address, |this, _, cx| {
            this.address_generation = this.address_generation.wrapping_add(1);
            this._address_validation = None;
            this.address_pending = false;
            this.address_error = None;
            cx.notify();
        })
        .detach();
        cx.observe(&icons, |_, _, cx| cx.notify()).detach();
        let preferences = cx
            .try_global::<crate::settings::SharedPreferences>()
            .map(|p| p.0.clone());
        let group_downloads = preferences
            .as_ref()
            .map_or(true, |p| p.read(cx).value.group_downloads);
        let pinned = preferences.as_ref().map_or_else(Vec::new, |p| {
            home::pinned_places(&p.read(cx).value.pinned_folders)
        });
        let show_hidden = preferences.as_ref().is_some_and(|p| p.read(cx).show_hidden);
        if let Some(preferences) = preferences {
            cx.observe(&preferences, |this, preferences, cx| {
                this.set_hidden_visibility(preferences.read(cx).show_hidden, cx);
                this.compact = preferences.read(cx).value.compact;
                let paths = &preferences.read(cx).value.pinned_folders;
                if this.pinned.len() != paths.len()
                    || this
                        .pinned
                        .iter()
                        .zip(paths)
                        .any(|(place, path)| place.path != *path)
                {
                    this.pinned = home::pinned_places(paths);
                    if this.is_home() {
                        this.selected = None;
                    }
                    cx.notify();
                }
                if !preferences.read(cx).value.smooth_scroll {
                    this.cancel_scroll_motion();
                }
                let automatic = preferences.read(cx).value.automatic_sizes;
                this.update_size_mode(automatic, cx);
                this.choose_auto_gallery(cx);
                let enabled = preferences.read(cx).value.group_downloads;
                if this.group_downloads != enabled {
                    this.group_downloads = enabled;
                    this.grouped = enabled && this.is_downloads();
                    let keep = this.selected_entry().map(|e| e.path.clone());
                    this.rebuild_visible(keep.as_deref());
                    cx.notify();
                }
            })
            .detach();
        }
        let media = if let Some(shared) = cx.try_global::<media_view::SharedMedia>() {
            shared.0.clone()
        } else {
            let cache = cx.new(MediaCache::new);
            cx.set_global(media_view::SharedMedia(cache.clone()));
            cache
        };
        cx.observe(&media, |_, _, cx| cx.notify()).detach();
        let archive_jobs = crate::archive_jobs::init(cx);
        cx.observe(&archive_jobs, |_, _, cx| cx.notify()).detach();
        let transfer_progress = transfers::transfer_progress(cx);
        cx.observe(&transfer_progress, |_, _, cx| cx.notify())
            .detach();
        let mut this = Self {
            display_settings: display_settings::DisplaySettings::new(cx),
            marquee: Default::default(),
            motion: Default::default(),
            multi_selection: Default::default(),
            selection_anchor: None,
            sizes: SizeState::default(),
            media,
            media_scope: MediaScope::new(),
            preview_scope: MediaScope::new(),
            inspector_image: Default::default(),
            preview_open: false,
            #[cfg(not(target_os = "macos"))]
            preview_window: None,
            gallery: false,
            gallery_override: None,
            gallery_columns: 1,
            gallery_rows: Vec::new(),
            context_menu: None,
            name_editor: None,
            archive_index: None,
            archive_dialog: None,
            archive_jobs,
            transfer_progress,
            menu_generation: 0,
            pane_generation: 0,
            pane_revealed_at: None,
            menu_focus: cx.focus_handle(),
            _shell_menu: None,
            _open_request: None,
            mode,
            pane: true,
            compact: false,
            search,
            address,
            editing_address: false,
            address_error: None,
            address_pending: false,
            address_generation: 0,
            _address_validation: None,
            _drive_task: None,
            _wsl_task: None,
            wsl: Vec::new(),
            query: String::new(),
            icons,
            focus,
            owner_window: window.window_handle(),
            _window_subscriptions: Vec::new(),
            history: History::new(start.clone()),
            entries: Vec::new(),
            visible: Vec::new(),
            display_rows: Vec::new(),
            entry_rows: Vec::new(),
            grouped: false,
            group_date: chrono::Local::now().date_naive(),
            group_downloads,
            selected: None,
            pending_select: None,
            startup_handoff: None,
            launch_selection: false,
            launch_reveal: None,
            sort: SortOrder::default(),
            show_hidden,
            scroll: UniformListScrollHandle::new(),
            scrollbar_drag: None,
            scrollbar_hovered: false,
            loading: false,
            stats: None,
            error: None,
            permission_observation: Default::default(),
            notice: None,
            cancel: CancelToken::new(),
            _listing: None,
            watch_state: Default::default(),
            favorites: platform::favorite_places(),
            pinned,
            pin_remove_focus: (0..crate::settings_data::MAX_PINS)
                .map(|_| cx.focus_handle())
                .collect(),
            drives: platform::drives(),
            drive_space: HashMap::new(),
            title_dirty: true,
        };
        this.attach_window(window, cx);
        this.observe_file_transfers(cx);
        this.observe_archive_jobs(cx);
        this.refresh_drives(cx);
        this.discover_wsl(cx);
        this.load(start, None, cx);
        this
    }

    pub fn attach_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_display_settings(cx);
        if self.owner_window != window.window_handle() {
            self.owner_window = window.window_handle();
            self.attach_name_editor_window(window, cx);
        }
        self.attach_motion_window(window);
        self._window_subscriptions.clear();
        self._window_subscriptions
            .push(cx.observe_window_activation(window, |this, w, cx| {
                if !w.is_window_active() {
                    this.cancel_scroll_motion();
                    cx.notify();
                }
            }));
        self._window_subscriptions
            .push(
                cx.observe_window_visibility(window, |this, visibility, _, _| {
                    if !visibility.is_visible() {
                        this.cancel_scroll_motion();
                    }
                }),
            );
        self._window_subscriptions
            .push(cx.on_focus(&self.focus, window, |this, _, cx| {
                let date = chrono::Local::now().date_naive();
                if date != this.group_date {
                    this.group_date = date;
                    this.rebuild_display_rows();
                }
                cx.notify();
            }));
        self._window_subscriptions
            .push(cx.on_blur(&self.focus, window, |this, _, cx| {
                this.cancel_scroll_motion();
                cx.notify();
            }));
        self._window_subscriptions
            .push(cx.observe_window_appearance(window, |_, _, cx| cx.notify()));
        self._window_subscriptions.push(cx.on_blur(
            &self.address.focus_handle(cx),
            window,
            |this, _, cx| this.cancel_address(cx),
        ));
    }

    pub(super) fn with_owner<R>(
        view: &gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        update: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> anyhow::Result<R> {
        let owner = view.read_with(cx, |view, _| view.owner_window)?;
        gpui::AppContext::update_window(cx, owner, |_, window, cx| {
            view.update(cx, |view, cx| update(view, window, cx))
        })?
    }

    fn refresh_drives(&mut self, cx: &mut Context<Self>) {
        self.drives = platform::drives();
        self.drive_space.clear();
        let paths: Vec<_> = self.drives.iter().map(|p| p.path.clone()).collect();
        #[cfg(target_os = "macos")]
        let paths = {
            let mut paths = paths;
            let home = platform::home_dir();
            if !paths.contains(&home) {
                paths.push(home);
            }
            paths
        };
        let (tx, rx) = async_channel::bounded(paths.len().max(1));
        let _ = std::thread::Builder::new()
            .name("drive-space".into())
            .spawn(move || {
                for path in paths {
                    let space = platform::drive_space(&path);
                    if tx.send_blocking((path, space)).is_err() {
                        break;
                    }
                }
            });
        self._drive_task = Some(cx.spawn(async move |view, cx| {
            while let Ok((path, space)) = rx.recv().await {
                if view
                    .update(cx, |view, cx| {
                        view.drive_space.insert(path, space.ok());
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    // ───────────── 読み込み ─────────────

    pub(crate) fn configure_launch(
        &mut self,
        selected_name: Option<String>,
        handoff: Option<crate::shell_handoff::PendingHandoff>,
        notice: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.launch_selection = selected_name.is_some();
        if selected_name.is_some() {
            self.launch_reveal = selected_name.clone();
            self.pending_select = selected_name;
            self.rebuild_visible(None);
        }
        self.startup_handoff = handoff;
        self.notice = notice;
        cx.notify();
    }

    fn finish_launch(&mut self, cx: &mut Context<Self>) {
        if self.launch_selection && self.pending_select.is_some() {
            self.notice = Some("指定された項目が見つからないため、親フォルダーを開きました".into());
        }
        self.launch_selection = false;
        if let Some(handoff) = self.startup_handoff.as_mut() {
            let receiver = handoff.complete();
            let generation = self.cancel.clone();
            cx.spawn(async move |view, cx| {
                if let Ok(message) = receiver.recv().await {
                    let _ = view.update(cx, |view, cx| {
                        if !generation.is_cancelled() {
                            view.startup_handoff = None;
                            view.notice = Some(message);
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    fn load(&mut self, location: Location, select: Option<String>, cx: &mut Context<Self>) {
        self.close_display_settings(cx);
        self.archive_dialog = None;
        self.startup_handoff = None;
        self.launch_selection = false;
        self.launch_reveal = None;
        self.name_editor = None;
        self.cancel_scroll_motion();
        self.cancel_address(cx);
        self.cancel_sizes(cx);
        self.media_scope.cancel();
        self.close_preview(cx);
        self.inspector_image.borrow_mut().take();
        self.gallery = self.gallery_override.unwrap_or(false);
        self.gallery_rows.clear();
        self.context_menu = None;
        self._shell_menu = None;
        self._open_request = None;
        // 前の列挙を止める（タスクをdropすると受信側も閉じ、列挙スレッドは次のsendで終了する）
        self.cancel.cancel();
        self.cancel = CancelToken::new();
        self._listing = None;

        self.grouped = self.group_downloads && self.is_downloads();
        self.group_date = chrono::Local::now().date_naive();
        self.display_rows.clear();
        self.entry_rows.clear();
        self.multi_selection.clear();
        self.selection_anchor = None;
        self.entries.clear();
        self.visible.clear();
        self.selected = None;
        self.pending_select = select;
        self.loading = true;
        self.stats = None;
        self.error = None;
        self.permission_observation = Default::default();
        self.notice = None;
        self.title_dirty = true;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);

        self.update_watch(location.path(), cx);
        if let Location::Archive { archive, prefix } = &location {
            self.gallery = false;
            self.load_archive(archive.clone(), prefix.clone(), cx);
            return;
        }
        self.archive_index = None;
        let Location::Folder(path) = location else {
            self.selected = self.pending_select.take().and_then(|path| {
                self.home_places()
                    .iter()
                    .position(|p| p.path.display().to_string() == path)
            });
            self.loading = false;
            cx.notify();
            return;
        };

        let (tx, rx) = async_channel::bounded::<ListingEvent>(16);
        let token = self.cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("list-dir".into())
            .spawn(move || {
                list_dir(&path, &token, |ev| tx.send_blocking(ev).is_ok());
            });
        if let Err(e) = spawned {
            self.loading = false;
            self.error = Some(format!("列挙スレッドを起動できません: {e}"));
            return;
        }

        let listing_token = self.cancel.clone();
        self._listing = Some(cx.spawn(async move |this, cx| {
            while let Ok(ev) = rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.on_requested_listing_event(ev, &listing_token, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn on_requested_listing_event(
        &mut self,
        ev: ListingEvent,
        token: &CancelToken,
        cx: &mut Context<Self>,
    ) {
        if !token.is_cancelled() {
            self.on_listing_event(ev, cx);
        }
    }

    fn on_listing_event(&mut self, ev: ListingEvent, cx: &mut Context<Self>) {
        match ev {
            ListingEvent::Batch(mut batch) => {
                let selected_path = self.selected_entry().map(|e| e.path.clone());
                let sort = self.sort;
                batch.sort_unstable_by(|a, b| sort.compare(a, b));
                self.entries = merge_sorted(std::mem::take(&mut self.entries), batch, sort);
                self.rebuild_visible(selected_path.as_deref());

                if let Some(name) = &self.pending_select {
                    if let Some(ix) = self
                        .visible
                        .iter()
                        .position(|&i| &self.entries[i].name == name)
                    {
                        self.select(ix, cx);
                        self.scroll_to_entry(ix, ScrollStrategy::Center);
                        self.pending_select = None;
                    }
                }
            }
            ListingEvent::Done(stats) => {
                self.permission_observation.record(Ok(()), stats.cancelled);
                self.loading = false;
                self.stats = Some(stats);
                self.choose_auto_gallery(cx);
                if !stats.cancelled {
                    self.finish_launch(cx);
                } else {
                    self.startup_handoff = None;
                }
                self.pending_select = None;
            }
            ListingEvent::Failed(kind, msg) => {
                self.permission_observation.record(Err(kind), false);
                self.startup_handoff = None;
                self.loading = false;
                self.error = Some(match kind {
                    std::io::ErrorKind::PermissionDenied => {
                        "このフォルダーにアクセスする権限がありません".into()
                    }
                    std::io::ErrorKind::NotFound => "フォルダーが見つかりません".into(),
                    _ => format!("開けませんでした: {msg}"),
                });
            }
        }
        cx.notify();
    }

    /// visible を作り直し、選択を同じパスの項目に合わせ直す
    fn rebuild_visible(&mut self, keep: Option<&Path>) {
        let show_hidden = self.show_hidden;
        let revealed = self.launch_reveal.as_deref();
        let query = self.query.to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (show_hidden || !e.hidden || revealed == Some(e.name.as_str()))
                    && e.name.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        self.apply_grouping();
        self.selected =
            keep.and_then(|p| self.visible.iter().position(|&i| self.entries[i].path == p));
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.selected
            .and_then(|ix| self.visible.get(ix))
            .map(|&i| &self.entries[i])
    }

    fn selected_name(&self) -> Option<String> {
        if self.is_home() {
            self.selected.and_then(|i| {
                self.home_places()
                    .get(i)
                    .map(|p| p.path.display().to_string())
            })
        } else {
            self.selected_entry().map(|e| e.name.clone())
        }
    }

    // ───────────── ナビゲーション ─────────────

    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.search.update(cx, |input, cx| input.reset(window, cx));
        window.focus(&self.focus, cx);
    }

    fn navigate_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Location::Folder(path), window, cx);
    }

    fn navigate(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        if &location == self.history.current() {
            return;
        }
        self.gallery_override = None;
        self.history.push(location.clone(), self.selected_name());
        self.clear_query(window, cx);
        self.load(location, None, cx);
    }

    fn open_entry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_home() {
            if self.recycle_bin_index() == Some(ix) {
                if let Err(error) = platform::open_recycle_bin() {
                    self.notice = Some(format!("ゴミ箱を開けませんでした: {error}"));
                    cx.notify();
                }
                return;
            }
            if let Some(place) = self.home_places().get(ix) {
                self.navigate_to(place.path.clone(), window, cx);
            }
            return;
        }
        let Some(&i) = self.visible.get(ix) else {
            return;
        };
        let entry = &self.entries[i];
        let path = entry.path.clone();
        if let Location::Archive { archive, .. } = self.history.current() {
            if entry.kind == EntryKind::Dir {
                self.navigate(
                    Location::Archive {
                        archive: archive.clone(),
                        prefix: path.to_string_lossy().replace('\\', "/"),
                    },
                    window,
                    cx,
                );
            } else {
                self.notice = Some("この項目を開くには、右クリックから解凍してください".into());
                cx.notify();
            }
            return;
        }
        if entry.kind == EntryKind::File && explorer_core::archive::detect_format(&path).is_some() {
            self.navigate(
                Location::Archive {
                    archive: path,
                    prefix: String::new(),
                },
                window,
                cx,
            );
            return;
        }
        if entry.kind == EntryKind::Dir {
            self.navigate_to(path, window, cx);
        } else {
            if self._open_request.is_some() {
                return;
            }
            let generation = self.cancel.clone();
            match crate::file_open::request(
                path,
                entry.kind == EntryKind::Symlink,
                generation.clone(),
            ) {
                Ok(receiver) => {
                    self.observe_open_request(receiver, generation, cx);
                }
                Err(error) => {
                    self.notice = Some(error);
                    cx.notify();
                }
            }
        }
    }

    fn observe_open_request(
        &mut self,
        receiver: async_channel::Receiver<Result<crate::file_open::OpenOutcome, String>>,
        generation: CancelToken,
        cx: &mut Context<Self>,
    ) {
        self._open_request = Some(cx.spawn(async move |view, cx| {
            let result = receiver.recv().await;
            let _ = Self::with_owner(&view, cx, |view, window, cx| {
                if generation.is_cancelled() {
                    return;
                }
                view._open_request = None;
                match result {
                    Ok(Ok(crate::file_open::OpenOutcome::Directory(path))) => {
                        let focused = view.focus.contains_focused(window, cx);
                        let previous = window.focused(cx);
                        view.navigate_to(path, window, cx);
                        // Completing a slow link lookup in an inactive tab must not take
                        // keyboard focus away from the tab the user switched to.
                        if !focused && let Some(previous) = previous {
                            window.focus(&previous, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        view.notice = Some(format!("開けませんでした: {error}"));
                    }
                    _ => (),
                }
                cx.notify();
            });
        }));
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((path, sel)) = self.history.go_back(self.selected_name()) {
            self.gallery_override = None;
            self.clear_query(window, cx);
            self.load(path, sel, cx);
        }
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((path, sel)) = self.history.go_forward(self.selected_name()) {
            self.gallery_override = None;
            self.clear_query(window, cx);
            self.load(path, sel, cx);
        }
    }

    fn go_up(&mut self, _: &GoUp, window: &mut Window, cx: &mut Context<Self>) {
        if let Location::Archive { archive, prefix } = self.history.current().clone() {
            if prefix.is_empty() {
                let parent = archive
                    .parent()
                    .unwrap_or_else(|| Path::new("/"))
                    .to_path_buf();
                let name = archive
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                let location = Location::Folder(parent);
                self.history.push(location.clone(), self.selected_name());
                self.clear_query(window, cx);
                self.load(location, name, cx);
            } else {
                let parent = prefix.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
                self.navigate(
                    Location::Archive {
                        archive,
                        prefix: parent,
                    },
                    window,
                    cx,
                );
            }
            return;
        }
        let Some(current) = self.path().map(Path::to_path_buf) else {
            return;
        };
        if let Some(parent) = current.parent() {
            let came_from = current
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            let location = Location::Folder(parent.to_path_buf());
            self.gallery_override = None;
            self.history.push(location.clone(), self.selected_name());
            self.clear_query(window, cx);
            self.load(location, came_from, cx);
        } else {
            self.navigate(Location::Home, window, cx);
        }
    }

    fn refresh(&mut self, _: &Refresh, w: &mut Window, cx: &mut Context<Self>) {
        if self.close_display_settings(cx) {
            w.focus(&self.focus, cx);
        }
        self.permission_observation.retry();
        self.refresh_drives(cx);
        if self.path().is_some() && !self.loading {
            self.request_refresh(cx);
            return;
        }
        let location = self.history.current().clone();
        let sel = self.selected_name();
        self.load(location, sel, cx);
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = !cx
            .try_global::<crate::settings::SharedPreferences>()
            .map_or(self.show_hidden, |p| p.0.read(cx).show_hidden);
        if let Some(preferences) = cx
            .try_global::<crate::settings::SharedPreferences>()
            .map(|p| p.0.clone())
        {
            preferences.update(cx, |p, cx| p.set_show_hidden(enabled, cx));
        }
        self.set_hidden_visibility(enabled, cx);
    }

    fn set_hidden_visibility(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.show_hidden == enabled {
            return;
        }
        self.launch_reveal = None;
        self.show_hidden = enabled;
        let keep = self.selected_entry().map(|e| e.path.clone());
        self.rebuild_visible(keep.as_deref());
        cx.notify();
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.apply_sort(self.sort.toggled(key), cx);
    }

    fn apply_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        if self.sort == order && !self.grouped {
            return;
        }
        self.grouped = false;
        self.sort = order;
        let keep = self.selected_entry().map(|e| e.path.clone());
        let sort = self.sort;
        self.entries.sort_unstable_by(|a, b| sort.compare(a, b));
        self.rebuild_visible(keep.as_deref());
        if let Some(ix) = self.selected {
            self.scroll_to_entry(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    // ───────────── 選択移動 ─────────────

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.item_count();
        if len == 0 {
            return;
        }
        let next = match self.selected {
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
            Some(cur) => (cur as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        self.select(next, cx);
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.marquee_in_progress() {
            return;
        }
        self.preview_scope.cancel();
        self.multi_selection.clear();
        if let Some(&i) = self.visible.get(ix) {
            self.multi_selection.insert(self.entries[i].path.clone());
        }
        self.selection_anchor = self.visible.get(ix).map(|&i| self.entries[i].path.clone());
        self.selected = Some(ix);
        if !self.is_home() {
            self.scroll_to_entry(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn page_rows(&self) -> isize {
        let h = self.scroll.0.borrow().base_handle.bounds().size.height;
        let row_height = if self.gallery {
            if self.compact { 156. } else { 172. }
        } else if self.compact {
            24.
        } else {
            28.
        };
        ((h / px(row_height)).floor() as isize - 1).max(1)
            * if self.gallery {
                self.gallery_columns as isize
            } else {
                1
            }
    }

    fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        self.move_selection(
            if self.gallery && !self.preview_open {
                -(self.gallery_columns as isize)
            } else {
                -1
            },
            cx,
        );
    }
    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        self.move_selection(
            if self.gallery && !self.preview_open {
                self.gallery_columns as isize
            } else {
                1
            },
            cx,
        );
    }
    fn select_first(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        if self.item_count() > 0 {
            self.select(0, cx);
        }
    }
    fn select_last(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        if self.item_count() > 0 {
            self.select(self.item_count() - 1, cx);
        }
    }
    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        let n = self.page_rows();
        self.move_selection(-n, cx);
    }
    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(window, cx) {
            return;
        }
        let n = self.page_rows();
        self.move_selection(n, cx);
    }
    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.address.focus_handle(cx).is_focused(window) {
            self.submit_address(window, cx);
            return;
        }
        if self.search.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if let Some(ix) = self.selected {
            if self.preview_open {
                self.close_preview(cx);
            }
            self.open_entry(ix, window, cx);
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.history.current().path()
    }
    pub fn is_home(&self) -> bool {
        matches!(self.history.current(), Location::Home)
    }
    pub fn is_archive(&self) -> bool {
        matches!(self.history.current(), Location::Archive { .. })
    }
    pub fn label(&self) -> String {
        if let Location::Archive { archive, prefix } = self.history.current() {
            return if prefix.is_empty() {
                archive
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                prefix.rsplit('/').next().unwrap_or(prefix).into()
            };
        }
        self.path()
            .map(|p| {
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .unwrap_or_else(|| "ホーム".into())
    }
    fn item_count(&self) -> usize {
        if self.is_home() {
            self.home_places().len() + usize::from(self.recycle_bin_index().is_some())
        } else {
            self.visible.len()
        }
    }
    fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.display_settings.open
            || self.name_editor.is_some()
            || self.archive_dialog.is_some()
            || self.search.focus_handle(cx).is_focused(window)
            || self.address.focus_handle(cx).is_focused(window)
    }
    pub fn cancel_address(&mut self, cx: &mut Context<Self>) {
        self.editing_address = false;
        self.address_error = None;
        self.address_pending = false;
        self.address_generation = self.address_generation.wrapping_add(1);
        self._address_validation = None;
        cx.notify();
    }
    fn edit_address(&mut self, _: &EditAddress, window: &mut Window, cx: &mut Context<Self>) {
        self.close_display_settings(cx);
        let text = self
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.address
            .update(cx, |input, cx| input.set_text(text, true, window, cx));
        self.editing_address = true;
        window.focus(&self.address.focus_handle(cx), cx);
        cx.notify();
    }
    fn submit_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.address.read(cx).is_composing() || self.address_pending {
            return;
        }
        let home = platform::home_dir();
        let result = resolve_address(
            &self.address.read(cx).content,
            self.path().unwrap_or(&home),
            &home,
        );
        let path = match result {
            Ok(path) => path,
            Err(message) => {
                self.address_error = Some(message);
                cx.notify();
                return;
            }
        };
        self.address_generation = self.address_generation.wrapping_add(1);
        let generation = self.address_generation;
        self.address_pending = true;
        self.address_error = None;
        let candidate = path.clone();
        let validate = cx.background_executor().spawn(async move {
            std::fs::metadata(candidate).and_then(|metadata| {
                if metadata.is_dir() {
                    Ok(())
                } else {
                    Err(std::io::Error::other("フォルダーではありません"))
                }
            })
        });
        self._address_validation = Some(cx.spawn_in(window, async move |view, cx| {
            let result = validate.await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this.address_generation != generation || !this.editing_address {
                    return;
                }
                this.address_pending = false;
                match result {
                    Ok(()) => {
                        this.editing_address = false;
                        this.address_error = None;
                        window.focus(&this.focus, cx);
                        this.navigate_to(path, window, cx);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                        // Attempt the requested location through the listing path so the denial
                        // retains its kind and offers the same recovery UI as sidebar navigation.
                        this.editing_address = false;
                        window.focus(&this.focus, cx);
                        this.navigate_to(path, window, cx);
                    }
                    Err(error) => {
                        this.address_error = Some(format!("開けませんでした: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn theme(&self, window: &Window) -> Theme {
        Theme::resolve(self.mode, window.appearance())
    }
    fn object_icon(
        &self,
        path: &Path,
        dir: bool,
        logical: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let physical = (logical * window.scale_factor()).ceil() as u32;
        let image = self.icons.update(cx, |icons, cx| {
            if self.is_archive() && !path.is_absolute() {
                icons.get_type(path, dir, physical)
            } else {
                icons.get(path, dir, physical, cx)
            }
        });
        div()
            .size(px(logical))
            .flex_shrink_0()
            .when_some(image, |d, image| d.child(img(image).size_full()))
            .into_any_element()
    }
    fn navigation_icon(
        &self,
        path: &Path,
        semantic: Option<platform::PlaceIcon>,
        logical: f32,
        color: gpui::Rgba,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(target_os = "macos")]
        if let Some(semantic) = semantic {
            let asset = match semantic {
                platform::PlaceIcon::Home => "icons/home.svg",
                platform::PlaceIcon::Desktop => "icons/desktop.svg",
                platform::PlaceIcon::Documents => "icons/documents.svg",
                platform::PlaceIcon::Downloads => "icons/downloads.svg",
                platform::PlaceIcon::Pictures => "icons/pictures.svg",
                platform::PlaceIcon::Drive => "icons/drive.svg",
            };
            return svg()
                .path(asset)
                .size(px(logical))
                .flex_shrink_0()
                .text_color(color)
                .into_any_element();
        }
        let _ = (semantic, color);
        let image = self.icons.update(cx, |icons, cx| {
            icons.get_navigation(
                path,
                true,
                (logical * window.scale_factor()).ceil() as u32,
                cx,
            )
        });
        div()
            .size(px(logical))
            .flex_shrink_0()
            .when_some(image, |d, image| d.child(img(image).size_full()))
            .into_any_element()
    }

    fn render_toolbar(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let all = self.path().map(breadcrumbs).unwrap_or_default();
        let n = all.len();
        let crumbs: Vec<_> = all
            .into_iter()
            .enumerate()
            .filter(|(i, _)| n <= 5 || *i == 0 || *i >= n - 3)
            .collect();
        let mut address = div()
            .id("breadcrumbs")
            .debug_selector(|| "breadcrumbs".into())
            .flex()
            .items_center()
            .h(px(36.))
            .flex_1()
            .min_w_0()
            .px(px(if window.viewport_size().width < px(560.) {
                4.
            } else {
                10.
            }))
            .bg(t.c("surface.field"))
            .border_1()
            .border_color(t.c("border.subtle"))
            .rounded(px(18.))
            .overflow_x_scroll();
        address = address
            .cursor_text()
            .on_click(cx.listener(|this, _, w, cx| {
                if !this.editing_address {
                    this.edit_address(&EditAddress, w, cx);
                }
            }))
            .when(self.editing_address, |d| {
                d.on_mouse_down_out(cx.listener(|this, _, w, cx| {
                    this.cancel_address(cx);
                    if this.address.focus_handle(cx).is_focused(w) {
                        w.focus(&this.focus, cx);
                    }
                }))
            });
        if self.editing_address {
            address = address
                .border_color(t.c(if self.address_error.is_some() {
                    "danger.fg"
                } else {
                    "focus.ring"
                }))
                .child(self.address.clone());
        } else if let Location::Archive { archive, prefix } = self.history.current() {
            address = address.child(div().truncate().child(format!(
                "{}  ›  {}",
                archive.display(),
                if prefix.is_empty() {
                    "圧縮ファイル"
                } else {
                    prefix
                }
            )));
        } else if let Some(path) = self.path() {
            address =
                address.child(self.navigation_icon(path, None, 18., t.c("accent.fg"), window, cx));
        } else {
            address = address
                .child(
                    svg()
                        .path("icons/home.svg")
                        .size(px(18.))
                        .text_color(t.c("icon.default")),
                )
                .child(div().ml(px(10.)).child("ホーム"));
        }
        for (i, (label, path)) in crumbs.into_iter().filter(|_| !self.editing_address) {
            let last = i + 1 == n;
            if n > 5 && i == n - 3 {
                address = address.child(
                    div()
                        .id("breadcrumb-omission")
                        .debug_selector(|| "breadcrumb-omission".into())
                        .flex_shrink_0()
                        .px(px(6.))
                        .text_color(t.c("text.secondary"))
                        .child("…"),
                );
            }
            address = address
                .when(i > 0, |d| {
                    d.child(
                        svg()
                            .path("icons/chevron-right.svg")
                            .size(px(12.))
                            .mx(px(3.))
                            .text_color(t.c("text.disabled")),
                    )
                })
                .child(
                    div()
                        .id(("crumb", i))
                        .flex()
                        .items_center()
                        .h(px(26.))
                        .px(px(if window.viewport_size().width < px(560.) {
                            4.
                        } else {
                            10.
                        }))
                        .max_w(px(180.))
                        .flex_shrink_0()
                        .rounded(px(13.))
                        .when(last, |d| {
                            d.bg(t.c("surface.chip-selected"))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                        })
                        .text_color(t.c("text.secondary"))
                        .hover(|s| s.bg(t.c("state.hover")))
                        .cursor_pointer()
                        .child(div().truncate().child(label))
                        .map(|d| Self::file_drop_target(d, Some(path.clone()), t, cx))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            cx.stop_propagation();
                            if last {
                                this.edit_address(&EditAddress, w, cx);
                            } else {
                                this.navigate_to(path.clone(), w, cx);
                            }
                        })),
                );
        }
        div()
            .flex()
            .items_center()
            .gap(px(if window.viewport_size().width < px(560.) {
                3.
            } else {
                10.
            }))
            .h(px(44.))
            .flex_shrink_0()
            .px(px(if window.viewport_size().width < px(560.) {
                4.
            } else {
                10.
            }))
            .bg(t.c("surface.chrome"))
            .border_1()
            .border_color(t.c("border.default"))
            .rounded(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .bg(t.c("surface.field"))
                    .rounded(px(18.))
                    .px(px(3.))
                    .child(
                        icon_button(
                            "back",
                            "icons/arrow-left.svg",
                            self.history.can_go_back(),
                            t,
                        )
                        .size(px(if window.viewport_size().width < px(560.) {
                            24.
                        } else {
                            32.
                        }))
                        .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
                    )
                    .child(
                        icon_button(
                            "forward",
                            "icons/arrow-right.svg",
                            self.history.can_go_forward(),
                            t,
                        )
                        .size(px(if window.viewport_size().width < px(560.) {
                            24.
                        } else {
                            32.
                        }))
                        .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
                    )
                    .child(
                        icon_button("up", "icons/arrow-up.svg", !self.is_home(), t)
                            .size(px(if window.viewport_size().width < px(560.) {
                                24.
                            } else {
                                32.
                            }))
                            .on_click(cx.listener(|this, _, w, cx| this.go_up(&GoUp, w, cx))),
                    )
                    .child(
                        icon_button("refresh", "icons/refresh.svg", true, t)
                            .size(px(if window.viewport_size().width < px(560.) {
                                24.
                            } else {
                                32.
                            }))
                            .on_click(cx.listener(|this, _, w, cx| this.refresh(&Refresh, w, cx))),
                    ),
            )
            .child(address)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(if window.viewport_size().width < px(560.) {
                        3.
                    } else {
                        8.
                    }))
                    .w(px(if window.viewport_size().width < px(560.) {
                        76.
                    } else if window.viewport_size().width < px(1150.) {
                        190.
                    } else {
                        256.
                    }))
                    .flex_shrink_0()
                    .h(px(36.))
                    .px(px(if window.viewport_size().width < px(560.) {
                        4.
                    } else {
                        12.
                    }))
                    .rounded(px(18.))
                    .bg(t.c("surface.field"))
                    .border_1()
                    .border_color(if self.search.focus_handle(cx).is_focused(window) {
                        t.c("focus.ring")
                    } else {
                        t.c("border.subtle")
                    })
                    .child(
                        svg()
                            .path("icons/search.svg")
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(t.c("icon.default")),
                    )
                    .child(self.search.clone())
                    .when(!self.query.is_empty(), |d| {
                        d.child(
                            icon_button("clear-search", "icons/x.svg", true, t)
                                .size(px(20.))
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.search.update(cx, |input, cx| input.reset(w, cx));
                                })),
                        )
                    }),
            )
            .child(self.render_display_trigger(t, cx))
            .child(
                icon_button("settings", "icons/settings.svg", true, t)
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(OpenSettings), cx)),
            )
    }
    fn render_sidebar(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut panel = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .flex_shrink_0()
            .p(px(6.))
            .bg(t.c("surface.chrome"))
            .border_1()
            .border_color(t.c("border.default"))
            .rounded(px(10.))
            .overflow_y_scroll();
        panel = panel.child(
            div()
                .id("home-place")
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(6.))
                .mb(px(12.))
                .rounded(px(8.))
                .bg(t.c(if self.is_home() {
                    "nav.current.bg"
                } else {
                    "surface.chrome"
                }))
                .text_color(t.c(if self.is_home() {
                    "nav.current.fg"
                } else {
                    "text.secondary"
                }))
                .when(!self.is_home(), |d| d.hover(|s| s.bg(t.c("state.hover"))))
                .cursor_pointer()
                .child(svg().path("icons/home.svg").size(px(18.)).text_color(t.c(
                    if self.is_home() {
                        "nav.current.fg"
                    } else {
                        "icon.default"
                    },
                )))
                .child("ホーム")
                .on_click(cx.listener(|this, _, w, cx| {
                    w.focus(&this.focus, cx);
                    this.navigate(Location::Home, w, cx);
                })),
        );
        for (title, places) in [
            ("ピン留め", &self.pinned),
            ("よく使う場所", &self.favorites),
            ("ストレージ", &self.drives),
        ] {
            if places.is_empty() {
                continue;
            }
            panel = panel.child(
                div()
                    .px(px(10.))
                    .pt(px(if title == "ストレージ" { 20. } else { 2. }))
                    .pb(px(8.))
                    .text_size(px(11.))
                    .text_color(t.c("text.muted"))
                    .child(title),
            );
            for (i, place) in places.iter().enumerate() {
                let path = place.path.clone();
                let unpin_path = path.clone();
                let active = self.path() == Some(path.as_path());
                let drive = title == "ストレージ";
                let space = self.drive_space.get(&path);
                let mut label =
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(div().truncate().child(
                            if place.icon == platform::PlaceIcon::Home {
                                "ユーザーフォルダー".to_string()
                            } else {
                                place.label.clone()
                            },
                        ));
                if drive {
                    if let Some(Some(space)) = space {
                        let used = if space.total == 0 {
                            0.
                        } else {
                            1.0 - space.available as f32 / space.total as f32
                        };
                        label = label
                            .child(
                                div()
                                    .h(px(5.))
                                    .w_full()
                                    .rounded(px(3.))
                                    .overflow_hidden()
                                    .bg(t.c(if active {
                                        "nav.current.meter-track"
                                    } else {
                                        "meter.track"
                                    }))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(gpui::relative(used.clamp(0., 1.)))
                                            .rounded(px(3.))
                                            .bg(t.c(if active {
                                                "nav.current.meter-fill"
                                            } else if used > 0.9 {
                                                "meter.warn"
                                            } else {
                                                "meter.fill"
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.c(if active {
                                        "nav.current.caption"
                                    } else {
                                        "text.muted"
                                    }))
                                    .child(format!(
                                        "空き {} / {}",
                                        format::size(space.available),
                                        format::size(space.total)
                                    )),
                            );
                    } else {
                        label = label.child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.c("text.muted"))
                                .child(if space.is_some() {
                                    "容量を取得できません"
                                } else {
                                    "容量を取得中…"
                                }),
                        );
                    }
                }
                panel = panel.child(
                    div()
                        .id((
                            if drive {
                                "drive"
                            } else if title == "ピン留め" {
                                "pin"
                            } else {
                                "fav"
                            },
                            i,
                        ))
                        .group("sidebar-place")
                        .flex()
                        .items_start()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(if drive { 8. } else { 4. }))
                        .mb(px(if drive { 6. } else { 3. }))
                        .rounded(px(10.))
                        .text_color(if active {
                            t.c("nav.current.fg")
                        } else {
                            t.c("text.secondary")
                        })
                        .when(drive && !active, |d| {
                            d.bg(t.c("surface.field"))
                                .border_1()
                                .border_color(t.c("border.subtle"))
                        })
                        .when(active, |d| d.bg(t.c("nav.current.bg")))
                        .when(!active, |d| d.hover(|s| s.bg(t.c("state.hover"))))
                        .cursor_pointer()
                        .child(self.navigation_icon(
                            &path,
                            (title != "ピン留め").then_some(place.icon),
                            18.,
                            t.c(if active {
                                "nav.current.fg"
                            } else {
                                "accent.fg"
                            }),
                            window,
                            cx,
                        ))
                        .child(label)
                        .when(title == "ピン留め", |d| {
                            d.child(
                                icon_button_with_color(
                                    "unpin",
                                    "icons/x.svg",
                                    true,
                                    t,
                                    t.c(if active {
                                        "nav.current.fg"
                                    } else {
                                        "icon.default"
                                    }),
                                )
                                .size(px(24.))
                                .rounded(px(12.))
                                .track_focus(&self.pin_remove_focus[i])
                                .tab_index(0)
                                .debug_selector(move || format!("unpin-{i}"))
                                .border_1()
                                .border_color(t.c("focus.ring").opacity(0.))
                                // Opacity preserves hit testing for touch; tapping the row
                                // focuses this control so the next tap has a visible target.
                                .opacity(if self.pin_remove_focus[i].is_focused(window) {
                                    1.
                                } else {
                                    0.
                                })
                                .group_hover("sidebar-place", |s| s.opacity(1.))
                                .focus(|s| {
                                    s.opacity(1.).border_color(t.c(if active {
                                        "nav.current.fg"
                                    } else {
                                        "focus.ring"
                                    }))
                                })
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    cx.stop_propagation();
                                    w.focus(&this.focus, cx);
                                    this.toggle_pin_path(unpin_path.clone(), cx);
                                }))
                                // These keys have Explorer action bindings. Consume the
                                // actions too, before they reach the file list handlers.
                                .on_action(cx.listener(move |this, _: &OpenSelected, w, cx| {
                                    this.remove_sidebar_pin(i, w, cx);
                                }))
                                .on_action(cx.listener(move |this, _: &QuickPreview, w, cx| {
                                    this.remove_sidebar_pin(i, w, cx);
                                }))
                                .on_key_down(cx.listener(
                                    move |this, event: &gpui::KeyDownEvent, w, cx| {
                                        let modifiers = event.keystroke.modifiers;
                                        if modifiers.control || modifiers.alt || modifiers.platform
                                        {
                                            return;
                                        }
                                        match event.keystroke.key.as_str() {
                                            "enter" | "space" => {
                                                this.remove_sidebar_pin(i, w, cx);
                                            }
                                            "tab" => {
                                                let target = if modifiers.shift && i == 0 {
                                                    &this.display_settings.trigger_focus
                                                } else if modifiers.shift {
                                                    &this.pin_remove_focus[i - 1]
                                                } else if i + 1 < this.pinned.len() {
                                                    &this.pin_remove_focus[i + 1]
                                                } else {
                                                    &this.focus
                                                };
                                                w.focus(target, cx);
                                            }
                                            "escape" => w.focus(&this.focus, cx),
                                            _ => return,
                                        }
                                        cx.stop_propagation();
                                    },
                                )),
                            )
                        })
                        .map(|d| Self::file_drop_target(d, Some(path.clone()), t, cx))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.navigate_to(path.clone(), w, cx);
                            if title == "ピン留め" {
                                w.focus(&this.pin_remove_focus[i], cx);
                            }
                        })),
                );
            }
        }
        panel.child(self.render_wsl(t, cx))
    }

    fn remove_sidebar_pin(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        if let Some(place) = self.pinned.get(index) {
            self.toggle_pin_path(place.path.clone(), cx);
        }
    }
    fn render_header(&self, t: Theme, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let col = |id: &'static str, label: &'static str, key: SortKey, cx: &mut Context<Self>| {
            let active = self.sort.key == key;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(3.))
                .h_full()
                .px(px(8.))
                .cursor_pointer()
                .text_color(if active {
                    t.c("accent.fg")
                } else {
                    t.c("text.muted")
                })
                .child(label)
                .when(active, |d| {
                    d.child(
                        svg()
                            .path(if self.sort.ascending {
                                "icons/chevron-up.svg"
                            } else {
                                "icons/chevron-down.svg"
                            })
                            .size(px(11.)),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };
        div()
            .flex()
            .items_center()
            .h(px(28.))
            .flex_shrink_0()
            .px(px(6.))
            .bg(t.c("surface.chrome"))
            .border_b_1()
            .border_color(t.c("border.subtle"))
            .text_size(px(12.))
            .child(div().flex_1().min_w_0().h_full().pl(px(24.)).child(col(
                "h-name",
                "名前",
                SortKey::Name,
                cx,
            )))
            .child(div().w(COL_MODIFIED).flex_shrink_0().h_full().child(col(
                "h-mod",
                "更新日時",
                SortKey::Modified,
                cx,
            )))
            .when(!narrow, |d| {
                d.child(div().w(COL_KIND).flex_shrink_0().h_full().child(col(
                    "h-kind",
                    "種類",
                    SortKey::Kind,
                    cx,
                )))
            })
            .child(
                div()
                    .w(COL_SIZE)
                    .flex_shrink_0()
                    .h_full()
                    .flex()
                    .justify_end()
                    .child(col("h-size", "サイズ", SortKey::Size, cx)),
            )
            .child(div().w(px(12.)).flex_shrink_0())
    }
    fn narrow(&self, window: &Window) -> bool {
        self.pane && window.viewport_size().width < px(1200.)
    }
    fn is_downloads(&self) -> bool {
        self.path().is_some_and(|path| {
            self.favorites
                .iter()
                .any(|p| p.icon == platform::PlaceIcon::Downloads && p.path == path)
        })
    }
    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = self.theme(window);
        let focused = self.focus.is_focused(window);
        let narrow = self.narrow(window);
        range
            .filter_map(|row_ix| {
                // Keep a reachable blank starting area even in a full folder.
                if row_ix >= self.display_rows.len() {
                    return Some(
                        div()
                            .w_full()
                            .h(px(if self.compact { 24. } else { 28. }))
                            .into_any_element(),
                    );
                }
                let ix = match *self.display_rows.get(row_ix)? {
                    DisplayRow::Entry(ix) => ix,
                    DisplayRow::Header(group, count) => {
                        return Some(
                            div()
                                .id(("date-group", row_ix))
                                .h(px(if self.compact { 24. } else { 28. }))
                                .w_full()
                                .px(px(16.))
                                .flex()
                                .items_center()
                                .border_b_1()
                                .border_color(t.c("border.subtle"))
                                .text_size(px(11.))
                                .text_color(t.c("text.muted"))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(format!("{} · {}", group.label(), count))
                                .into_any_element(),
                        );
                    }
                };
                let &i = self.visible.get(ix)?;
                let e = self.entries[i].clone();
                let dragging = self.is_drag_source(&e.path, cx);
                let selected = self.item_is_selected(ix) && !dragging;
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
                let row = row_surface(t, selected, focused)
                    .when(dragging, |d| d.opacity(0.45))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .flex_1()
                            .min_w_0()
                            .child(self.object_icon(
                                &e.path,
                                e.kind == EntryKind::Dir,
                                18.,
                                window,
                                cx,
                            ))
                            .child(self.render_inline_name(&e.path, t, cx).unwrap_or_else(|| {
                                div()
                                    .truncate()
                                    .text_color(name_color)
                                    .when(selected, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                    .when(e.hidden, |d| d.opacity(0.55))
                                    .child(e.name.clone())
                                    .into_any_element()
                            })),
                    )
                    .child(
                        div()
                            .w(COL_MODIFIED)
                            .flex_shrink_0()
                            .px(px(8.))
                            .text_size(px(12.))
                            .text_color(meta_color)
                            .truncate()
                            .child(e.modified.map(format::modified).unwrap_or_default()),
                    )
                    .when(!narrow, |d| {
                        d.child(
                            div()
                                .w(COL_KIND)
                                .flex_shrink_0()
                                .px(px(8.))
                                .text_size(px(12.))
                                .text_color(meta_color)
                                .truncate()
                                .child(e.kind_label()),
                        )
                    })
                    .child(
                        div()
                            .w(COL_SIZE)
                            .flex_shrink_0()
                            .px(px(8.))
                            .flex()
                            .justify_end()
                            .text_size(px(12.))
                            .text_color(meta_color)
                            .child(if e.kind == EntryKind::Dir && !self.is_archive() {
                                self.size_cell(&e.path, t, cx)
                            } else {
                                div()
                                    .child(e.size.map(format::size).unwrap_or_else(|| "—".into()))
                                    .into_any_element()
                            }),
                    );
                Some(
                    div()
                        .id(ix)
                        .w_full()
                        .h(px(if self.compact { 24. } else { 28. }))
                        .px(px(12.))
                        .child(
                            row.when(!self.is_archive(), |row| {
                                row.on_drag(self.file_drag(ix, cx), |drag, _, _, cx| {
                                    drag.resolve(cx);
                                    cx.new(|_| drag.clone())
                                })
                                .external_drag_payload(
                                    |drag: &FileDrag, _, cx| drag.external_payload(cx),
                                )
                            })
                            .when(e.kind == EntryKind::Dir && !self.is_archive(), |d| {
                                Self::file_drop_target(d, Some(e.path.clone()), t, cx)
                            })
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event, w, cx| {
                                    this.open_context_menu(Some(ix), event, w, cx)
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, w, cx| {
                                    this.pointer_select(ix, event, w, cx)
                                }),
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, w, cx| {
                                    this.finish_pointer_click(ix, e, cx);
                                    if e.click_count() == 2 {
                                        this.open_entry(ix, w, cx)
                                    }
                                },
                            )),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }
    fn render_list(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(err) = &self.error {
            return self
                .selection_surface(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.))
                        .child(empty_state(err.clone(), t.c("danger.fg")))
                        .when(
                            cfg!(target_os = "macos")
                                && self.permission_observation.observation
                                    == crate::macos_permissions::Observation::Denied,
                            |d| {
                                d.child(
                                    div()
                                        .id("permission-recovery")
                                        .cursor_pointer()
                                        .p(px(12.))
                                        .text_color(t.c("accent.fg"))
                                        .child("アクセス権限の設定を開く")
                                        .on_click(cx.listener(|_, _, window, cx| {
                                            window.dispatch_action(
                                                Box::new(crate::OpenPermissions),
                                                cx,
                                            )
                                        })),
                                )
                            },
                        ),
                    t,
                    cx,
                )
                .into_any_element();
        }
        if self.visible.is_empty() {
            return self
                .selection_surface(
                    empty_state(
                        if self.loading {
                            "読み込み中…"
                        } else if !self.query.is_empty() {
                            "一致する項目はありません"
                        } else {
                            "空のフォルダー"
                        }
                        .into(),
                        t.c("text.muted"),
                    ),
                    t,
                    cx,
                )
                .into_any_element();
        }
        let list = uniform_list(
            "entries",
            self.display_rows.len() + 2,
            cx.processor(Self::render_rows),
        )
        .flex_1()
        .min_h_0()
        .py(px(2.))
        .track_scroll(&self.scroll)
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event, w, cx| this.open_context_menu(None, event, w, cx)),
        );
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(self.selection_surface(list, t, cx))
            .child(self.render_scrollbar(t, cx))
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
            .into_any_element()
    }
    fn render_inspector(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut panel = div()
            .id("inspector")
            .w(px(268.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.c("surface.chrome"))
            .border_1()
            .border_color(t.c("border.default"))
            .rounded(px(10.))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(18.))
                    .border_b_1()
                    .border_color(t.c("border.subtle"))
                    .text_size(px(12.))
                    .text_color(t.c("text.muted"))
                    .child("詳細")
                    .child(
                        icon_button("close-pane", "icons/x.svg", true, t)
                            .size(px(22.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.request_pane(false, cx);
                            })),
                    ),
            );
        let mut body = div()
            .id("inspector-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(16.));
        if let Some(e) = self.selected_entry() {
            body = body
                .child(
                    div()
                        .h(px(156.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(t.c("surface.card"))
                        .border_1()
                        .border_color(t.c("border.card"))
                        .rounded(px(12.))
                        .overflow_hidden()
                        .child(self.inspector_picture(e, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_size(px(15.))
                                .child(e.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.c("badge.fg"))
                                .child(e.kind_label()),
                        ),
                );
            let metadata = vec![
                (
                    "サイズ",
                    e.size.map(format::size).unwrap_or_else(|| "—".into()),
                ),
                (
                    "更新日時",
                    e.modified
                        .map(format::modified)
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    if self.is_archive() {
                        "圧縮内"
                    } else {
                        "場所"
                    },
                    e.path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| {
                            if self.is_archive() {
                                "/".into()
                            } else {
                                String::new()
                            }
                        }),
                ),
            ];
            body = body
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .p(px(14.))
                        .bg(t.c("surface.card"))
                        .border_1()
                        .border_color(t.c("border.card"))
                        .rounded(px(12.))
                        .children(metadata.into_iter().map(|(k, v)| {
                            div()
                                .flex()
                                .gap(px(12.))
                                .text_size(px(12.))
                                .child(
                                    div()
                                        .w(px(50.))
                                        .flex_shrink_0()
                                        .text_color(t.c("text.muted"))
                                        .child(k),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(v))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("open-inspector")
                                .h(px(32.))
                                .px(px(16.))
                                .flex()
                                .items_center()
                                .rounded(px(16.))
                                .bg(t.c("button.primary.bg"))
                                .text_color(t.c("text.on-accent"))
                                .cursor_pointer()
                                .child("開く")
                                .on_click(cx.listener(|this, _, w, cx| {
                                    if let Some(ix) = this.selected {
                                        this.open_entry(ix, w, cx)
                                    }
                                })),
                        )
                        .when(!self.is_archive(), |d| {
                            d.child(
                                div()
                                    .id("copy-path")
                                    .h(px(32.))
                                    .px(px(12.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(16.))
                                    .bg(t.c("button.bg"))
                                    .border_1()
                                    .border_color(t.c("button.border"))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .child("パスをコピー")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.is_archive() {
                                            return;
                                        }
                                        if let Some(e) = this.selected_entry() {
                                            this.copy_path_list(&[e.path.clone()], cx);
                                            this.notice = Some("パスをコピーしました".into());
                                            cx.notify()
                                        }
                                    })),
                            )
                        }),
                );
        } else {
            body = body.child(
                div()
                    .mt(px(52.))
                    .text_size(px(12.))
                    .text_color(t.c("text.muted"))
                    .child("項目を選択すると詳細を表示します"),
            );
        }
        use gpui::{Animation, AnimationExt};
        let elapsed = self
            .pane_revealed_at
            .map(|at| cx.background_executor().now().saturating_duration_since(at));
        panel = if let Some(elapsed) = elapsed.filter(|dt| *dt < crate::motion::PANE_REVEAL)
            && !cx.reduce_motion()
        {
            let start = elapsed.as_secs_f32() / crate::motion::PANE_REVEAL.as_secs_f32();
            panel.child(body.with_animation(
                ("pane-reveal", self.pane_generation),
                Animation::new(crate::motion::PANE_REVEAL),
                move |d, p| d.opacity(crate::motion::ease_out_cubic(p.max(start))),
            ))
        } else {
            panel.child(body)
        };
        panel
    }
    fn render_permission_notice(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("folder-permission-notice")
            .debug_selector(|| "folder-permission-notice".into())
            .mx(px(6.))
            .mb(px(6.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(t.c("surface.card"))
            .border_1()
            .border_color(t.c("border.subtle"))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().child("このフォルダーにアクセスできません"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.c("text.secondary"))
                    .child("macOS のアクセス設定、またはフォルダー自体の権限を確認してください。"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .id("folder-permission-settings")
                            .debug_selector(|| "folder-permission-settings".into())
                            .cursor_pointer()
                            .text_color(t.c("accent.fg"))
                            .child("設定の案内")
                            .on_click(cx.listener(|_, _, w, cx| {
                                w.dispatch_action(Box::new(crate::OpenPermissions), cx)
                            })),
                    )
                    .child(
                        div()
                            .id("folder-permission-retry")
                            .debug_selector(|| "folder-permission-retry".into())
                            .cursor_pointer()
                            .text_color(t.c("accent.fg"))
                            .child("再試行")
                            .on_click(cx.listener(|this, _, w, cx| this.refresh(&Refresh, w, cx))),
                    )
                    .child(
                        div()
                            .id("folder-permission-dismiss")
                            .debug_selector(|| "folder-permission-dismiss".into())
                            .cursor_pointer()
                            .child("閉じる")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.permission_observation.dismiss();
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_status(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(28.))
            .flex_shrink_0()
            .px(px(10.))
            .bg(t.c("surface.statusbar"))
            .border_t_1()
            .border_color(t.c("border.statusbar"))
            .text_size(px(11.))
            .text_color(t.c("text.secondary"))
            .child(format!(
                "{} 項目{}",
                format::count(self.item_count()),
                if self.loading {
                    " · 読み込み中…"
                } else {
                    ""
                }
            ))
            .when(self.marquee_active(), |d| d.child("範囲選択中…"))
            .when_some(
                self.selected_entry().filter(|_| !self.marquee_active()),
                |d, e| {
                    d.child(div().text_color(t.c("accent.fg")).child(format!(
                        "{} 項目を選択{}", self.multi_selection.len(),
                        e.size
                            .map(|s| format!(" · {}", format::size(s)))
                            .unwrap_or_default()
                    )))
                },
            )
            .when_some(self.notice.clone(), |d, n| {
                d.child(div().truncate().child(n))
            })
            .when(
                cfg!(target_os = "macos")
                    && self.permission_observation.observation
                        == crate::macos_permissions::Observation::Denied,
                |d| {
                    d.child(
                        div()
                            .id("status-permission-recovery")
                            .cursor_pointer()
                            .text_color(t.c("accent.fg"))
                            .child("アクセス権限の設定")
                            .on_click(cx.listener(|_, _, w, cx| {
                                w.dispatch_action(Box::new(crate::OpenPermissions), cx)
                            })),
                    )
                },
            )
            .child(div().flex_1())
            .when(self.show_hidden, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_color(t.c("accent.fg"))
                        .child(svg().path("icons/eye.svg").size(px(13.)))
                        .child("隠しファイル表示中"),
                )
            })
    }
}
impl Drop for FolderView {
    fn drop(&mut self) {
        self.media_scope.cancel();
        self.preview_scope.cancel();
        self.cancel.cancel();
    }
}
impl Focusable for FolderView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some(dialog) = &self.archive_dialog {
            dialog.input.focus_handle(cx)
        } else if let Some(editor) = &self.name_editor {
            editor.input.focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}
impl Render for FolderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.flush_directory_refresh(cx);
        let t = self.theme(window);
        self.check_motion_layout();
        self.cancel_marquee_if_layout_changed();
        self.search.update(cx, |input, _| {
            input.theme = t;
            input.placeholder = if self.is_home() {
                "ホーム内を検索…"
            } else {
                "このフォルダーを検索…"
            }
            .into();
        });
        self.address.update(cx, |input, _| input.theme = t);
        #[cfg(target_os = "macos")]
        {
            if self.preview_open {
                if let Some(entry) = self.selected_entry() {
                    match crate::media::quicklook::update(&entry.path, window) {
                        Ok(true) => (),
                        Ok(false) => {
                            self.preview_open = false;
                            self.preview_scope.cancel();
                        }
                        Err(error) => {
                            self.preview_open = false;
                            self.preview_scope.cancel();
                            self.notice = Some(error);
                        }
                    }
                }
            } else {
                let _ = crate::media::quicklook::close(window);
            }
        }

        let content_width = f32::from(window.viewport_size().width)
            - f32::from(SIDEBAR_WIDTH)
            - if self.pane { 280. } else { 0. }
            - 40.;
        let columns = (content_width / if self.compact { 112. } else { 128. })
            .floor()
            .max(1.) as usize;
        if columns != self.gallery_columns {
            self.gallery_columns = columns;
            self.rebuild_display_rows();
        }
        div()
            .key_context("Explorer")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                if event.keystroke.key == "tab" && !this.input_focused(w, cx) {
                    if event.keystroke.modifiers.shift && !this.pinned.is_empty() {
                        w.focus(&this.pin_remove_focus[this.pinned.len() - 1], cx);
                    } else {
                        w.focus(&this.display_settings.trigger_focus, cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.drag_scrollbar(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.release_marquee(event.position, cx);
                    this.scrollbar_drag = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.release_marquee(event.position, cx);
                    this.scrollbar_drag = None;
                    cx.notify();
                }),
            )
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy_files))
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::delete_permanently))
            .on_action(cx.listener(Self::show_properties))
            .on_action(cx.listener(|this, _: &NewFolderSelected, w, cx| {
                if !this.input_focused(w, cx) {
                    if let Some(path) = this.path().map(Path::to_path_buf) {
                        this.begin_name_edit(name_operations::NameOperation::Folder(path), w, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &CopySelectedPaths, w, cx| {
                if !this.input_focused(w, cx) && this.path().is_some() {
                    let paths = this.selected_paths();
                    this.copy_path_list(&paths, cx);
                }
            }))
            .on_action(cx.listener(Self::cut_files))
            .on_action(cx.listener(Self::paste_files))
            .on_drag_move(cx.listener(Self::promote_file_drag))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::keyboard_context_menu))
            .on_action(cx.listener(Self::edit_address))
            .on_action(cx.listener(|this, _: &SelectLeft, w, cx| {
                if !this.input_focused(w, cx) && (this.gallery || this.preview_open) {
                    this.move_selection(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectRight, w, cx| {
                if !this.input_focused(w, cx) && (this.gallery || this.preview_open) {
                    this.move_selection(1, cx);
                }
            }))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_up))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::toggle_hidden))
            .on_action(cx.listener(|this, _: &FocusSearch, w, cx| {
                this.close_display_settings(cx);
                w.focus(&this.search.focus_handle(cx), cx);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, w, cx| {
                if this.cancel_marquee() {
                    cx.notify();
                    return;
                }
                if this.preview_open {
                    this.close_preview(cx);
                    return;
                }
                if this.editing_address {
                    this.cancel_address(cx);
                } else {
                    this.search.update(cx, |input, cx| input.reset(w, cx));
                }
                w.focus(&this.focus, cx);
                cx.notify()
            }))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _: &MouseDownEvent, w, cx| this.go_back(&GoBack, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _: &MouseDownEvent, w, cx| this.go_forward(&GoForward, w, cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(t.c("surface.canvas"))
            .text_color(t.c("text.primary"))
            .font_family(if cfg!(target_os = "macos") {
                ".SystemUIFont"
            } else {
                "Segoe UI"
            })
            .text_size(px(13.5))
            .child(
                div()
                    .px(px(6.))
                    .pb(px(6.))
                    .flex_shrink_0()
                    .child(self.render_toolbar(t, window, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(6.))
                    .px(px(6.))
                    .pb(px(6.))
                    .child(self.render_sidebar(t, window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(t.c("surface.content"))
                            .border_1()
                            .border_color(t.c("border.content"))
                            .rounded(px(10.))
                            .overflow_hidden()
                            .map(|d| {
                                Self::file_drop_receiver(d, self.path().map(Path::to_path_buf), cx)
                            })
                            .when(self.is_home(), |d| d.child(self.render_home(t, window, cx)))
                            .when(!self.is_home(), |d| {
                                d.on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(|this, event, w, cx| {
                                        this.open_context_menu(None, event, w, cx)
                                    }),
                                )
                                .when(!self.gallery, |d| {
                                    d.child(self.render_header(t, self.narrow(window), cx))
                                })
                                .child(
                                    if self.gallery && !self.visible.is_empty() {
                                        self.render_gallery(t, cx)
                                    } else {
                                        self.render_list(t, cx)
                                    },
                                )
                            }),
                    )
                    .when(self.pane && !self.is_home(), |d| {
                        d.child(self.render_inspector(t, window, cx))
                    }),
            )
            .when(self.address_pending || self.address_error.is_some(), |d| {
                d.child(
                    div()
                        .px(px(16.))
                        .py(px(5.))
                        .text_size(px(12.))
                        .text_color(t.c("danger.fg"))
                        .child(
                            self.address_error
                                .clone()
                                .unwrap_or_else(|| "パスを確認中…".into()),
                        ),
                )
            })
            .when(
                cfg!(target_os = "macos") && self.permission_observation.visible(),
                |d| d.child(self.render_permission_notice(t, cx)),
            )
            .child(self.render_status(t, cx))
            .when_some(
                self.transfer_progress.read(cx).latest.clone(),
                |d, progress| {
                    let percent = if progress.work_total == 0 {
                        String::new()
                    } else {
                        format!(
                            " · {}%",
                            (u64::from(progress.work_done) * 100 / u64::from(progress.work_total))
                                .min(100)
                        )
                    };
                    d.child(
                        div()
                            .px(px(14.))
                            .py(px(8.))
                            .text_size(px(12.))
                            .bg(t.c("surface.chrome"))
                            .child(format!(
                                "{}{percent} · {} · 中止はWindowsの進捗ウィンドウから",
                                progress.label,
                                progress.current_item.unwrap_or_else(|| "準備中…".into())
                            )),
                    )
                },
            )
            .child(self.render_archive_jobs(t, cx))
            .when(self.is_archive(), |d| {
                d.child(self.render_archive_bar(t, cx))
            })
            .when(self.inline_name_target().is_some(), |d| {
                d.when_some(
                    self.name_editor.as_ref().and_then(|e| e.error.clone()),
                    |d, error| {
                        d.child(
                            div()
                                .px(px(16.))
                                .py(px(4.))
                                .text_size(px(12.))
                                .text_color(t.c("danger.fg"))
                                .child(error),
                        )
                    },
                )
            })
            .when(self.context_menu.is_some(), |d| {
                d.child(self.render_context_menu(t, cx))
            })
            .when(self.display_settings.open, |d| {
                d.child(self.render_display_settings(t, window, cx))
            })
            .when(
                self.name_editor.is_some() && self.inline_name_target().is_none(),
                |d| d.child(self.render_name_editor(t, cx)),
            )
            .when(self.archive_dialog.is_some(), |d| {
                d.child(self.render_archive_dialog(t, cx))
            })
    }
}
// Shared by real list rows and the hover regression probe.
fn row_surface(t: Theme, selected: bool, focused: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id("row-surface")
        .flex()
        .items_center()
        .w_full()
        .h_full()
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(if selected {
            if focused {
                t.c("selection.border")
            } else {
                t.c("selection.inactive.border")
            }
        } else {
            gpui::transparent_black().into()
        })
        .when(selected, |d| {
            d.bg(t.c(if focused {
                "selection.bg"
            } else {
                "selection.inactive.bg"
            }))
        })
        .when(!selected, |d| d.hover(|s| s.bg(t.c("state.hover"))))
}

pub fn icon_button(
    id: &'static str,
    icon: &'static str,
    enabled: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    icon_button_with_color(id, icon, enabled, t, t.c("icon.default"))
}

fn icon_button_with_color(
    id: &'static str,
    icon: &'static str,
    enabled: bool,
    t: Theme,
    color: gpui::Rgba,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(32.))
        .flex_shrink_0()
        .rounded(px(16.))
        .child(svg().path(icon).size(px(16.)).text_color(color))
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(t.c("state.hover")))
        })
        .when(!enabled, |d| d.opacity(0.35))
        .when(id != "display-settings-trigger", |d| {
            d.tooltip(move |_, cx| {
                cx.new(|_| Tip {
                    text: match id {
                        "back" => "戻る（Alt+←）",
                        "forward" => "進む（Alt+→）",
                        "up" => "上へ（Alt+↑）",
                        "refresh" => "更新（F5）",
                        "pane" => "詳細ペイン",
                        "close-pane" => "詳細ペインを閉じる",
                        "unpin" => "ピン留めを外す",
                        "clear-search" => "検索をクリア（Esc）",
                        "new-tab" => "新しいタブ（Ctrl+T）",
                        "display-settings-trigger" => "表示設定",
                        "settings" => "設定（Ctrl+,）",
                        "close-settings" => "設定を閉じる（Esc）",
                        _ => "テーマ切替",
                    }
                    .into(),
                    theme: t,
                })
                .into()
            })
        })
}
fn empty_state(msg: String, color: gpui::Rgba) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(color)
        .child(msg)
}
fn merge_sorted(a: Vec<Entry>, b: Vec<Entry>, sort: SortOrder) -> Vec<Entry> {
    if a.is_empty() {
        return b;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut a = a.into_iter().peekable();
    let mut b = b.into_iter().peekable();
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        if sort.compare(x, y) != std::cmp::Ordering::Greater {
            out.push(a.next().unwrap())
        } else {
            out.push(b.next().unwrap())
        }
    }
    out.extend(a);
    out.extend(b);
    out
}

struct Tip {
    text: String,
    theme: Theme,
}
impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(self.theme.c("surface.raised"))
            .border_1()
            .border_color(self.theme.c("border.default"))
            .text_color(self.theme.c("text.primary"))
            .text_size(px(12.))
            .child(self.text.clone())
    }
}
