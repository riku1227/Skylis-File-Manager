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
enum Command {
    Open,
    NewTab,
    Preview,
    Copy,
    Cut,
    Paste,
    Size,
    CancelSize,
    CopyPath,
    Refresh,
    Shell,
    Reveal,
    Share,
    Services,
}

impl FolderView {
    pub(super) fn open_context_menu(
        &mut self,
        item: Option<usize>,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
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
        if event.modifiers.shift && cfg!(windows) {
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
        if self.input_focused(window, cx) || self.is_home() {
            return;
        }
        self.context_menu = Some(ContextMenu {
            position: gpui::point(px(260.), px(150.)),
            target: self.selected_entry().map(|e| e.path.clone()),
            item: self.selected,
            active: 0,
        });
        window.focus(&self.menu_focus, cx);
        cx.notify();
    }
    fn menu_commands(&self) -> Vec<(&'static str, Command)> {
        let mut commands = Vec::new();
        if self.context_menu.as_ref().is_some_and(|m| m.item.is_some()) {
            commands.push(("開く", Command::Open));
            if self.selected_entry().is_some_and(Entry::is_dir_like) {
                commands.push(("新しいタブで開く", Command::NewTab));
            }
        }
        if self.context_menu.as_ref().is_some_and(|m| m.item.is_some()) {
            commands.push(("クイックプレビュー", Command::Preview));
            commands.push(("コピー", Command::Copy));
            if cfg!(windows) {
                commands.push(("切り取り", Command::Cut));
            }
            if self
                .selected_entry()
                .is_some_and(|e| e.kind == EntryKind::Dir)
            {
                commands.push(("サイズを計算", Command::Size));
            }
            if cfg!(target_os = "macos") {
                commands.push(("Finderで表示", Command::Reveal));
                commands.push(("共有…", Command::Share));
                commands.push(("サービス…", Command::Services));
            }
        }
        if cfg!(any(windows, target_os = "macos")) && self.path().is_some() {
            commands.push(("貼り付け", Command::Paste));
        }
        commands.push(("サイズ計算を中止", Command::CancelSize));
        commands.push(("パスをコピー", Command::CopyPath));
        commands.push(("更新", Command::Refresh));
        if cfg!(windows) {
            commands.push(("その他のオプションを表示…", Command::Shell));
        }
        commands
    }
    fn execute_menu(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
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
            Command::Paste => self.paste_files(&PasteFiles, window, cx),
            Command::Size => {
                for path in self.selected_paths() {
                    self.request_size(path, cx);
                }
            }
            Command::CancelSize => self.stop_sizes(cx),
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
                cx.write_to_clipboard(ClipboardItem::new_string(
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
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
        let menu = self.context_menu.clone().unwrap();
        let commands = self.menu_commands();
        deferred(
            anchored().position(menu.position).snap_to_window().child(
                div()
                    .id("context-menu")
                    .occlude()
                    .track_focus(&self.menu_focus)
                    .w(px(264.))
                    .p(px(4.))
                    .rounded(px(8.))
                    .bg(t.c("surface.raised"))
                    .border_1()
                    .border_color(t.c("border.subtle"))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, w, cx| {
                        this.context_menu = None;
                        w.focus(&this.focus, cx);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                        let commands = this.menu_commands();
                        if let Some(menu) = &mut this.context_menu {
                            match event.keystroke.key.as_str() {
                                "escape" => {
                                    this.context_menu = None;
                                    w.focus(&this.focus, cx);
                                }
                                "down" => menu.active = (menu.active + 1) % commands.len(),
                                "up" => {
                                    menu.active =
                                        (menu.active + commands.len() - 1) % commands.len()
                                }
                                "enter" | "space" => {
                                    let command = commands[menu.active].1;
                                    this.execute_menu(command, w, cx);
                                }
                                _ => (),
                            }
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .children(
                        commands
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (label, command))| {
                                div()
                                    .id(("menu-command", ix))
                                    .h(px(28.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(4.))
                                    .when(ix == menu.active, |d| d.bg(t.c("state.hover")))
                                    .hover(|s| s.bg(t.c("state.hover")))
                                    .cursor_pointer()
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.execute_menu(command, w, cx)
                                    }))
                            }),
                    ),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }
}
