use super::*;

impl Explorer {
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        window.focus(&self.tabs[self.active].focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn render_settings(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.preferences.read(cx);
        let value = p.value.clone();
        let error = p.error.clone();
        let refresh_description =
            refresh_description(self.display_interval, value.high_refresh_rate);
        let rows = [
            (
                "テーマ",
                "システム設定に追従、ライト、ダークを切り替えます",
                false,
                "",
            ),
            (
                "コンパクト表示",
                "ファイル一覧とギャラリーの表示密度",
                value.compact,
                if value.compact {
                    "コンパクト"
                } else {
                    "標準"
                },
            ),
            (
                "ダウンロードの日付グループ",
                "ダウンロードを新しい順に日付でまとめます",
                value.group_downloads,
                if value.group_downloads {
                    "有効"
                } else {
                    "無効"
                },
            ),
            (
                "画像フォルダーの自動ギャラリー",
                "ファイルの80%以上が画像のときに切り替えます",
                value.auto_gallery,
                if value.auto_gallery {
                    "有効"
                } else {
                    "無効"
                },
            ),
            (
                "フォルダーサイズの自動計算",
                "手動は必要なときだけ計算。ネットワークは常に手動です",
                value.automatic_sizes,
                if value.automatic_sizes {
                    "自動"
                } else {
                    "手動"
                },
            ),
            (
                "高リフレッシュレート対応",
                refresh_description.as_str(),
                value.high_refresh_rate,
                if value.high_refresh_rate {
                    "有効"
                } else {
                    "最大60fps"
                },
            ),
            (
                "滑らかなスクロール",
                if crate::motion_preferences::reduce_motion_status(cx)
                    == crate::motion_preferences::ReduceMotionStatus::Unknown
                {
                    "一覧とギャラリーに適用（OSの動作低減設定は取得できません）"
                } else {
                    "一覧とギャラリーのホイール操作を滑らかにします"
                },
                value.smooth_scroll,
                if !value.smooth_scroll {
                    "無効"
                } else if cx.reduce_motion() {
                    "OS設定により停止中"
                } else {
                    "有効"
                },
            ),
        ];
        let mut body = div()
            .id("settings-body")
            .debug_selector(|| "settings-body".into())
            .min_h_0()
            .overflow_y_scroll()
            .px(px(20.))
            .py(px(14.))
            .when_some(error, |d, error| {
                d.child(
                    div()
                        .mb(px(14.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(t.c("danger.fg").opacity(0.35))
                        .bg(t.c("danger.fg").opacity(0.12))
                        .text_color(t.c("danger.fg"))
                        .child(error),
                )
            });
        for (title, range) in [
            ("外観と表示", 0..2),
            ("フォルダーと一覧の動作", 2..5),
            ("描画とスクロール", 5..7),
        ] {
            let mut group = div()
                .rounded(px(10.))
                .overflow_hidden()
                .border_1()
                .border_color(t.c("border.subtle"))
                .bg(t.c("surface.raised"));
            for i in range.clone() {
                let (label, description, enabled, state) = rows[i];
                let control = if i == 0 {
                    div()
                        .flex()
                        .items_center()
                        .p(px(2.))
                        .gap(px(2.))
                        .rounded(px(8.))
                        .bg(t.c("surface.field"))
                        .border_1()
                        .border_color(t.c("border.subtle"))
                        .children(
                            [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark]
                                .into_iter()
                                .enumerate()
                                .map(|(index, mode)| {
                                    let selected = ThemeMode::from(value.theme) == mode;
                                    div()
                                        .id(("settings-theme", index))
                                        .debug_selector(|| match index {
                                            0 => "settings-theme-system".into(),
                                            1 => "settings-theme-light".into(),
                                            _ => "settings-theme-dark".into(),
                                        })
                                        .h(px(26.))
                                        .px(px(10.))
                                        .rounded(px(6.))
                                        .flex()
                                        .items_center()
                                        .text_size(px(11.5))
                                        .cursor_pointer()
                                        .border_1()
                                        .border_color(gpui::transparent_black())
                                        .text_color(t.c("text.secondary"))
                                        .when(selected, |d| {
                                            d.bg(t.c("surface.chip-selected"))
                                                .border_color(t.c("tab.active.border"))
                                                .text_color(t.c("text.primary"))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                        })
                                        .hover(|s| s.bg(t.c("state.hover")))
                                        .child(mode.label())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.preferences.update(cx, |p, cx| {
                                                p.change(|s| s.theme = mode.into(), cx)
                                            });
                                        }))
                                }),
                        )
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(t.c(if enabled {
                                    "accent.fg"
                                } else {
                                    "text.secondary"
                                }))
                                .child(state),
                        )
                        .child(
                            div()
                                .w(px(36.))
                                .h(px(20.))
                                .p(px(2.))
                                .rounded(px(10.))
                                .flex()
                                .items_center()
                                .when(enabled, |d| d.justify_end().bg(t.c("button.primary.bg")))
                                .when(!enabled, |d| {
                                    d.justify_start()
                                        .bg(t.c("surface.field"))
                                        .border_1()
                                        .border_color(t.c("border.default"))
                                })
                                .child(div().size(px(14.)).rounded(px(7.)).bg(t.c(if enabled {
                                    "text.on-accent"
                                } else {
                                    "text.secondary"
                                }))),
                        )
                        .into_any_element()
                };
                group = group.child(
                    div()
                        .id(("preference", i))
                        .debug_selector(|| match i {
                            0 => "settings-row-theme".into(),
                            1 => "settings-row-density".into(),
                            2 => "settings-row-downloads".into(),
                            3 => "settings-row-gallery".into(),
                            4 => "settings-row-sizes".into(),
                            5 => "settings-row-high-refresh".into(),
                            _ => "settings-row-smooth-scroll".into(),
                        })
                        .px(px(14.))
                        .py(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .cursor_pointer()
                        .when(i != range.start, |d| {
                            d.border_t_1().border_color(t.c("border.subtle"))
                        })
                        .hover(|s| s.bg(t.c("state.hover")))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.))
                                        .child(
                                            div()
                                                .size(px(18.))
                                                .flex_shrink_0()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .bg(t.c("surface.field"))
                                                .text_size(px(10.5))
                                                .text_color(t.c("text.muted"))
                                                .child((i + 1).to_string()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(label),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl(px(25.))
                                        .mt(px(4.))
                                        .text_size(px(11.5))
                                        .text_color(t.c("text.muted"))
                                        .child(description.to_owned()),
                                ),
                        )
                        .child(div().flex_shrink_0().child(control))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.change_preference(i, cx);
                        })),
                );
            }
            body = body.child(
                div()
                    .when(range.start > 0, |d| d.mt(px(14.)))
                    .child(
                        div()
                            .mb(px(6.))
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.c("text.muted"))
                            .child(title),
                    )
                    .child(group),
            );
        }
        div().id("settings-backdrop").absolute().inset_0().occlude()
            .flex().items_center().justify_center().px(px(16.)).py(px(24.))
            .bg(t.c("surface.backdrop"))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, w, cx| { cx.stop_propagation(); this.close_settings(w, cx); }))
            .child(div().id("settings-panel").debug_selector(|| "settings-panel".into()).track_focus(&self.settings_focus)
                .w(px(520.)).max_w_full().max_h_full().flex().flex_col().overflow_hidden()
                .rounded(px(12.)).border_1().border_color(t.c("border.dialog")).bg(t.c("surface.dialog")).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                    if event.keystroke.modifiers.modified() { return; }
                    if event.keystroke.key == "escape" {
                        this.close_settings(w, cx); cx.stop_propagation();
                    } else if let Ok(number) = event.keystroke.key.parse::<usize>() && (1..=7).contains(&number) {
                        this.change_preference(number - 1, cx); cx.stop_propagation();
                    }
                }))
                .child(div().flex_shrink_0().px(px(20.)).py(px(14.)).bg(t.c("surface.chrome"))
                    .border_b_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().flex_1().min_w_0()
                        .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).child("設定"))
                        .child(div().mt(px(4.)).text_size(px(11.5)).text_color(t.c("text.muted")).child("変更はすべてのウィンドウに反映され、自動保存されます。")))
                    .child(icon_button("close-settings", "icons/x.svg", true, t).on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))))
                .child(body)
                .child(div().flex_shrink_0().px(px(20.)).py(px(10.)).bg(t.c("surface.chrome"))
                    .border_t_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().text_size(px(11.)).text_color(t.c("text.muted")).child("1〜7 で切替 · Esc で閉じる"))
                    .child(div().id("settings-done").debug_selector(|| "settings-done".into()).h(px(28.)).px(px(14.)).rounded(px(6.)).flex().items_center()
                        .border_1().border_color(t.c("button.border")).bg(t.c("button.bg")).hover(|s| s.bg(t.c("state.hover")))
                        .cursor_pointer().child("完了").on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx))))))
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
                    5 => s.high_refresh_rate = !s.high_refresh_rate,
                    6 => s.smooth_scroll = !s.smooth_scroll,
                    _ => (),
                },
                cx,
            )
        });
    }
}

fn refresh_description(interval: Option<std::time::Duration>, enabled: bool) -> String {
    let display = interval
        .filter(|interval| !interval.is_zero())
        .map(|interval| format!("画面: 約{:.0}Hz", 1.0 / interval.as_secs_f64()))
        .unwrap_or_else(|| "画面: 取得できません".into());
    format!(
        "{display} / {}",
        if enabled {
            "OSの描画タイミングに追従"
        } else {
            "描画上限: 60fps"
        }
    )
}
