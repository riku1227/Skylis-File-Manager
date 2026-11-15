use std::ops::Range;
#[cfg(test)]
mod tests;
use std::path::{Path, PathBuf};

mod home;
mod transfers;
use transfers::FileDrag;
mod sizes;
use sizes::SizeState;
mod grouping;
mod media_view;
mod wsl;
use crate::media::{MediaCache, MediaScope};
mod context_menu;
mod scrollbar;
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
}
impl gpui::EventEmitter<FolderEvent> for FolderView {}

pub struct FolderView {
    multi_selection: std::collections::BTreeSet<PathBuf>,
    selection_anchor: Option<PathBuf>,
    sizes: SizeState,
    media: Entity<MediaCache>,
    media_scope: MediaScope,
    preview_scope: MediaScope,
    preview_open: bool,
    gallery: bool,
    gallery_override: Option<bool>,
    gallery_columns: usize,
    gallery_rows: Vec<(Option<explorer_core::date_group::DateGroup>, Vec<usize>)>,
    context_menu: Option<ContextMenu>,
    menu_focus: FocusHandle,
    _shell_menu: Option<Task<()>>,
    pub mode: ThemeMode,
    pub pane: bool,
    pub compact: bool,
    search: Entity<TextInput>,
    query: String,
    icons: Entity<ShellIcons>,
    focus: FocusHandle,
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
    sort: SortOrder,
    show_hidden: bool,
    scroll: UniformListScrollHandle,
    scrollbar_drag: Option<f32>,
    loading: bool,
    stats: Option<ListingStats>,
    error: Option<String>,
    notice: Option<String>,
    cancel: CancelToken,
    _listing: Option<Task<()>>,
    favorites: Vec<Place>,
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
        if let Some(preferences) = preferences {
            cx.observe(&preferences, |this, preferences, cx| {
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
        let mut this = Self {
            multi_selection: Default::default(),
            selection_anchor: None,
            sizes: SizeState::default(),
            media,
            media_scope: MediaScope::new(),
            preview_scope: MediaScope::new(),
            preview_open: false,
            gallery: false,
            gallery_override: None,
            gallery_columns: 1,
            gallery_rows: Vec::new(),
            context_menu: None,
            menu_focus: cx.focus_handle(),
            _shell_menu: None,
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
            sort: SortOrder::default(),
            show_hidden: false,
            scroll: UniformListScrollHandle::new(),
            scrollbar_drag: None,
            loading: false,
            stats: None,
            error: None,
            notice: None,
            cancel: CancelToken::new(),
            _listing: None,
            favorites: platform::favorite_places(),
            drives: platform::drives(),
            drive_space: HashMap::new(),
            title_dirty: true,
        };
        this.attach_window(window, cx);
        this.observe_file_transfers(cx);
        this.refresh_drives(cx);
        this.discover_wsl(cx);
        this.load(start, None, cx);
        this
    }

    pub fn attach_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._window_subscriptions.clear();
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
            .push(cx.on_blur(&self.focus, window, |_, _, cx| cx.notify()));
        self._window_subscriptions
            .push(cx.observe_window_appearance(window, |_, _, cx| cx.notify()));
        self._window_subscriptions.push(cx.on_blur(
            &self.address.focus_handle(cx),
            window,
            |this, _, cx| this.cancel_address(cx),
        ));
    }

    fn refresh_drives(&mut self, cx: &mut Context<Self>) {
        self.drives = platform::drives();
        self.drive_space.clear();
        let paths: Vec<_> = self.drives.iter().map(|p| p.path.clone()).collect();
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

    fn load(&mut self, location: Location, select: Option<String>, cx: &mut Context<Self>) {
        self.cancel_address(cx);
        self.cancel_sizes(cx);
        self.media_scope.cancel();
        self.preview_scope.cancel();
        self.preview_open = false;
        self.gallery = self.gallery_override.unwrap_or(false);
        self.gallery_rows.clear();
        self.context_menu = None;
        self._shell_menu = None;
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
        self.notice = None;
        self.title_dirty = true;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);

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

        self._listing = Some(cx.spawn(async move |this, cx| {
            while let Ok(ev) = rx.recv().await {
                if this
                    .update(cx, |this, cx| this.on_listing_event(ev, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
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
                self.loading = false;
                self.stats = Some(stats);
                self.choose_auto_gallery(cx);
                self.pending_select = None;
            }
            ListingEvent::Failed(kind, msg) => {
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
        let query = self.query.to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| (show_hidden || !e.hidden) && e.name.to_lowercase().contains(&query))
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
        // リンクは開く瞬間だけ先を確認（一覧作成時には追わない）
        let is_dir = match entry.kind {
            EntryKind::Dir => true,
            EntryKind::Symlink => path.is_dir(),
            EntryKind::File => false,
        };
        if is_dir {
            self.navigate_to(path, window, cx);
        } else if let Err(e) = platform::open_with_default_app(&path) {
            self.notice = Some(format!("開けませんでした: {e}"));
            cx.notify();
        }
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

    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        let location = self.history.current().clone();
        let sel = self.selected_name();
        self.refresh_drives(cx);
        self.load(location, sel, cx);
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        let keep = self.selected_entry().map(|e| e.path.clone());
        self.rebuild_visible(keep.as_deref());
        cx.notify();
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.grouped = false;
        self.sort = self.sort.toggled(key);
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
    pub fn label(&self) -> String {
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
        self.search.focus_handle(cx).is_focused(window)
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
        let image = self
            .icons
            .update(cx, |icons, cx| icons.get(path, dir, physical, cx));
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
            .flex()
            .items_center()
            .h(px(36.))
            .flex_1()
            .min_w_0()
            .px(px(10.))
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
        } else if let Some(path) = self.path() {
            address = address.child(self.object_icon(path, true, 18., window, cx));
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
                        .px(px(10.))
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
            .gap(px(10.))
            .h(px(44.))
            .flex_shrink_0()
            .px(px(10.))
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
                        .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
                    )
                    .child(
                        icon_button(
                            "forward",
                            "icons/arrow-right.svg",
                            self.history.can_go_forward(),
                            t,
                        )
                        .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
                    )
                    .child(
                        icon_button("up", "icons/arrow-up.svg", !self.is_home(), t)
                            .on_click(cx.listener(|this, _, w, cx| this.go_up(&GoUp, w, cx))),
                    )
                    .child(
                        icon_button("refresh", "icons/refresh.svg", true, t)
                            .on_click(cx.listener(|this, _, w, cx| this.refresh(&Refresh, w, cx))),
                    ),
            )
            .child(address)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .w(px(if window.viewport_size().width < px(1150.) {
                        190.
                    } else {
                        256.
                    }))
                    .flex_shrink_0()
                    .h(px(36.))
                    .px(px(12.))
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
            .child(
                div()
                    .id("view-mode")
                    .px(px(8.))
                    .cursor_pointer()
                    .text_color(t.c("text.secondary"))
                    .child(if self.gallery {
                        "詳細表示"
                    } else {
                        "ギャラリー"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.set_gallery(!this.gallery, cx))),
            )
            .child(
                icon_button("settings", "icons/settings.svg", true, t)
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(OpenSettings), cx)),
            )
            .child(
                icon_button("theme", "icons/sun.svg", true, t)
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(CycleTheme), cx)),
            )
            .child(
                icon_button("pane", "icons/panel.svg", true, t)
                    .when(self.pane, |d| d.bg(t.c("accent.soft")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pane = !this.pane;
                        cx.notify()
                    })),
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
            ("よく使う場所", &self.favorites),
            ("ストレージ", &self.drives),
        ] {
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
                panel =
                    panel.child(
                        div()
                            .id((if drive { "drive" } else { "fav" }, i))
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
                            .child(self.object_icon(&path, true, 18., window, cx))
                            .child(label)
                            .map(|d| Self::file_drop_target(d, Some(path.clone()), t, cx))
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.navigate_to(path.clone(), w, cx)
                            })),
                    );
            }
        }
        panel.child(self.render_wsl(t, cx))
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
                let selected = self.multi_selection.contains(&e.path);
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
                            .child(
                                div()
                                    .truncate()
                                    .text_color(name_color)
                                    .when(selected, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                    .when(e.hidden, |d| d.opacity(0.55))
                                    .child(e.name.clone()),
                            ),
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
                            .child(if e.kind == EntryKind::Dir {
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
                        .px(px(6.))
                        .child(row)
                        .on_drag(self.file_drag(ix, cx), |drag, _, _, cx| {
                            drag.resolve(cx);
                            cx.new(|_| drag.clone())
                        })
                        .external_drag_payload(|drag: &FileDrag, _, cx| drag.external_payload(cx))
                        .when(e.kind == EntryKind::Dir, |d| {
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                            this.finish_pointer_click(ix, e, cx);
                            if e.click_count() == 2 {
                                this.open_entry(ix, w, cx)
                            }
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }
    fn render_list(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(err) = &self.error {
            return empty_state(err.clone(), t.c("danger.fg")).into_any_element();
        }
        if self.visible.is_empty() {
            return empty_state(
                if self.loading {
                    "読み込み中…"
                } else if !self.query.is_empty() {
                    "一致する項目はありません"
                } else {
                    "空のフォルダー"
                }
                .into(),
                t.c("text.muted"),
            )
            .into_any_element();
        }
        let list = uniform_list(
            "entries",
            self.display_rows.len(),
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
            .child(list)
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
                    .child("インスペクタ")
                    .child(
                        icon_button("close-pane", "icons/x.svg", true, t)
                            .size(px(22.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pane = false;
                                cx.notify()
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
                        .child(self.object_icon(
                            &e.path,
                            e.kind == EntryKind::Dir,
                            68.,
                            window,
                            cx,
                        )),
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
                    "場所",
                    e.path
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
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
                        .child(
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
                                    if let Some(e) = this.selected_entry() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            e.path.display().to_string(),
                                        ));
                                        this.notice = Some("パスをコピーしました".into());
                                        cx.notify()
                                    }
                                })),
                        ),
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
        panel = panel.child(body);
        panel
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
            .when_some(self.selected_entry(), |d, e| {
                d.child(div().text_color(t.c("accent.fg")).child(format!(
                        "{} 項目を選択{}", self.multi_selection.len(),
                        e.size
                            .map(|s| format!(" · {}", format::size(s)))
                            .unwrap_or_default()
                    )))
            })
            .when_some(self.notice.clone(), |d, n| {
                d.child(div().truncate().child(n))
            })
            .child(div().flex_1())
            .child(
                div()
                    .id("density")
                    .cursor_pointer()
                    .child(if self.compact {
                        "コンパクト"
                    } else {
                        "標準"
                    })
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(ToggleDensity), cx)),
            )
            .child(
                div()
                    .id("toggle-hidden")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(if self.show_hidden {
                        t.c("accent.fg")
                    } else {
                        t.c("text.secondary")
                    })
                    .child(svg().path("icons/eye.svg").size(px(13.)))
                    .child("隠しファイル")
                    .on_click(
                        cx.listener(|this, _, w, cx| this.toggle_hidden(&ToggleHidden, w, cx)),
                    ),
            )
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
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for FolderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme(window);
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
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.drag_scrollbar(event.position, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.scrollbar_drag = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.scrollbar_drag = None;
                    cx.notify();
                }),
            )
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy_files))
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
                w.focus(&this.search.focus_handle(cx), cx);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, w, cx| {
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
            .font_family("Segoe UI")
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
                                Self::file_drop_target(d, self.path().map(Path::to_path_buf), t, cx)
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
            .child(self.render_status(t, cx))
            .when(self.preview_open && !cfg!(target_os = "macos"), |d| {
                d.child(self.render_preview(t, cx))
            })
            .when(self.context_menu.is_some(), |d| {
                d.child(self.render_context_menu(t, cx))
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
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(32.))
        .flex_shrink_0()
        .rounded(px(16.))
        .child(
            svg()
                .path(icon)
                .size(px(16.))
                .text_color(t.c("icon.default")),
        )
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(t.c("state.hover")))
        })
        .when(!enabled, |d| d.opacity(0.35))
        .tooltip(move |_, cx| {
            cx.new(|_| Tip {
                text: match id {
                    "back" => "戻る（Alt+←）",
                    "forward" => "進む（Alt+→）",
                    "up" => "上へ（Alt+↑）",
                    "refresh" => "更新（F5）",
                    "pane" => "詳細ペイン",
                    "close-pane" => "詳細ペインを閉じる",
                    "clear-search" => "検索をクリア（Esc）",
                    "new-tab" => "新しいタブ（Ctrl+T）",
                    "settings" => "設定（Ctrl+,）",
                    "close-settings" => "設定を閉じる（Esc）",
                    _ => "テーマ切替",
                }
                .into(),
                theme: t,
            })
            .into()
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
