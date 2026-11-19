use super::*;
use explorer_core::archive::{self, ArchiveFormat, ArchiveIndex};
use std::sync::Arc;

#[derive(Clone)]
enum Operation {
    Extract {
        source: PathBuf,
        selected: Vec<String>,
        cached: Option<Arc<ArchiveIndex>>,
    },
    Create {
        paths: Vec<PathBuf>,
        format: ArchiveFormat,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
        cx.background_executor.allow_parking();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, window, cx| {
                window.activate_window();
                let directory = std::env::temp_dir().join("skylis-archive-dialog-model");
                view.history = History::new(Location::Folder(directory.clone()));
                view._drive_task = None;
                view._wsl_task = None;
                view.open_archive_dialog(
                    Operation::Create {
                        paths: vec![directory.join("source.txt")],
                        format: ArchiveFormat::Zip,
                    },
                    directory.join("保存先.zip"),
                    window,
                    cx,
                );
            })
            .unwrap();
        window
    }

    #[gpui::test]
    fn archive_dialog_keeps_focus_when_address_and_search_shortcuts_are_pressed(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            cx.bind_keys([
                gpui::KeyBinding::new("ctrl-l", EditAddress, Some("Explorer")),
                gpui::KeyBinding::new("ctrl-f", FocusSearch, Some("Explorer")),
                gpui::KeyBinding::new("escape", ClearSearch, Some("Explorer")),
            ])
        });
        let window = dialog_window(cx);
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        for key in ["ctrl-l", "ctrl-f"] {
            cx.simulate_keystrokes(window.into(), key);
            window
                .update(cx, |view, window, cx| {
                    let dialog = view.archive_dialog.as_ref().unwrap();
                    assert!(dialog.input.focus_handle(cx).is_focused(window));
                    assert!(!view.editing_address);
                    assert!(!view.search.focus_handle(cx).is_focused(window));
                    assert!(!dialog.pending);
                })
                .unwrap();
        }
        cx.simulate_keystrokes(window.into(), "escape");
        window
            .update(cx, |view, window, _| {
                assert!(view.archive_dialog.is_none());
                assert!(view.focus.is_focused(window));
            })
            .unwrap();
    }

    #[gpui::test]
    fn reactivating_a_tab_restores_its_archive_dialog_focus_and_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = dialog_window(cx);
        window
            .update(cx, |view, window, cx| {
                let input = view.archive_dialog.as_ref().unwrap().input.clone();
                input.update(cx, |input, cx| {
                    input.set_text("D:/作業/入力途中.zip".into(), false, window, cx);
                });
                window.focus(&view.search.focus_handle(cx), cx);
                // Explorer::switch restores this Focusable handle when reactivating a tab.
                window.focus(&view.focus_handle(cx), cx);
                assert!(input.focus_handle(cx).is_focused(window));
                assert_eq!(input.read(cx).content.as_ref(), "D:/作業/入力途中.zip");
                assert!(!view.archive_dialog.as_ref().unwrap().pending);
            })
            .unwrap();
    }

    struct HostedFolder(Entity<FolderView>);
    impl Render for HostedFolder {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(self.0.clone())
        }
    }

    #[gpui::test]
    fn pending_open_finishes_in_moved_tab_before_first_paint_after_source_closes(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let source = dialog_window(cx);
        let folder = source.update(cx, |_, _, cx| cx.entity()).unwrap();
        let (sender, receiver) = async_channel::bounded(1);
        source
            .update(cx, |view, _, cx| {
                view.observe_open_request(receiver, view.cancel.clone(), cx);
                assert!(view._open_request.is_some());
            })
            .unwrap();
        let target_folder = folder.clone();
        let target = cx.add_window(move |window, cx| {
            target_folder.update(cx, |view, cx| {
                view.attach_window(window, cx);
                window.focus(&view.focus_handle(cx), cx);
            });
            HostedFolder(target_folder)
        });
        source
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        sender
            .try_send(Err("owner routing regression fixture".into()))
            .unwrap();
        cx.run_until_parked();
        target
            .update(cx, |host, window, cx| {
                host.0.update(cx, |view, cx| {
                    assert_eq!(view.owner_window, window.window_handle());
                    assert!(view._open_request.is_none());
                    assert_eq!(
                        view.notice.as_deref(),
                        Some("開けませんでした: owner routing regression fixture")
                    );
                    let dialog = view
                        .archive_dialog
                        .as_ref()
                        .expect("moving keeps the draft");
                    assert!(dialog.input.read(cx).content.ends_with("保存先.zip"));
                    assert!(dialog.input.focus_handle(cx).is_focused(window));
                });
            })
            .unwrap();
    }
}

