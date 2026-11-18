use super::*;
use crate::media::MediaCache;
use preview_window::{ImageSequence, PreviewWindow};

#[derive(Clone)]
pub(super) struct SharedMedia(pub Entity<MediaCache>);
impl gpui::Global for SharedMedia {}

pub(super) fn is_gallery_image(kind: EntryKind, path: &Path) -> bool {
    kind == EntryKind::File && crate::media::is_image_path(path)
}

impl FolderView {
    pub(super) fn toggle_preview(
        &mut self,
        _: &QuickPreview,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.descendants.selected.is_some() {
            return;
        }
        if self.is_archive() || self.input_focused(w, cx) {
            return;
        }
        if self.preview_open {
            #[cfg(target_os = "macos")]
            let _ = crate::media::quicklook::close(w);
            self.close_preview(cx);
            return;
        }
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };
        self.cancel_scroll_motion();
        self.preview_scope.cancel();
        #[cfg(target_os = "macos")]
        {
            match crate::media::quicklook::show(&entry.path, w) {
                Ok(()) => self.preview_open = true,
                Err(error) => self.notice = Some(error.to_string()),
            }
            cx.notify();
        }
        #[cfg(not(target_os = "macos"))]
        self.open_preview_window(entry, None, w, cx);
    }

    pub(super) fn open_image_viewer(
        &mut self,
        path: &Path,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Snapshot the displayed order, including any search results already shown.
        // This copies only listing metadata and opens no files on the UI thread.
        let entries = self
            .visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|entry| is_gallery_image(entry.kind, &entry.path))
            .cloned()
            .chain(
                self.descendants
                    .hits
                    .iter()
                    .filter(|hit| self.recursive_search() && is_gallery_image(hit.kind, &hit.path))
                    .map(|hit| Entry::from_search_hit(hit)),
            )
            .collect();
        let Some(sequence) = ImageSequence::new(entries, path) else {
            return;
        };
        #[cfg(target_os = "macos")]
        let _ = crate::media::quicklook::close(w);
        self.close_preview(cx);
        self.cancel_scroll_motion();
        self.open_preview_window(sequence.current().clone(), Some(sequence), w, cx);
    }

    fn open_preview_window(
        &mut self,
        entry: Entry,
        sequence: Option<ImageSequence>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = cx.entity();
        let owner = w.window_handle();
        let media = self.media.clone();
        let scope = self.preview_scope.clone();
        let mode = self.mode;
        let viewer = sequence.is_some();
        let bounds = gpui::Bounds::new(
            w.bounds().origin + gpui::point(px(48.), px(48.)),
            gpui::size(px(800.), px(600.)),
        );
        let native_owner = preview_window::native_owner(w);
        match cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                window_min_size: Some(gpui::size(px(400.), px(300.))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(
                        if viewer {
                            "画像ビューア — Skylis"
                        } else {
                            "クイックプレビュー — Skylis"
                        }
                        .into(),
                    ),
                    appears_transparent: cfg!(windows),
                    ..Default::default()
                }),
                focus: viewer,
                is_minimizable: false,
                ..Default::default()
            },
            move |window, cx| {
                preview_window::attach_native_owner(window, native_owner);
                cx.new(|cx| {
                    PreviewWindow::new(
                        source, owner, entry, sequence, media, scope, mode, window, cx,
                    )
                })
            },
        ) {
            Ok(handle) => {
                self.preview_open = true;
                self.preview_window = Some(handle);
            }
            Err(error) => self.notice = Some(format!("プレビューを開けません: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = false;
        self.preview_scope.cancel();
        if let Some(handle) = self.preview_window.take() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
        cx.notify();
    }

    pub(super) fn inspector_picture(
        &self,
        entry: &Entry,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.is_archive() {
            return self.object_icon(&entry.path, entry.is_dir_like(), 68., window, cx);
        }
        let thumbnail = self.media.update(cx, |media, cx| {
            media.get_thumbnail(
                entry,
                (232. * window.scale_factor()).ceil() as u32,
                &self.preview_scope,
                cx,
            )
        });
        let mut displayed = self.inspector_image.borrow_mut();
        if displayed.as_ref().is_some_and(|(old, _)| {
            old.path != entry.path || old.modified != entry.modified || old.size != entry.size
        }) {
            *displayed = None;
        }
        if let Some(image) = thumbnail {
            let keep_existing = displayed.as_ref().is_some_and(|(_, old)| {
                old.as_bytes(0).map_or(0, <[u8]>::len) > image.as_bytes(0).map_or(0, <[u8]>::len)
            });
            if !keep_existing {
                *displayed = Some((entry.clone(), image));
            }
        }
        if let Some((_, image)) = displayed.as_ref() {
            img(image.clone())
                .size_full()
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element()
        } else {
            self.object_icon(&entry.path, entry.kind == EntryKind::Dir, 68., window, cx)
        }
    }

    pub(super) fn set_gallery(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.is_archive() {
            return;
        }
        self.gallery = enabled;
        self.gallery_override = Some(enabled);
        self.media_scope.cancel();
        self.rebuild_display_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }
    pub(super) fn choose_auto_gallery(&mut self, cx: &mut Context<Self>) {
        if self.is_archive() {
            self.gallery = false;
            return;
        }
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
        let query_key = self.query.to_lowercase();
        let name_matcher = explorer_core::entry::NameMatcher::new(&query_key);
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
                            .child(self.render_inline_name(&entry.path, t, cx).unwrap_or_else(
                                || {
                                    div()
                                        .mt(px(4.))
                                        .h(px(18.))
                                        .truncate()
                                        .child(search_presentation::name_text(
                                            &entry.name,
                                            name_matcher.ranges(&entry),
                                            t,
                                        ))
                                        .into_any_element()
                                },
                            ))
                            .on_drag(self.file_drag(ix, cx), |drag, _, _, cx| {
                                drag.resolve(cx);
                                cx.new(|_| drag.clone())
                            })
                            .external_drag_payload(|drag: &FileDrag, _, cx| {
                                drag.external_payload(cx)
                            })
                            .when(entry.kind == EntryKind::Dir, |d| {
                                Self::file_drop_target(d, Some(entry.path.clone()), t, cx)
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
