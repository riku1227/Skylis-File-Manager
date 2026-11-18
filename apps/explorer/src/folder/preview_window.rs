//! A nonmodal preview follows one source tab, including when it receives keyboard focus.
use super::*;
use crate::media::PreviewContent;
use gpui::{Animation, AnimationExt, AnyWindowHandle, KeyDownEvent, WeakEntity};

pub(super) struct ImageSequence {
    entries: Vec<Entry>,
    index: usize,
}

#[derive(Clone, Copy)]
enum ImageStep {
    Previous,
    Next,
}

impl ImageSequence {
    pub(super) fn new(entries: Vec<Entry>, path: &Path) -> Option<Self> {
        let index = entries.iter().position(|entry| entry.path == path)?;
        Some(Self { entries, index })
    }

    pub(super) fn current(&self) -> &Entry {
        &self.entries[self.index]
    }

    fn neighbor(&self, step: ImageStep) -> Option<usize> {
        match step {
            ImageStep::Previous => self.index.checked_sub(1),
            ImageStep::Next => (self.index + 1 < self.entries.len()).then_some(self.index + 1),
        }
    }
}

pub(super) struct PreviewWindow {
    source: WeakEntity<FolderView>,
    owner: AnyWindowHandle,
    media: Entity<MediaCache>,
    scope: MediaScope,
    entry: Entry,
    sequence: Option<ImageSequence>,
    displayed: Option<(Entry, PreviewContent)>,
    mode: ThemeMode,
    focus: FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl PreviewWindow {
    #[cfg(test)]
    pub(super) fn entry_name_for_test(&self) -> &str {
        &self.entry.name
    }

    fn present(&mut self, content: PreviewContent) {
        if matches!(content, PreviewContent::Loading) {
            return;
        }
        if let Some((previous, PreviewContent::Image(old))) = &self.displayed {
            if previous.path == self.entry.path
                && previous.modified == self.entry.modified
                && previous.size == self.entry.size
                && let PreviewContent::Image(new) = &content
                && new.as_bytes(0).map_or(0, <[u8]>::len) < old.as_bytes(0).map_or(0, <[u8]>::len)
            {
                return;
            }
        }
        self.displayed = Some((self.entry.clone(), content));
    }

    pub(super) fn new(
        source: Entity<FolderView>,
        owner: AnyWindowHandle,
        entry: Entry,
        sequence: Option<ImageSequence>,
        media: Entity<MediaCache>,
        scope: MediaScope,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let source_changed = cx.observe_in(&source, window, |this, source, window, cx| {
            let source = source.read(cx);
            if !source.preview_open {
                window.remove_window();
                return;
            }
            if this.sequence.is_none() {
                let Some(entry) = source.selected_entry() else {
                    window.remove_window();
                    return;
                };
                this.entry = entry.clone();
            }
            this.mode = source.mode;
            cx.notify();
        });
        let source_released = cx.observe_release_in(&source, window, |_, _, window, _| {
            window.remove_window();
        });
        let media_changed = cx.observe(&media, |_, _, cx| cx.notify());
        let weak = source.downgrade();
        let preview_id = window.window_handle().window_id();
        cx.on_release(move |_, cx| {
            // Closing an older window must not clear a newly opened preview.
            cx.defer(move |cx| {
                let _ = weak.update(cx, |source, cx| {
                    if source
                        .preview_window
                        .is_some_and(|w| w.window_id() == preview_id)
                    {
                        source.preview_window = None;
                        source.preview_open = false;
                        source.preview_scope.cancel();
                        cx.notify();
                    }
                });
            });
        })
        .detach();
        Self {
            source: source.downgrade(),
            owner,
            media,
            scope,
            entry,
            sequence,
            displayed: None,
            mode,
            focus,
            _subscriptions: vec![source_changed, source_released, media_changed],
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scope.cancel();
        window.remove_window();
        let owner = self.owner;
        cx.defer(move |cx| {
            let _ = owner.update(cx, |_, window, _| window.activate_window());
        });
    }

    fn forward_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if matches!(event.keystroke.key.as_str(), "escape" | "space") {
            self.close(window, cx);
            return;
        }
        if self.sequence.is_some() {
            if !event.keystroke.modifiers.modified() {
                match event.keystroke.key.as_str() {
                    "left" => self.step_image(ImageStep::Previous, cx),
                    "right" => self.step_image(ImageStep::Next, cx),
                    _ => (),
                }
            }
            return;
        }
        let source = self.source.clone();
        let owner = self.owner;
        let key = event.keystroke.clone();
        // Dispatch against the actual source window's focus tree and keymap. This
        // preserves selection, tab and file shortcuts without a second key table.
        cx.defer(move |cx| {
            let _ = owner.update(cx, |_, window, cx| {
                if let Some(source) = source.upgrade() {
                    // Keep the original focus target: Ctrl+F/Ctrl+L may have
                    // moved it into an input on a previous forwarded key.
                    window.dispatch_keystroke(key, cx);
                    if source.read(cx).input_focused(window, cx)
                        || source.read(cx).context_menu.is_some()
                    {
                        window.activate_window();
                    }
                }
            });
        });
    }

    fn step_image(&mut self, step: ImageStep, cx: &mut Context<Self>) {
        if let Some(sequence) = &mut self.sequence
            && let Some(index) = sequence.neighbor(step)
        {
            sequence.index = index;
            self.scope.cancel();
            self.entry = sequence.current().clone();
            cx.notify();
        }
    }

    fn open_with_default_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .displayed
            .as_ref()
            .map_or(&self.entry, |(entry, _)| entry);
        let (path, kind) = (entry.path.clone(), entry.kind);
        self.close(window, cx);
        let owner = self.owner;
        let source = self.source.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |_, window, cx| {
                let _ = source.update(cx, |source, cx| {
                    source.open_filesystem_target(path, kind, window, cx);
                });
            });
        });
    }

    fn image_navigation(
        sequence: &ImageSequence,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut nav = div().flex().items_center().gap(px(12.));
        for (id, label, step) in [
            ("previous-image", "← 前へ", ImageStep::Previous),
            ("next-image", "次へ →", ImageStep::Next),
        ] {
            if matches!(step, ImageStep::Next) {
                nav = nav.child(format!(
                    "{} / {}",
                    sequence.index + 1,
                    sequence.entries.len()
                ));
            }
            let enabled = sequence.neighbor(step).is_some();
            nav = nav.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .rounded(px(4.))
                    .px(px(8.))
                    .py(px(4.))
                    .child(label)
                    .when(enabled, |button| {
                        button
                            .text_color(theme.c("text.primary"))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.c("state.hover")))
                            .on_click(cx.listener(move |this, _, _, cx| this.step_image(step, cx)))
                    })
                    .when(!enabled, |button| button.opacity(0.45)),
            );
        }
        nav
    }
}