pub(super) struct ArchiveDialog {
    pub(super) input: Entity<TextInput>,
    operation: Operation,
    error: Option<String>,
    pending: bool,
    cancel: CancelToken,
}
impl Drop for ArchiveDialog {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl FolderView {
    pub(super) fn observe_archive_jobs(&mut self, cx: &mut Context<Self>) {
        let jobs = self.operations.clone();
        cx.subscribe(
            &jobs,
            |view, _, completion: &crate::operation_center::ArchiveCompletion, cx| {
                if view.path().is_some_and(|path| {
                    completion
                        .affected_parents
                        .iter()
                        .any(|parent| parent == path)
                }) {
                    view.request_refresh(cx);
                    view.notice = Some(match &completion.output {
                        Ok(path) => format!(
                            "保存しました: {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        Err(_) if completion.cancelled => "圧縮・解凍を中止しました".into(),
                        Err(error) => error.clone(),
                    });
                }
                cx.notify();
            },
        )
        .detach();
    }

    pub(super) fn load_archive(&mut self, source: PathBuf, prefix: String, cx: &mut Context<Self>) {
        let cancel = self.cancel.clone();
        let receiver = match crate::archive_jobs::inspect_async(
            source,
            cancel.clone(),
            self.archive_index.clone(),
        ) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.loading = false;
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let sort = self.sort;
        self._listing = Some(cx.spawn(async move |view, cx| {
            let result = receiver
                .recv()
                .await
                .unwrap_or_else(|_| Err("圧縮ファイルの読み込みを中止しました".into()));
            // Expand implicit directories and allocate row metadata away from paint/input.
            let rows = cx
                .background_executor()
                .spawn(async move {
                    result.map(|index| {
                        let mut rows = archive::children(&index, &prefix)
                            .into_iter()
                            .map(|e| {
                                let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_owned();
                                Entry::virtual_entry(
                                    name,
                                    PathBuf::from(e.path),
                                    if e.is_dir {
                                        EntryKind::Dir
                                    } else {
                                        EntryKind::File
                                    },
                                    (!e.is_dir).then_some(e.size),
                                    e.modified,
                                )
                            })
                            .collect::<Vec<_>>();
                        rows.sort_unstable_by(|a, b| sort.compare(a, b));
                        (index, rows)
                    })
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                if cancel.is_cancelled() {
                    return;
                }
                view.loading = false;
                match rows {
                    Ok((index, entries)) => {
                        view.archive_index = Some(index);
                        if view.sort != sort {
                            view.load(
                                view.history.current().clone(),
                                view.pending_select.clone(),
                                cx,
                            );
                            return;
                        }
                        view.entries = entries;
                        view.rebuild_visible(None);
                        if let Some(name) = view.pending_select.take() {
                            if let Some(ix) = view
                                .visible
                                .iter()
                                .position(|&i| view.entries[i].name == name)
                            {
                                view.select(ix, cx);
                                view.scroll_to_entry(ix, ScrollStrategy::Center);
                            }
                        }
                    }
                    Err(error) => {
                        view.archive_index = None;
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn begin_extract(
        &mut self,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        let (source, members, cached) =
            if let Location::Archive { archive, .. } = self.history.current() {
                let members = if selected {
                    self.selected_paths()
                        .iter()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .collect()
                } else {
                    Vec::new()
                };
                (archive.clone(), members, self.archive_index.clone())
            } else {
                let Some(entry) = self
                    .selected_entry()
                    .filter(|e| archive::detect_format(&e.path).is_some())
                else {
                    return;
                };
                (entry.path.clone(), Vec::new(), None)
            };
        let destination = source.parent().unwrap_or(Path::new("")).to_path_buf();
        self.open_archive_dialog(
            Operation::Extract {
                source,
                selected: members,
                cached,
            },
            destination,
            window,
            cx,
        );
    }

    pub(super) fn begin_compress(
        &mut self,
        format: ArchiveFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        let Some(parent) = self.path() else {
            return;
        };
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let stem = if paths.len() == 1 {
            paths[0]
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            "アーカイブ".into()
        };
        let extension = if format == ArchiveFormat::Zip {
            "zip"
        } else {
            "7z"
        };
        let destination = parent.join(format!("{stem}.{extension}"));
        self.open_archive_dialog(Operation::Create { paths, format }, destination, window, cx);
    }

    fn open_archive_dialog(
        &mut self,
        operation: Operation,
        destination: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_scroll_motion();
        self.context_menu = None;
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_text(destination.display().to_string(), true, window, cx);
            input
        });
        window.focus(&input.focus_handle(cx), cx);
        self.archive_dialog = Some(ArchiveDialog {
            input,
            operation,
            error: None,
            pending: false,
            cancel: CancelToken::new(),
        });
        cx.notify();
    }

    fn cancel_archive_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .archive_dialog
            .as_ref()
            .is_some_and(|d| d.input.read(cx).is_composing())
        {
            return;
        }
        self.archive_dialog = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn submit_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.archive_dialog else {
            return;
        };
        if dialog.pending || dialog.input.read(cx).is_composing() {
            return;
        }
        let text = dialog.input.read(cx).content.to_string();
        let destination = PathBuf::from(text.trim().trim_matches('"'));
        if !destination.is_absolute() || text.contains(['\0', '\r', '\n']) {
            dialog.error = Some("保存先を絶対パスで指定してください".into());
            cx.notify();
            return;
        }
        let operation = dialog.operation.clone();
        let jobs = self.operations.clone();
        let owner = window.window_handle();
        match operation {
            Operation::Create { paths, format } => {
                match jobs.update(cx, |jobs, cx| {
                    jobs.submit_create(paths, destination, format, window.window_handle(), cx)
                }) {
                    Ok(_) => {
                        self.archive_dialog = None;
                        window.focus(&self.focus, cx);
                        self.notice = None;
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
            Operation::Extract {
                source,
                selected,
                cached,
            } => {
                let cancel = dialog.cancel.clone();
                let receiver =
                    match crate::archive_jobs::inspect_async(source, cancel.clone(), cached) {
                        Ok(receiver) => receiver,
                        Err(error) => {
                            dialog.error = Some(error);
                            cx.notify();
                            return;
                        }
                    };
                dialog.pending = true;
                cx.spawn(async move |view, cx| {
                    let result = receiver
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err("圧縮ファイルの読み込みを中止しました".into()));
                    let _ = Self::with_owner(&view, cx, |view, window, cx| {
                        if cancel.is_cancelled() {
                            return;
                        }
                        let result = result.and_then(|index| {
                            jobs.update(cx, |jobs, cx| {
                                jobs.submit_extract(index, selected, destination, owner, cx)
                            })
                        });
                        match result {
                            Ok(_) => {
                                let editing_focused =
                                    view.archive_dialog.as_ref().is_some_and(|dialog| {
                                        dialog.input.focus_handle(cx).is_focused(window)
                                    });
                                view.archive_dialog = None;
                                if editing_focused {
                                    window.focus(&view.focus, cx);
                                }
                                view.notice = None;
                            }
                            Err(error) => {
                                if let Some(dialog) = &mut view.archive_dialog {
                                    dialog.pending = false;
                                    dialog.error = Some(error);
                                }
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    pub(super) fn render_archive_bar(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(14.))
            .py(px(8.))
            .text_size(px(12.))
            .bg(t.c("surface.chrome"))
            .child(div().flex_1().child("圧縮ファイル内 · 読み取り専用"))
            .child(
                div()
                    .id("archive-extract-all")
                    .text_color(t.c("accent.fg"))
                    .cursor_pointer()
                    .child("すべて解凍…")
                    .on_click(cx.listener(|this, _, w, cx| this.begin_extract(false, w, cx))),
            )
            .child(
                div()
                    .id("archive-default-open")
                    .cursor_pointer()
                    .child("既定のアプリで開く")
                    .on_click(cx.listener(|this, _, _, cx| this.open_archive_external(cx))),
            )
            .into_any_element()
    }

    pub(super) fn open_archive_external(&mut self, cx: &mut Context<Self>) {
        let source = match self.history.current() {
            Location::Archive { archive, .. } => Some(archive.clone()),
            _ => self.selected_entry().map(|e| e.path.clone()),
        };
        if let Some(source) = source {
            if self._open_request.is_some() {
                return;
            }
            let generation = self.cancel.clone();
            match crate::file_open::request(source, false, generation.clone()) {
                Ok(receiver) => {
                    self._open_request = Some(cx.spawn(async move |view, cx| {
                        let result = receiver.recv().await;
                        let _ = view.update(cx, |view, cx| {
                            if generation.is_cancelled() {
                                return;
                            }
                            view._open_request = None;
                            if let Ok(Err(error)) = result {
                                view.notice = Some(error);
                            }
                            cx.notify();
                        });
                    }))
                }
                Err(error) => {
                    self.notice = Some(error);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn render_archive_dialog(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = &self.archive_dialog else {
            return div().into_any_element();
        };
        dialog.input.update(cx, |input, _| input.theme = t);
        let extracting = matches!(dialog.operation, Operation::Extract { .. });
        div().absolute().inset_0().occlude().flex().items_center().justify_center().bg(t.c("surface.canvas").opacity(0.75))
            .on_action(|_: &EditAddress, _, cx| cx.stop_propagation())
            .on_action(|_: &FocusSearch, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &OpenSelected, w, cx| { this.submit_archive(w, cx); cx.stop_propagation(); }))
            .on_action(cx.listener(|this, _: &ClearSearch, w, cx| { this.cancel_archive_dialog(w, cx); cx.stop_propagation(); }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                match event.keystroke.key.as_str() { "enter" => this.submit_archive(w, cx), "escape" => this.cancel_archive_dialog(w, cx), _ => return }
                cx.stop_propagation();
            }))
            .child(div().w(px(560.)).p(px(20.)).rounded(px(8.)).bg(t.c("surface.raised")).border_1().border_color(t.c("border.subtle")).shadow_md().flex().flex_col().gap(px(14.))
                .child(if extracting { "解凍先のフォルダー" } else { "圧縮ファイルの保存先" })
                .child(div().h(px(34.)).px(px(8.)).border_1().border_color(t.c("border.subtle")).child(dialog.input.clone()))
                .child(div().text_size(px(12.)).text_color(t.c("text.muted")).child(if extracting { "指定先の中に新しいフォルダーを作ります。同名の場合は番号を付けます。" } else { "元のファイルはそのまま残ります。既存の圧縮ファイルは上書きしません。" }))
                .when_some(dialog.error.clone(), |d, error| d.child(div().text_size(px(12.)).text_color(t.c("danger.fg")).child(error)))
                .child(div().flex().justify_end().gap(px(20.))
                    .child(div().id("archive-cancel").cursor_pointer().child("キャンセル").on_click(cx.listener(|this, _, w, cx| this.cancel_archive_dialog(w, cx))))
                    .child(div().id("archive-submit").text_color(t.c("accent.fg")).cursor_pointer().child(if dialog.pending { "確認中…" } else if extracting { "解凍" } else { "圧縮" }).on_click(cx.listener(|this, _, w, cx| this.submit_archive(w, cx))))))
            .into_any_element()
    }
}
