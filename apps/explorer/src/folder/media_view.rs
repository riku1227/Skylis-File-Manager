use super::*;
use crate::media::{MediaCache, PreviewContent};

#[derive(Clone)]
pub(super) struct SharedMedia(pub Entity<MediaCache>);
impl gpui::Global for SharedMedia {}

impl FolderView {
    pub(super) fn toggle_preview(
        &mut self,
        _: &QuickPreview,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focused(w, cx) {
            return;
        }
        #[cfg(target_os = "macos")]
        if crate::media::quicklook::is_visible(w).unwrap_or(false) {
            let _ = crate::media::quicklook::close(w);
            self.close_preview(cx);
            return;
        }
        #[cfg(not(target_os = "macos"))]
        if self.preview_open {
            self.close_preview(cx);
            return;
        }
        if self.selected_entry().is_none() {
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.selected_entry() {
            if let Err(error) = crate::media::quicklook::show(&entry.path, w) {
                self.notice = Some(error.to_string());
                cx.notify();
                return;
            }
        }
        self.cancel_scroll_motion();
        self.preview_scope.cancel();
        self.preview_open = true;
        cx.notify();
    }
    pub(super) fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = false;
        self.preview_scope.cancel();
        cx.notify();
    }
    pub(super) fn render_preview(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(entry) = self.selected_entry() else {
            return div().into_any_element();
        };
        let content = self.media.update(cx, |media, cx| {
            media.get_preview(entry, &self.preview_scope, cx)
        });
        let content = match content {
            PreviewContent::Loading => div()
                .text_color(t.c("text.muted"))
                .child("プレビューを読み込み中…")
                .into_any_element(),
            PreviewContent::Image(image) => div()
                .relative()
                .size_full()
                .overflow_hidden()
                .child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
                .into_any_element(),
            PreviewContent::Text { text, truncated } => div()
                .id("preview-text")
                .size_full()
                .overflow_y_scroll()
                .font_family("Consolas")
                .text_size(px(13.))
                .child(text.to_string())
                .when(truncated, |d| {
                    d.child(
                        div()
                            .mt(px(16.))
                            .text_color(t.c("text.muted"))
                            .child("先頭128 KiBのみ表示しています"),
                    )
                })
                .into_any_element(),
            PreviewContent::Metadata { reason } => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(entry.kind_label())
                .child(entry.size.map(format::size).unwrap_or_default())
                .child(
                    div()
                        .text_color(t.c("text.muted"))
                        .child(reason.to_string()),
                )
                .into_any_element(),
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .p(px(28.))
            .bg(t.c("surface.canvas").opacity(0.94))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.c("border.subtle"))
                    .bg(t.c("surface.raised"))
                    .child(
                        div()
                            .h(px(44.))
                            .flex_shrink_0()
                            .px(px(16.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(div().flex_1().truncate().child(entry.name.clone()))
                            .child(
                                div()
                                    .id("preview-open")
                                    .cursor_pointer()
                                    .text_color(t.c("accent.fg"))
                                    .child("既定のアプリで開く")
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.close_preview(cx);
                                        this.open_selected(&OpenSelected, w, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .p(px(16.))
                            .child(content),
                    )
                    .child(
                        div()
                            .h(px(32.))
                            .flex_shrink_0()
                            .px(px(16.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_color(t.c("text.muted"))
                            .child("↑ ↓ 選択を移動  ·  Space / Esc 閉じる")
                            .child(
                                div()
                                    .id("preview-close")
                                    .cursor_pointer()
                                    .child("閉じる")
                                    .on_click(cx.listener(|this, _, _, cx| this.close_preview(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn set_gallery(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.gallery = enabled;
        self.gallery_override = Some(enabled);
        self.media_scope.cancel();
        self.rebuild_display_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }
    pub(super) fn choose_auto_gallery(&mut self, cx: &mut Context<Self>) {
        if self.gallery_override.is_none() {
            let enabled = cx
                .try_global::<crate::settings::SharedPreferences>()
                .is_none_or(|p| p.0.read(cx).value.auto_gallery);
            let gallery = enabled
                && crate::media::should_use_gallery(self.visible.iter().map(|&i| &self.entries[i]));
            if gallery != self.gallery {
                self.gallery = gallery;
                self.rebuild_display_rows();
            }
        }
    }
    pub(super) fn render_gallery_rows(
        &mut self,
        range: Range<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = self.theme(w);
        range
            .filter_map(|row| {
                let (label, indices) = self.gallery_rows.get(row)?.clone();
                let well = if self.compact { 96. } else { 112. };
                let mut tiles = div().flex().gap(px(8.)).w_full();
                for ix in indices {
                    let entry = self.entries[self.visible[ix]].clone();
                    let dragging = self.is_drag_source(&entry.path, cx);
                    let thumb = self.media.update(cx, |media, cx| {
                        media.get_thumbnail(
                            &entry,
                            (well * w.scale_factor()).ceil() as u32,
                            &self.media_scope,
                            cx,
                        )
                    });
                    let picture = thumb
                        .map(|image| {
                            img(image)
                                .size_full()
                                .object_fit(gpui::ObjectFit::Contain)
                                .into_any_element()
                        })
                        .unwrap_or_else(|| {
                            self.object_icon(&entry.path, entry.is_dir_like(), 48., w, cx)
                        });
                    tiles = tiles.child(
                        div()
                            .id(("gallery-item", ix))
                            .flex_1()
                            .min_w_0()
                            .p(px(4.))
                            .rounded(px(6.))
                            .when(self.item_is_selected(ix) && !dragging, |d| {
                                d.bg(t.c("selection.bg"))
                            })
                            .when(dragging, |d| d.opacity(0.45))
                            .hover(|s| s.bg(t.c("state.hover")))
                            .cursor_pointer()
                            .child(
                                div()
                                    .h(px(well))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .overflow_hidden()
                                    .child(picture),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .h(px(18.))
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .on_drag(self.file_drag(ix, cx), |drag, _, _, cx| {
                                drag.resolve(cx);
                                cx.new(|_| drag.clone())
                            })
                            .external_drag_payload(|drag: &FileDrag, _, cx| {
                                drag.external_payload(cx)
                            })
                            .when(entry.kind == EntryKind::Dir, |d| {
                                let destination = entry.path.clone();
                                let external = destination.clone();
                                d.on_drop(cx.listener(move |this, drag: &FileDrag, w, cx| {
                                    cx.stop_propagation();
                                    let kind = if w.modifiers().shift {
                                        crate::file_operations::TransferKind::Move
                                    } else {
                                        crate::file_operations::TransferKind::Copy
                                    };
                                    this.receive_files(
                                        drag.paths(cx),
                                        destination.clone(),
                                        kind,
                                        w,
                                        cx,
                                    );
                                }))
                                .on_drop(cx.listener(
                                    move |this, drag: &gpui::ExternalPaths, w, cx| {
                                        cx.stop_propagation();
                                        this.receive_files(
                                            drag.paths().to_vec(),
                                            external.clone(),
                                            crate::file_operations::TransferKind::Copy,
                                            w,
                                            cx,
                                        );
                                    },
                                ))
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, w, cx| {
                                    this.pointer_select(ix, event, w, cx)
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event, w, cx| {
                                    this.open_context_menu(Some(ix), event, w, cx)
                                }),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                                this.finish_pointer_click(ix, event, cx);
                                if event.click_count() == 2 {
                                    this.open_entry(ix, w, cx);
                                }
                            })),
                    );
                }
                // Empty cells retain a stable grid width on the final row.
                let count = self.gallery_rows[row].1.len();
                for _ in count..self.gallery_columns {
                    tiles = tiles.child(div().flex_1().min_w_0());
                }
                Some(
                    div()
                        .h(px(if self.compact { 156. } else { 172. }))
                        .px(px(10.))
                        .w_full()
                        .child(
                            div()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .text_size(px(11.))
                                .text_color(t.c("text.muted"))
                                .child(label.map(|g| g.label()).unwrap_or("")),
                        )
                        .child(tiles)
                        .into_any_element(),
                )
            })
            .collect()
    }
    pub(super) fn render_gallery(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(
                self.selection_surface(
                    uniform_list(
                        "gallery",
                        self.gallery_rows.len(),
                        cx.processor(Self::render_gallery_rows),
                    )
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&self.scroll),
                    t,
                    cx,
                ),
            )
            .child(self.render_scrollbar(t, cx))
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
            .into_any_element()
    }
}
