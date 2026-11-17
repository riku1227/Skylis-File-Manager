use super::name_operations::NameOperation;
use super::*;
use gpui::{KeyDownEvent, Point, anchored, deferred};

#[derive(Clone)]
pub(super) struct ContextMenu {
    position: Point<Pixels>,
    item: Option<usize>,
    target: Option<PathBuf>,
    active: usize,
}
#[derive(Clone, Copy)]
pub(super) enum Command {
    Open,
    NewTab,
    Preview,
    Copy,
    Cut,
    Paste,
    Rename,
    Delete,
    Extract,
    ExtractAll,
    CompressZip,
    Compress7z,
    OpenExternal,
    Properties,
    NewFolder,
    NewText,
    Pin,
    CopyPath,
    Refresh,
    Shell,
    Reveal,
    Share,
    Services,
}

impl FolderView {
    pub(super) fn show_properties(
        &mut self,
        _: &ShowProperties,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        if self.path().is_none() || self.input_focused(w, cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        match crate::native_shell::properties(paths, w) {
            Ok(receiver) => {
                cx.spawn(async move |view, cx| {
                    if let Ok(Err(error)) = receiver.recv().await {
                        let _ = view.update(cx, |view, cx| {
                            view.notice = Some(error);
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => self.notice = Some(error),
        }
        cx.notify();
    }
    pub(super) fn copy_path_list(&self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if !paths.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                paths
                    .iter()
                    .map(|p| format!("\"{}\"", p.display()))
                    .collect::<Vec<_>>()
                    .join("\r\n"),
            ));
        }
    }
    pub(super) fn open_context_menu(
        &mut self,
        item: Option<usize>,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if item.is_none() && self.descendant_area(event.position) {
            return;
        }
        self.descendants.selected = None;
        self.close_display_settings(cx);
        self.cancel_scroll_motion();
        window.focus(&self.menu_focus, cx);
        if let Some(ix) = item {
            let path = self.entries[self.visible[ix]].path.clone();
            if !self.multi_selection.contains(&path) {
                self.multi_selection.clear();
                self.multi_selection.insert(path);
            }
        } else {
            self.multi_selection.clear();
        }
        self.selected = item;
        self.context_menu = Some(ContextMenu {
            position: event.position,
            target: item.map(|ix| self.entries[self.visible[ix]].path.clone()),
            item,
            active: 0,
        });
        if event.modifiers.shift && cfg!(windows) && self.path().is_some() {
            self.execute_menu(Command::Shell, window, cx);
        }
        cx.notify();
    }
    pub(super) fn keyboard_context_menu(
        &mut self,
        _: &ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        self.close_display_settings(cx);
        if self.input_focused(window, cx) || self.is_home() {
            return;
        }
        self.cancel_scroll_motion();
        self.context_menu = Some(ContextMenu {
            position: self.menu_anchor(),
            target: self.selected_entry().map(|e| e.path.clone()),
            item: self.selected,
            active: 0,
        });
        window.focus(&self.menu_focus, cx);
        cx.notify();
    }
    fn menu_anchor(&self) -> Point<Pixels> {
        let handle = self.scroll.0.borrow();
        let bounds = handle.base_handle.bounds();
        let height = if self.gallery {
            if self.compact { 156. } else { 172. }
        } else if self.compact {
            24.
        } else {
            28.
        };
        let row = self
            .selected
            .and_then(|i| self.entry_rows.get(i).copied())
            .unwrap_or(0);
        let y = (px((row + 1) as f32 * height) + handle.base_handle.offset().y)
            .clamp(px(8.), bounds.size.height.max(px(8.)));
        bounds.origin + gpui::point(px(24.), y)
    }
    pub(super) fn menu_commands(&self) -> Vec<(&'static str, Command)> {
        let mut commands = Vec::new();
        let item = self.context_menu.as_ref().is_some_and(|m| m.item.is_some());
        let single = item && self.multi_selection.len() <= 1;
        let directory = single && self.selected_entry().is_some_and(Entry::is_dir_like);
        if self.is_archive() {
            if directory {
                commands.push(("開く", Command::Open));
            }
            if item {
                commands.push(("選択した項目を解凍…", Command::Extract));
            }
            commands.push(("すべて解凍…", Command::ExtractAll));
            commands.push(("圧縮ファイルを既定のアプリで開く", Command::OpenExternal));
            commands.push(("更新", Command::Refresh));
            return commands;
        }
        if single {
            commands.push(("開く", Command::Open));
            if directory {
                commands.push(("新しいタブで開く", Command::NewTab));
            } else {
                commands.push(("クイックプレビュー", Command::Preview));
            }
        }
        if !item && self.path().is_some() {
            commands.push(("新しいフォルダー", Command::NewFolder));
            commands.push(("新しいテキスト ドキュメント", Command::NewText));
        }
        if item {
            if cfg!(windows) {
                commands.push(("切り取り", Command::Cut));
            }
            commands.push(("コピー", Command::Copy));
        }
        if cfg!(any(windows, target_os = "macos")) && (!item || directory) && self.path().is_some()
        {
            commands.push(("貼り付け", Command::Paste));
        }
        if single {
            commands.push(("名前の変更", Command::Rename));
        }
        if item
            && self.path().is_some()
            && crate::file_operations::supports_delete(crate::file_operations::DeleteKind::Recycle)
        {
            commands.push(("削除", Command::Delete));
        }
        if single
            && self
                .selected_entry()
                .is_some_and(|e| explorer_core::archive::detect_format(&e.path).is_some())
        {
            commands.push(("解凍…", Command::ExtractAll));
            commands.push(("既定のアプリで開く", Command::OpenExternal));
        }
        if item && self.path().is_some() {
            commands.push(("ZIPに圧縮…", Command::CompressZip));
            commands.push(("7zに圧縮…", Command::Compress7z));
        }
        if directory || !item {
            let path = self
                .context_menu
                .as_ref()
                .and_then(|m| m.target.as_deref())
                .or(self.path());
            let pinned = path.is_some_and(|p| self.is_pinned(p));
            commands.push((
                if pinned {
                    "ピン留めを外す"
                } else {
                    "よく使う場所にピン留め"
                },
                Command::Pin,
            ));
        }
        commands.push(("パスをコピー", Command::CopyPath));
        if !item {
            commands.push(("更新", Command::Refresh));
        }
        if cfg!(target_os = "macos") && item {
            if single {
                commands.push(("Finderで表示", Command::Reveal));
            }
            commands.push(("共有…", Command::Share));
            commands.push(("サービス…", Command::Services));
        }
        if cfg!(windows) {
            if item {
                commands.push(("プロパティ", Command::Properties));
            }
            commands.push(("その他のオプションを表示…", Command::Shell));
        }
        commands
    }
    fn execute_menu(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if self.descendants.selected.is_some() {
            return;
        }
        // A stale or directly dispatched command cannot treat virtual members as disk paths.
        if self.is_archive()
            && !matches!(
                command,
                Command::Open
                    | Command::Extract
                    | Command::ExtractAll
                    | Command::OpenExternal
                    | Command::Refresh
            )
        {
            return;
        }
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        window.focus(&self.focus, cx);
        let path = menu
            .target
            .clone()
            .or_else(|| self.path().map(Path::to_path_buf));
        match command {
            Command::Open => {
                if let Some(ix) = menu.target.as_ref().and_then(|path| {
                    self.visible
                        .iter()
                        .position(|&i| &self.entries[i].path == path)
                }) {
                    self.open_entry(ix, window, cx);
                }
            }
            Command::NewTab => {
                if let Some(path) = path {
                    cx.emit(FolderEvent::OpenInTab(path));
                }
            }
            Command::Preview => self.toggle_preview(&QuickPreview, window, cx),
            Command::Copy => self.copy_files(&CopyFiles, window, cx),
            Command::Cut => self.cut_files(&CutFiles, window, cx),
            Command::Paste => {
                if let Some(path) = path {
                    self.paste_into(path, window, cx);
                }
            }
            Command::Rename => {
                if let Some(path) = path {
                    self.begin_name_edit(NameOperation::Rename(path), window, cx);
                }
            }
            Command::Delete => self.delete_selected(&DeleteFiles, window, cx),
            Command::Extract => self.begin_extract(true, window, cx),
            Command::ExtractAll => self.begin_extract(false, window, cx),
            Command::CompressZip => {
                self.begin_compress(explorer_core::archive::ArchiveFormat::Zip, window, cx)
            }
            Command::Compress7z => {
                self.begin_compress(explorer_core::archive::ArchiveFormat::SevenZip, window, cx)
            }
            Command::OpenExternal => self.open_archive_external(cx),
            Command::Properties => self.show_properties(&ShowProperties, window, cx),
            Command::NewFolder | Command::NewText => {
                if let Some(path) = self.path().map(Path::to_path_buf) {
                    let operation = if matches!(command, Command::NewFolder) {
                        NameOperation::Folder(path)
                    } else {
                        NameOperation::Text(path)
                    };
                    self.begin_name_edit(operation, window, cx);
                }
            }
            Command::Pin => {
                if let Some(path) = path {
                    self.toggle_pin_path(path, cx);
                }
            }
            Command::Share | Command::Services => {
                #[cfg(target_os = "macos")]
                {
                    let paths = self.selected_paths();
                    let result = if matches!(command, Command::Share) {
                        crate::media::macos_actions::show_share(&paths, window, menu.position)
                    } else {
                        crate::media::macos_actions::show_services(&paths, window, menu.position)
                    };
                    if let Err(error) = result {
                        self.notice = Some(error);
                    }
                }
            }
            Command::Reveal => {
                if let Some(path) = path {
                    std::thread::spawn(move || {
                        let _ = std::process::Command::new("/usr/bin/open")
                            .arg("-R")
                            .arg(path)
                            .status();
                    });
                }
            }
            Command::CopyPath => {
                let paths = if menu.item.is_some() {
                    self.selected_paths()
                } else {
                    path.into_iter().collect()
                };
                self.copy_path_list(&paths, cx);
            }
            Command::Refresh => self.refresh(&Refresh, window, cx),
            Command::Shell => {
                let Some(path) = path else {
                    return;
                };
                let request = if menu.item.is_some() {
                    crate::native_shell::show_context_menu(
                        self.selected_paths(),
                        window,
                        menu.position,
                    )
                } else {
                    crate::native_shell::show_background_menu(path, window, menu.position)
                };
                match request {
                    Err(error) => self.notice = Some(error.to_string()),
                    Ok(receiver) => {
                        self._shell_menu = Some(cx.spawn(async move |view, cx| {
                            if let Ok(result) = receiver.recv().await {
                                let _ = view.update(cx, |view, cx| {
                                    match result {
                                        Ok(crate::native_shell::MenuOutcome::Invoked) => view.load(
                                            view.history.current().clone(),
                                            view.selected_name(),
                                            cx,
                                        ),
                                        Err(error) => view.notice = Some(error),
                                        _ => (),
                                    }
                                    cx.notify();
                                });
                            }
                        }));
                    }
                }
            }
        }
        cx.notify();
    }
    pub(super) fn render_context_menu(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        // The ghost draws the commands it had when it closed: the selection they
        // were derived from may already be gone.
        let Some(shown) = self.menu_presence.shown() else {
            return div().into_any_element();
        };
        let (progress, live) = (shown.progress, shown.live);
        let (menu, commands) = shown.content.clone();
        deferred(
            anchored().position(menu.position).snap_to_window().child(
                div()
                    .id("context-menu")
                    .debug_selector(|| "context-menu".into())
                    .occlude()
                    .when(live, |d| d.track_focus(&self.menu_focus))
                    .w(px(300.))
                    .text_size(px(13.))
                    .p(px(4.))
                    .rounded(px(8.))
                    .bg(t.c("surface.raised"))
                    .border_1()
                    .border_color(t.c("border.subtle"))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, w, cx| {
                        // A ghost is already closed and must not move focus again.
                        if this.context_menu.take().is_some() {
                            w.focus(&this.focus, cx);
                            cx.notify();
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                        let commands = this.menu_commands();
                        if let Some(menu) = &mut this.context_menu {
                            match event.keystroke.key.as_str() {
                                "escape" | "tab" => {
                                    this.context_menu = None;
                                    w.focus(&this.focus, cx);
                                }
                                "home" => menu.active = 0,
                                "end" => menu.active = commands.len().saturating_sub(1),
                                "down" => menu.active = (menu.active + 1) % commands.len().max(1),
                                "up" => {
                                    menu.active = (menu.active + commands.len().saturating_sub(1))
                                        % commands.len().max(1)
                                }
                                "enter" | "space" => {
                                    if let Some((_, command)) = commands.get(menu.active) {
                                        this.execute_menu(*command, w, cx);
                                    }
                                }
                                _ => (),
                            }
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .children(
                        commands
                            .iter()
                            .copied()
                            .enumerate()
                            .map(|(ix, (label, command))| {
                                let separator =
                                    ix > 0 && commands[ix - 1].1.group() != command.group();
                                div()
                                    .when(separator, |d| {
                                        d.child(
                                            div()
                                                .h(px(1.))
                                                .mx(px(8.))
                                                .my(px(4.))
                                                .bg(t.c("border.subtle")),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id(("menu-command", ix))
                                            .h(px(if self.compact { 28. } else { 30. }))
                                            .px(px(10.))
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .rounded(px(4.))
                                            .text_color(if ix == menu.active {
                                                t.c("menu.highlight.fg")
                                            } else {
                                                t.c("text.primary")
                                            })
                                            .when(ix == menu.active, |d| {
                                                d.bg(t.c("menu.highlight.bg"))
                                            })
                                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                                if let Some(menu) = &mut this.context_menu {
                                                    if menu.active != ix {
                                                        menu.active = ix;
                                                        cx.notify();
                                                    }
                                                }
                                            }))
                                            .cursor_pointer()
                                            .child(div().flex_1().child(label))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(if ix == menu.active {
                                                        t.c("menu.highlight.kbd")
                                                    } else {
                                                        t.c("text.muted")
                                                    })
                                                    .child(command.shortcut()),
                                            )
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                this.execute_menu(command, w, cx)
                                            })),
                                    )
                            }),
                    )
                    .when(!live, |d| d.child(crate::motion::ghost_shield()))
                    .opacity(progress),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

impl Command {
    fn group(self) -> u8 {
        match self {
            Self::Open | Self::NewTab | Self::Preview | Self::NewFolder | Self::NewText => 0,
            Self::Copy | Self::Cut | Self::Paste | Self::Rename | Self::Delete => 1,
            Self::Pin | Self::CopyPath | Self::Refresh => 2,
            _ => 3,
        }
    }
    fn shortcut(self) -> &'static str {
        match self {
            Self::Open => "Enter",
            Self::Preview => "Space",
            Self::Rename => "F2",
            Self::Delete => "Delete",
            Self::Properties => "Alt+Enter",
            Self::CopyPath => "Ctrl+Shift+C",
            Self::NewFolder => {
                if cfg!(target_os = "macos") {
                    "⌘⇧N"
                } else {
                    "Ctrl+Shift+N"
                }
            }
            Self::Refresh => "F5",
            Self::Copy => {
                if cfg!(target_os = "macos") {
                    "⌘C"
                } else {
                    "Ctrl+C"
                }
            }
            Self::Cut => "Ctrl+X",
            Self::Paste => {
                if cfg!(target_os = "macos") {
                    "⌘V"
                } else {
                    "Ctrl+V"
                }
            }
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn archive_menu_and_shortcuts_cannot_mutate_members(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        let window = cx.add_window(|w, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, w, cx)
        });
        window
            .update(cx, |view, w, cx| {
                view.history.push(
                    Location::Archive {
                        archive: PathBuf::from("C:/sample.zip"),
                        prefix: String::new(),
                    },
                    None,
                );
                view.entries = vec![Entry::virtual_entry(
                    "member.txt".into(),
                    PathBuf::from("member.txt"),
                    EntryKind::File,
                    Some(1),
                    None,
                )];
                view.rebuild_visible(None);
                view.select(0, cx);
                view.keyboard_context_menu(&ShowContextMenu, w, cx);
                let labels: Vec<_> = view
                    .menu_commands()
                    .iter()
                    .map(|(label, _)| *label)
                    .collect();
                assert_eq!(
                    labels,
                    [
                        "選択した項目を解凍…",
                        "すべて解凍…",
                        "圧縮ファイルを既定のアプリで開く",
                        "更新"
                    ]
                );
                for command in [
                    Command::Rename,
                    Command::Delete,
                    Command::Paste,
                    Command::Shell,
                    Command::NewFolder,
                    Command::NewText,
                    Command::Cut,
                    Command::Copy,
                ] {
                    view.execute_menu(command, w, cx);
                    assert!(
                        view.context_menu.is_some(),
                        "disallowed action must not dispatch"
                    );
                }
                view.context_menu = None;
                view.rename_selected(&RenameSelected, w, cx);
                view.delete_selected(&DeleteFiles, w, cx);
                view.delete_permanently(&DeletePermanently, w, cx);
                view.copy_files(&CopyFiles, w, cx);
                view.paste_into(PathBuf::from("C:/sample.zip/inside"), w, cx);
                view.receive_files(
                    vec![PathBuf::from("C:/source.txt")],
                    PathBuf::from("C:/sample.zip/inside"),
                    crate::file_operations::TransferKind::Move,
                    w,
                    cx,
                );
                view.request_size(PathBuf::from("member.txt"), cx);
                assert!(view.path().is_none());
                assert!(view.name_editor.is_none());
                assert!(view.notice.is_none());
            })
            .unwrap();
    }
}
