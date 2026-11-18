use super::name_operations::{self, NameOperation};
use super::*;

pub(super) struct NameEditor {
    pub(super) input: Entity<TextInput>,
    operation: NameOperation,
    title: &'static str,
    origin: Location,
    pub(super) error: Option<String>,
    pending: bool,
    _blur: Option<gpui::Subscription>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::EntityInputHandler;

    fn editor_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
        cx.background_executor.allow_parking();
        let window = cx.add_window(|window, cx| {
            let icons = cx.new(ShellIcons::new);
            FolderView::new(Location::Home, ThemeMode::Light, icons, window, cx)
        });
        window
            .update(cx, |view, window, cx| {
                window.activate_window();
                // Only model entries: accidental submit/delete during the regression tests
                // cannot touch repository files or an existing user directory.
                let directory = std::env::temp_dir()
                    .join(format!("skylis-editor-model-{}", std::process::id()));
                let target = directory.join("original.txt");
                view.history = History::new(Location::Folder(directory));
                view._drive_task = None;
                view._wsl_task = None;
                view.grouped = false;
                view.pane = false;
                view.entries = vec![Entry::virtual_entry(
                    "original.txt".into(),
                    target.clone(),
                    EntryKind::File,
                    Some(1),
                    None,
                )];
                view.rebuild_visible(Some(&target));
                view.begin_name_edit(NameOperation::Rename(target), window, cx);
            })
            .unwrap();
        window
    }

    #[gpui::test]
    fn composition_blocks_submit_cancel_and_blur_commit(cx: &mut gpui::TestAppContext) {
        let window = editor_window(cx);
        window
            .update(cx, |view, window, cx| {
                let input = view.name_editor.as_ref().unwrap().input.clone();
                input.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(
                        Some(0..8),
                        "日本語",
                        Some(3..3),
                        window,
                        cx,
                    );
                });
                view.submit_name(window, cx);
                view.cancel_name_edit(window, cx);
                assert!(
                    view.name_editor
                        .as_ref()
                        .is_some_and(|editor| !editor.pending)
                );
                window.focus(&view.search.focus_handle(cx), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                let input = view.name_editor.as_ref().unwrap().input.clone();
                assert!(!view.name_editor.as_ref().unwrap().pending);
                assert!(input.read(cx).is_composing());
                input.update(cx, |input, cx| input.unmark_text(window, cx));
                view.cancel_name_edit(window, cx);
                assert!(view.name_editor.is_none());
                assert!(view.focus.is_focused(window));
            })
            .unwrap();
    }

    #[gpui::test]
    fn delete_edits_text_and_permanent_delete_does_not_escape_the_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            crate::search::bind_keys(cx);
            cx.bind_keys([
                gpui::KeyBinding::new("delete", DeleteFiles, Some("Explorer")),
                gpui::KeyBinding::new("shift-delete", DeletePermanently, Some("Explorer")),
            ]);
        });
        let window = editor_window(cx);
        window
            .update(cx, |view, window, cx| {
                view.name_editor
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| {
                        input.set_text("remove this text.txt".into(), true, window, cx)
                    });
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "delete");
        cx.simulate_keystrokes(window.into(), "shift-delete");
        window
            .update(cx, |view, _, cx| {
                let editor = view.name_editor.as_ref().unwrap();
                assert_eq!(editor.input.read(cx).content.as_ref(), "");
                assert!(!editor.pending);
                assert_eq!(view.selected_entry().unwrap().name, "original.txt");
                assert!(view.notice.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn replacing_an_editor_does_not_submit_the_new_editor_on_old_blur(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = editor_window(cx);
        window
            .update(cx, |view, window, cx| {
                view.name_editor
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| {
                        input.set_text("../invalid".into(), false, window, cx)
                    });
                let directory = view.path().unwrap().to_path_buf();
                view.begin_name_edit(NameOperation::Folder(directory), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                let editor = view.name_editor.as_ref().unwrap();
                assert!(matches!(editor.operation, NameOperation::Folder(_)));
                assert!(!editor.pending);
                assert!(editor.error.is_none());
                assert!(editor.input.focus_handle(cx).is_focused(window));
            })
            .unwrap();
    }

    struct HostedEditor(Entity<FolderView>);
    impl Render for HostedEditor {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    #[gpui::test]
    fn migrated_editor_ignores_old_window_blur_and_commits_on_new_window_blur(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = editor_window(cx);
        cx.update_window(source.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        let folder = source
            .update(cx, |view, window, cx| {
                view.name_editor
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| {
                        // Validation fails before any filesystem operation can be dispatched.
                        input.set_text("../invalid".into(), false, window, cx);
                    });
                cx.entity()
            })
            .unwrap();
        cx.run_until_parked();
        let hosted = folder.clone();
        let target = cx.add_window(move |window, cx| {
            window.activate_window();
            hosted.update(cx, |view, cx| {
                view.attach_window(window, cx);
                window.focus(&view.focus_handle(cx), cx);
            });
            HostedEditor(hosted)
        });
        cx.update_window(target.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
        source
            .update(cx, |view, window, cx| {
                window.focus(&view.search.focus_handle(cx), cx);
            })
            .unwrap();
        cx.update_window(source.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
        target
            .update(cx, |host, window, cx| {
                host.0.update(cx, |view, cx| {
                    let editor = view.name_editor.as_ref().unwrap();
                    assert!(
                        editor.error.is_none(),
                        "old host must not submit the moved draft"
                    );
                    assert_eq!(editor.input.read(cx).content.as_ref(), "../invalid");
                    window.focus(&view.search.focus_handle(cx), cx);
                });
            })
            .unwrap();
        cx.update_window(target.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
        target
            .update(cx, |host, window, cx| {
                host.0.update(cx, |view, cx| {
                    let editor = view.name_editor.as_ref().unwrap();
                    assert!(
                        editor.error.is_some(),
                        "blur in the new host still commits/validates"
                    );
                    assert!(!editor.pending);
                    assert!(editor.input.focus_handle(cx).is_focused(window));
                });
            })
            .unwrap();
    }
}

impl FolderView {
    pub(super) fn inline_name_target(&self) -> Option<&Path> {
        match &self.name_editor.as_ref()?.operation {
            NameOperation::Rename(path) => Some(path),
            _ => None,
        }
    }

    pub(super) fn render_inline_name(
        &self,
        path: &Path,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.inline_name_target() != Some(path) {
            return None;
        }
        let editor = self.name_editor.as_ref()?;
        editor.input.update(cx, |input, _| input.theme = t);
        Some(
            div()
                .id("inline-name-editor")
                .debug_selector(|| "inline-name-editor".into())
                .flex_1()
                .min_w_0()
                .h(px(24.))
                .px(px(4.))
                .border_1()
                .rounded(px(3.))
                .border_color(t.c(if editor.error.is_some() {
                    "danger.fg"
                } else {
                    "accent.fg"
                }))
                .bg(t.c("surface.canvas"))
                .when(editor.pending, |d| d.opacity(0.65))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                )
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|this, _: &OpenSelected, w, cx| {
                    this.submit_name(w, cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|this, _: &ClearSearch, w, cx| {
                    this.cancel_name_edit(w, cx);
                    cx.stop_propagation();
                }))
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" => this.submit_name(w, cx),
                        "escape" => this.cancel_name_edit(w, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }))
                .child(editor.input.clone())
                .into_any_element(),
        )
    }

    fn cancel_name_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .name_editor
            .as_ref()
            .is_some_and(|editor| editor.pending || editor.input.read(cx).is_composing())
        {
            return;
        }
        self.name_editor = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn begin_name_edit(
        &mut self,
        operation: NameOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        if self.path().is_none() {
            return;
        }
        if self
            .name_editor
            .as_ref()
            .is_some_and(|editor| editor.pending)
        {
            return;
        }
        let (title, name) = match &operation {
            NameOperation::Folder(_) => ("フォルダーを作成", "新しいフォルダー".to_owned()),
            NameOperation::Text(_) => (
                "テキスト ドキュメントを作成",
                "新しいテキスト ドキュメント.txt".to_owned(),
            ),
            NameOperation::Rename(path) => (
                "名前の変更",
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        };
        self.cancel_scroll_motion();
        self.context_menu = None;
        if let NameOperation::Rename(path) = &operation {
            let Some(ix) = self
                .visible
                .iter()
                .position(|&i| self.entries[i].path == *path)
            else {
                return;
            };
            self.select(ix, cx);
            self.scroll_to_entry(ix, ScrollStrategy::Center);
        }
        let input = cx.new(TextInput::new);
        input.update(cx, |input, cx| input.set_text(name, true, window, cx));
        let file_name = match &operation {
            NameOperation::Text(_) => true,
            NameOperation::Rename(path) => self
                .entries
                .iter()
                .any(|entry| &entry.path == path && !entry.is_dir_like()),
            _ => false,
        };
        if file_name {
            input.update(cx, |input, cx| input.select_filename_stem(cx));
        }
        window.focus(&input.focus_handle(cx), cx);
        self.name_editor = Some(NameEditor {
            input,
            operation,
            title,
            origin: self.history.current().clone(),
            error: None,
            pending: false,
            _blur: None,
        });
        self.attach_name_editor_window(window, cx);
        cx.notify();
    }

    pub(super) fn attach_name_editor_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = &mut self.name_editor else {
            return;
        };
        editor._blur = None;
        if !matches!(editor.operation, NameOperation::Rename(_)) {
            return;
        }
        let input_id = editor.input.entity_id();
        let owner = window.window_handle();
        editor._blur = Some(cx.on_blur(
            &editor.input.focus_handle(cx),
            window,
            move |this, window, cx| {
                // A callback already queued by the previous host must not submit a
                // migrated draft or a newer editor when that old window changes focus.
                if this.owner_window != owner
                    || window.window_handle() != owner
                    || this
                        .name_editor
                        .as_ref()
                        .is_none_or(|editor| editor.input.entity_id() != input_id)
                {
                    return;
                }
                this.submit_name(window, cx);
            },
        ));
    }

    pub(super) fn rename_selected(
        &mut self,
        _: &RenameSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focused(window, cx) || self.multi_selection.len() > 1 {
            return;
        }
        if let Some(entry) = self.selected_entry() {
            self.begin_name_edit(NameOperation::Rename(entry.path.clone()), window, cx);
        }
    }

    fn submit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.name_editor else {
            return;
        };
        if editor.pending || editor.input.read(cx).is_composing() {
            return;
        }
        let name = editor.input.read(cx).content.to_string();
        if let Err(error) = name_operations::validate_name(&name) {
            editor.error = Some(error);
            window.focus(&editor.input.focus_handle(cx), cx);
            cx.notify();
            return;
        }
        editor.pending = true;
        let operation = editor.operation.clone();
        let origin = editor.origin.clone();
        let editor_id = editor.input.entity_id();
        let worker = cx
            .background_executor()
            .spawn(async move { name_operations::execute(&operation, &name) });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = Self::with_owner(&view, cx, |view, window, cx| {
                if *view.history.current() != origin {
                    return;
                }
                if view
                    .name_editor
                    .as_ref()
                    .is_none_or(|editor| editor.input.entity_id() != editor_id)
                {
                    return;
                }
                match result {
                    Ok(path) => {
                        let editing_focused = view
                            .name_editor
                            .as_ref()
                            .is_some_and(|editor| editor.input.focus_handle(cx).is_focused(window));
                        view.name_editor = None;
                        if editing_focused {
                            window.focus(&view.focus, cx);
                        }
                        view.load(
                            origin,
                            path.file_name().map(|n| n.to_string_lossy().into_owned()),
                            cx,
                        );
                    }
                    Err(error) => {
                        if let Some(editor) = &mut view.name_editor {
                            editor.pending = false;
                            editor.error = Some(error);
                            if view.focus.contains_focused(window, cx) {
                                window.focus(&editor.input.focus_handle(cx), cx);
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_name_editor(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let editor = self.name_editor.as_ref().unwrap();
        editor.input.update(cx, |input, _| input.theme = t);
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.c("surface.canvas").opacity(0.75))
            .on_action(cx.listener(|this, _: &OpenSelected, window, cx| {
                this.submit_name(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| {
                this.cancel_name_edit(window, cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.submit_name(window, cx),
                    "escape" => {
                        this.cancel_name_edit(window, cx);
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .w(px(420.))
                    .p(px(20.))
                    .rounded(px(8.))
                    .bg(t.c("surface.raised"))
                    .border_1()
                    .border_color(t.c("border.subtle"))
                    .shadow_md()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(editor.title)
                    .child(
                        div()
                            .h(px(34.))
                            .px(px(8.))
                            .border_1()
                            .border_color(t.c("border.subtle"))
                            .child(editor.input.clone()),
                    )
                    .when_some(editor.error.clone(), |d, error| {
                        d.child(div().text_color(t.c("text.muted")).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(18.))
                            .child(
                                div()
                                    .id("name-cancel")
                                    .cursor_pointer()
                                    .child("キャンセル")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_name_edit(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("name-submit")
                                    .cursor_pointer()
                                    .text_color(t.c("accent.fg"))
                                    .child(if editor.pending {
                                        "処理中…"
                                    } else {
                                        "保存"
                                    })
                                    .on_click(
                                        cx.listener(|this, _, w, cx| this.submit_name(w, cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
