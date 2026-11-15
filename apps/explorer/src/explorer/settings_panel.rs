use super::*;

impl Explorer {
    pub(super) fn render_settings(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.preferences.read(cx);
        let value = p.value.clone();
        let error = p.error.clone();
        let rows = [
            (
                "テーマ",
                ThemeMode::from(value.theme).label(),
                "システム設定に追従、ライト、ダークを切り替えます",
            ),
            (
                "表示密度",
                if value.compact {
                    "コンパクト"
                } else {
                    "標準"
                },
                "ファイル一覧とギャラリーの密度",
            ),
            (
                "ダウンロードの日付グループ",
                if value.group_downloads {
                    "有効"
                } else {
                    "無効"
                },
                "ダウンロードを新しい順に日付でまとめます",
            ),
            (
                "画像フォルダーの自動ギャラリー",
                if value.auto_gallery {
                    "有効"
                } else {
                    "無効"
                },
                "ファイルの80%以上が画像のときに切り替えます",
            ),
            (
                "フォルダーサイズ",
                if value.automatic_sizes {
                    "自動"
                } else {
                    "手動"
                },
                "手動は必要なときだけ計算。ネットワークは常に手動です",
            ),
        ];
        div().id("settings-panel").track_focus(&self.settings_focus)
            .size_full().overflow_y_scroll().p(px(24.)).bg(t.c("surface.canvas"))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                if event.keystroke.key == "escape" {
                    this.settings_open = false;
                    w.focus(&this.tabs[this.active].focus_handle(cx), cx);
                    cx.notify();
                } else if let Ok(number) = event.keystroke.key.parse::<usize>() {
                    if (1..=5).contains(&number) { this.change_preference(number - 1, cx); }
                }
                cx.stop_propagation();
            }))
            .child(div().flex().items_center().justify_between().mb(px(12.))
                .child(div().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child("設定"))
                .child(icon_button("close-settings", "icons/x.svg", true, t).on_click(cx.listener(|this, _, w, cx| {
                    this.settings_open = false; w.focus(&this.tabs[this.active].focus_handle(cx), cx); cx.notify();
                }))))
            .child(div().mb(px(20.)).text_color(t.c("text.muted")).child("変更はすべてのウィンドウに反映され、自動保存されます。数字キー1〜5でも切り替えできます。"))
            .children(rows.into_iter().enumerate().map(|(i, (label, state, description))| {
                div().id(("preference", i)).max_w(px(760.)).p(px(14.)).mb(px(8.))
                    .rounded(px(8.)).border_1().border_color(t.c("border.subtle"))
                    .bg(t.c("surface.raised")).cursor_pointer().hover(|s| s.bg(t.c("state.hover")))
                    .child(div().flex().justify_between()
                        .child(format!("{}  {}", i + 1, label))
                        .child(div().text_color(t.c("accent.fg")).child(state)))
                    .child(div().mt(px(6.)).text_color(t.c("text.muted")).child(description))
                    .on_click(cx.listener(move |this, _, _, cx| this.change_preference(i, cx)))
            }))
            .when_some(error, |d, error| d.child(div().mt(px(12.)).text_color(t.c("danger.fg")).child(error)))
    }

    fn change_preference(&mut self, index: usize, cx: &mut Context<Self>) {
        self.preferences.update(cx, |p, cx| {
            p.change(
                |s| match index {
                    0 => s.theme = ThemeMode::from(s.theme).next().into(),
                    1 => s.compact = !s.compact,
                    2 => s.group_downloads = !s.group_downloads,
                    3 => s.auto_gallery = !s.auto_gallery,
                    4 => s.automatic_sizes = !s.automatic_sizes,
                    _ => (),
                },
                cx,
            )
        });
    }
}
