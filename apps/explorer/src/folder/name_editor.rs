use super::name_operations::{self, NameOperation};
use super::*;

pub(super) struct NameEditor {
    pub(super) input: Entity<TextInput>,
    operation: NameOperation,
    title: &'static str,
    origin: Location,
    pub(super) error: Option<String>,
    pending: bool,
}

impl FolderView {
    pub(super) fn begin_name_edit(
        &mut self,
        operation: NameOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        });
        cx.notify();
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
        cx.spawn_in(window, async move |view, cx| {
            let result = worker.await;
            let _ = view.update_in(cx, |view, window, cx| {
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
                        view.name_editor = None;
                        window.focus(&view.focus, cx);
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
                this.name_editor = None;
                window.focus(&this.focus, cx);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.submit_name(window, cx),
                    "escape" => {
                        this.name_editor = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
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
                                        this.name_editor = None;
                                        window.focus(&this.focus, cx);
                                        cx.notify();
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
