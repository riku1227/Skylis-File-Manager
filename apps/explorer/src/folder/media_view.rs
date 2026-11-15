use super::*;
use crate::media::{MediaCache, PreviewContent};

#[derive(Clone)]
pub(super) struct SharedMedia(pub Entity<MediaCache>);
impl gpui::Global for SharedMedia {}

impl FolderView {
    pub(super) fn toggle_preview(&mut self, _: &QuickPreview, w: &mut Window, cx: &mut Context<Self>) {
        if self.input_focused(w,cx) { return; }
        #[cfg(target_os="macos")]
        if crate::media::quicklook::is_visible(w).unwrap_or(false) {
            let _=crate::media::quicklook::close(w);self.close_preview(cx);return;
        }
        #[cfg(not(target_os="macos"))]
        if self.preview_open { self.close_preview(cx); return; }
        if self.selected_entry().is_none() { return; }
        #[cfg(target_os="macos")]
        if let Some(entry)=self.selected_entry() {
            if let Err(error)=crate::media::quicklook::show(&entry.path,w) {self.notice=Some(error.to_string());cx.notify();return;}
        }
        self.preview_scope.cancel();
        self.preview_open = true;
        cx.notify();
    }
    pub(super) fn close_preview(&mut self,cx:&mut Context<Self>) {
        self.preview_open = false; self.preview_scope.cancel(); cx.notify();
    }
    pub(super) fn render_preview(&self,t:Theme,cx:&mut Context<Self>)->AnyElement {
        let Some(entry) = self.selected_entry() else { return div().into_any_element(); };
        let content = self.media.update(cx,|media,cx| media.get_preview(entry,&self.preview_scope,cx));
        let content = match content {
            PreviewContent::Loading => div().text_color(t.c("text.muted")).child("プレビューを読み込み中…").into_any_element(),
            PreviewContent::Image(image) => div().relative().size_full().overflow_hidden().child(img(image).absolute().inset_0().size_full().object_fit(gpui::ObjectFit::Contain)).into_any_element(),
            PreviewContent::Text {text,truncated} => div().id("preview-text").size_full().overflow_y_scroll().font_family("Consolas").text_size(px(13.))
                .child(text.to_string()).when(truncated,|d|d.child(div().mt(px(16.)).text_color(t.c("text.muted")).child("先頭128 KiBのみ表示しています"))).into_any_element(),
            PreviewContent::Metadata {reason} => div().flex().flex_col().gap(px(12.))
                .child(entry.kind_label()).child(entry.size.map(format::size).unwrap_or_default())
                .child(div().text_color(t.c("text.muted")).child(reason.to_string())).into_any_element(),
        };
        div().absolute().inset_0().occlude().p(px(28.)).bg(t.c("surface.canvas").opacity(0.94))
            .child(div().size_full().flex().flex_col().rounded(px(8.)).border_1().border_color(t.c("border.subtle")).bg(t.c("surface.raised"))
                .child(div().h(px(44.)).flex_shrink_0().px(px(16.)).flex().items_center().gap(px(12.))
                    .child(div().flex_1().truncate().child(entry.name.clone()))
                    .child(div().id("preview-open").cursor_pointer().text_color(t.c("accent.fg")).child("既定のアプリで開く")
                        .on_click(cx.listener(|this,_,w,cx| {this.close_preview(cx);this.open_selected(&OpenSelected,w,cx);}))))
                .child(div().flex_1().min_h_0().overflow_hidden().p(px(16.)).child(content))
                .child(div().h(px(32.)).flex_shrink_0().px(px(16.)).flex().items_center().justify_between().text_color(t.c("text.muted"))
                    .child("↑ ↓ 選択を移動  ·  Space / Esc 閉じる")
                    .child(div().id("preview-close").cursor_pointer().child("閉じる").on_click(cx.listener(|this,_,_,cx|this.close_preview(cx))))))
            .into_any_element()
    }

}
