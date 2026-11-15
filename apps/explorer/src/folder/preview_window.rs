//! A nonmodal preview follows one source tab, including when it receives keyboard focus.
use super::*;
use crate::media::PreviewContent;
use gpui::{Animation, AnimationExt, AnyWindowHandle, KeyDownEvent, WeakEntity};

pub(super) struct PreviewWindow {
    source: WeakEntity<FolderView>,
    owner: AnyWindowHandle,
    media: Entity<MediaCache>,
    scope: MediaScope,
    entry: Entry,
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
            if !source.preview_open || source.selected_entry().is_none() {
                window.remove_window();
                return;
            }
            this.entry = source.selected_entry().unwrap().clone();
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
                .font_family("Consolas")
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
                                this.close(window, cx);
                                let owner = this.owner;
                                let source = this.source.clone();
                                cx.defer(move |cx| {
                                    let _ = owner.update(cx, |_, window, cx| {
                                        let _ = source.update(cx, |source, cx| {
                                            source.open_selected(&OpenSelected, window, cx);
                                        });
                                    });
                                });
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
                    .h(px(32.))
                    .flex_shrink_0()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .text_color(theme.c("text.muted"))
                    .child(if loading {
                        format!("{} を読み込み中…", self.entry.name)
                    } else {
                        "↑ ↓ 選択を移動  ·  Space / Esc 閉じる".into()
                    }),
            )
            .with_animation(
                "preview-reveal",
                Animation::new(std::time::Duration::from_millis(110)),
                |element, progress| element.opacity(1. - (1. - progress).powi(3)),
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