impl Render for PreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::resolve(self.mode, window.appearance());
        let high_refresh = cx
            .try_global::<crate::settings::SharedPreferences>()
            .is_none_or(|p| p.0.read(cx).value.high_refresh_rate);
        let _ = window.set_frame_rate_limit((!high_refresh).then_some(60.));
        let content = self.media.update(cx, |media, cx| {
            media.get_preview(&self.entry, &self.scope, cx)
        });
        let loading = matches!(content, PreviewContent::Loading);
        self.present(content);
        let window_title = if self.sequence.is_some() {
            "画像ビューア"
        } else {
            "クイックプレビュー"
        };
        let footer_status = if loading {
            format!("{} を読み込み中…", self.entry.name)
        } else if self.sequence.is_some() {
            "Esc で閉じる".into()
        } else {
            "↑ ↓ 選択を移動  ·  Space / Esc 閉じる".into()
        };
        let title = self
            .displayed
            .as_ref()
            .map(|(entry, _)| &entry.name)
            .unwrap_or(&self.entry.name)
            .clone();
        let body = match self.displayed.as_ref() {
            Some((_, PreviewContent::Image(image))) => div()
                .size_full()
                .relative()
                .child(
                    img(image.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
                .into_any_element(),
            Some((_, PreviewContent::Text { text, truncated })) => div()
                .id("preview-text")
                .size_full()
                .overflow_y_scroll()
                .font_family(if cfg!(target_os = "macos") {
                    "Menlo"
                } else {
                    "Consolas"
                })
                .text_size(px(13.))
                .child(text.to_string())
                .when(*truncated, |d| {
                    d.child(
                        div()
                            .mt(px(16.))
                            .text_color(theme.c("text.muted"))
                            .child("先頭128 KiBのみ表示しています"),
                    )
                })
                .into_any_element(),
            Some((entry, PreviewContent::Metadata { reason })) => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(entry.kind_label())
                .child(entry.size.map(format::size).unwrap_or_default())
                .child(
                    div()
                        .text_color(theme.c("text.muted"))
                        .child(reason.to_string()),
                )
                .into_any_element(),
            _ => div()
                .text_color(theme.c("text.muted"))
                .child("読み込み中…")
                .into_any_element(),
        };
        div()
            .id("quick-preview-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.c("surface.canvas"))
            .text_color(theme.c("text.primary"))
            .text_size(px(12.))
            .on_key_down(cx.listener(Self::forward_key))
            .when(cfg!(windows), |d| {
                d.child(
                    div()
                        .id("preview-titlebar")
                        .h(px(38.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .bg(theme.c("surface.canvas"))
                        .child(
                            div()
                                .id("preview-titlebar-drag")
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .px(px(16.))
                                .flex()
                                .items_center()
                                .text_color(theme.c("text.muted"))
                                .window_control_area(gpui::WindowControlArea::Drag)
                                .child(window_title),
                        )
                        .child(crate::explorer::window_controls(
                            window.is_maximized(),
                            false,
                            theme,
                        )),
                )
            })
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(theme.c("border.subtle"))
                    .child(div().flex_1().truncate().child(title))
                    .child(
                        div()
                            .id("preview-open")
                            .cursor_pointer()
                            .text_color(theme.c("accent.fg"))
                            .child("既定のアプリで開く")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_with_default_app(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .p(px(16.))
                    .child(body),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .text_color(theme.c("text.muted"))
                    .child(div().flex_1().min_w_0().truncate().child(footer_status))
                    .when_some(self.sequence.as_ref(), |footer, sequence| {
                        footer.child(Self::image_navigation(sequence, theme, cx))
                    }),
            )
            .with_animation(
                "preview-reveal",
                Animation::new(crate::motion::WINDOW_REVEAL),
                |element, progress| element.opacity(crate::motion::ease_out_cubic(progress)),
            )
    }
}

/// Keep the preview above its owner only; other applications are never covered by
/// a global topmost window. This small adapter contains the Windows integration.
#[cfg(windows)]
pub(super) fn native_owner(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}
#[cfg(windows)]
pub(super) fn attach_native_owner(window: &Window, owner: Option<isize>) {
    if let (Some(hwnd), Some(owner)) = (native_owner(window), owner) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                hwnd as _,
                windows_sys::Win32::UI::WindowsAndMessaging::GWLP_HWNDPARENT,
                owner,
            );
        }
    }
}
#[cfg(not(windows))]
pub(super) fn native_owner(_: &Window) -> Option<isize> {
    None
}
#[cfg(not(windows))]
pub(super) fn attach_native_owner(_: &Window, _: Option<isize>) {}
