use super::*;

impl Explorer {
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        window.focus(&self.tabs[self.active].focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(560.);
        let p = self.preferences.read(cx);
        let value = p.value.clone();
        let error = p.error.clone();
        let integration = p.folder_integration.clone();
        let integration_busy = p.integration_busy;
        let integration_error = p.integration_error.clone();
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
            (
                "パフォーマンス表示",
                "FPSは描画送信回数（表示の更新分を除外）、メモリはアプリ全体の常駐量です",
                value.performance_overlay,
                if value.performance_overlay {
                    "有効"
                } else {
                    "無効"
                },
            ),
        ];

        let mut categories = vec![
            ("外観と表示", "icons/desktop.svg", 0..2),
            ("フォルダーと一覧", "icons/folder-outline.svg", 2..5),
            ("描画とスクロール", "icons/activity.svg", 5..7),
            ("開発者向け", "icons/code.svg", 7..8),
        ];
        if cfg!(windows) {
            categories.push(("Windows連携", "icons/link.svg", 8..8));
        }
        let category_count = categories.len();
        let mut nav = div()
            .id("settings-navigation")
            .w(px(if narrow { 124. } else { 166. }))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .p(px(8.))
            .bg(t.c("surface.chrome"))
            .border_r_1()
            .border_color(t.c("border.subtle"));
        for (index, (title, icon, _)) in categories.iter().enumerate() {
            nav = nav.child(
                div()
                    .id(("settings-category", index))
                    .debug_selector(move || format!("settings-category-{index}").into())
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(12.))
                    .mb(px(4.))
                    .rounded(px(6.))
                    .border_l_2()
                    .border_color(gpui::transparent_black())
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(index == self.settings_category, |d| {
                        d.bg(t.c("surface.chip-selected"))
                            .border_color(t.c("accent.fg"))
                            .text_color(t.c("accent.fg"))
                    })
                    .hover(|s| s.bg(t.c("state.hover")))
                    .child(
                        svg()
                            .path(*icon)
                            .size(px(16.))
                            .flex_shrink_0()
                            .text_color(t.c("text.secondary")),
                    )
                    .child(*title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.settings_category = index;
                        this.settings_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );
        }
        let (title, _, range) = &categories[self.settings_category];
        let mut body = div()
            .id(("settings-body", self.settings_category))
            .track_scroll(&self.settings_scroll)
            .debug_selector(|| "settings-body".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(20.))
            .py(px(18.))
            .child(
                div()
                    .mb(px(20.))
                    .text_size(px(20.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(*title),
            )
            .when_some(error, |d, error| {
                d.child(
                    div()
                        .mb(px(14.))
                        .p(px(10.))
                        .rounded(px(6.))
                        .bg(t.c("danger.fg").opacity(0.12))
                        .text_color(t.c("danger.fg"))
                        .child(error),
                )
            });
        if self.settings_category == 4 {
            let enabled = integration.as_ref().is_some_and(|s| s.enabled);
            body = body.child(div().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).child("フォルダーをSkylisで開く"))
                .child(div().mt(px(10.)).text_color(t.c("text.secondary")).child("ほかのアプリから通常のフォルダーを開いたときに、Skylisを使います。現在のユーザーに適用され、ここから元に戻せます。"))
                .child(div().mt(px(10.)).text_size(px(12.)).text_color(t.c("text.muted")).child("explorer.exeを直接起動するアプリ、Win+E、ゴミ箱などの特殊な場所は対象外です。登録後はSkylisの実行ファイルを移動しないでください。"))
                .child(div().mt(px(16.)).child(if integration_busy { "確認・変更中…" } else if enabled { "Skylisを使用中" } else { "Windowsの設定を使用中" }))
                .when_some(integration_error, |d, error| d.child(div().mt(px(12.)).text_color(t.c("danger.fg")).child(error)));
            for (id, label, install, available) in [
                ("enable-folder-integration", "Skylisを使う", true, !enabled),
                (
                    "restore-folder-integration",
                    "元の設定に戻す",
                    false,
                    integration.as_ref().is_some_and(|s| s.can_restore),
                ),
            ] {
                body = body.child(
                    div()
                        .id(id)
                        .mt(px(12.))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(t.c("button.border"))
                        .bg(t.c("button.bg"))
                        .when(!integration_busy && available, |d| {
                            d.cursor_pointer().hover(|s| s.bg(t.c("state.hover")))
                        })
                        .opacity(if integration_busy || !available {
                            0.45
                        } else {
                            1.
                        })
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !integration_busy && available {
                                this.preferences
                                    .update(cx, |p, cx| p.set_folder_integration(install, cx));
                            }
                        })),
                );
            }
        }
        for i in range.clone() {
            let (label, description, enabled, state) = rows[i];
            if i < 2 {
                let mut choices = div()
                    .flex()
                    .gap(px(8.))
                    .w_full()
                    .when(narrow, |d| d.flex_col());
                let count = if i == 0 { 3 } else { 2 };
                for index in 0..count {
                    let mode = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark][index];
                    let selected = if i == 0 {
                        ThemeMode::from(value.theme) == mode
                    } else {
                        value.compact == (index == 1)
                    };
                    let caption = if i == 0 {
                        mode.label()
                    } else if index == 0 {
                        "標準"
                    } else {
                        "コンパクト"
                    };
                    let preview = if i == 0 {
                        theme_preview(mode, window.appearance()).into_any_element()
                    } else {
                        density_preview(index == 1, t).into_any_element()
                    };
                    choices = choices.child(
                        div()
                            .id(("settings-choice", i * 3 + index))
                            .debug_selector(move || {
                                match (i, index) {
                                    (0, 0) => "settings-theme-system",
                                    (0, 1) => "settings-theme-light",
                                    (0, _) => "settings-theme-dark",
                                    (_, 0) => "settings-density-standard",
                                    _ => "settings-density-compact",
                                }
                                .into()
                            })
                            .flex_1()
                            .min_w_0()
                            .p(px(5.))
                            .rounded(px(7.))
                            .border_2()
                            .border_color(t.c(if selected {
                                "accent.fg"
                            } else {
                                "border.subtle"
                            }))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.c("state.hover")))
                            .child(preview)
                            .child(
                                div()
                                    .mt(px(8.))
                                    .mb(px(3.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .text_size(px(11.))
                                    .child(caption)
                                    .child(
                                        div()
                                            .size(px(10.))
                                            .flex_shrink_0()
                                            .rounded(px(5.))
                                            .border_2()
                                            .border_color(t.c(if selected {
                                                "accent.fg"
                                            } else {
                                                "border.default"
                                            }))
                                            .when(selected, |d| d.bg(t.c("accent.fg"))),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.preferences.update(cx, |p, cx| {
                                    p.change(
                                        |s| {
                                            if i == 0 {
                                                s.theme = mode.into();
                                            } else {
                                                s.compact = index == 1;
                                            }
                                        },
                                        cx,
                                    )
                                });
                            })),
                    );
                }
                body = body.child(
                    div()
                        .id(("settings-section", i))
                        .debug_selector(move || {
                            if i == 0 {
                                "settings-row-theme".into()
                            } else {
                                "settings-row-density".into()
                            }
                        })
                        .mb(px(22.))
                        .child(
                            div()
                                .mb(px(10.))
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(if i == 0 {
                                    "1  テーマ"
                                } else {
                                    "2  表示密度"
                                }),
                        )
                        .child(choices)
                        .child(
                            div()
                                .mt(px(8.))
                                .text_size(px(11.))
                                .text_color(t.c("text.muted"))
                                .child(description.to_owned()),
                        ),
                );
            } else {
                body = body.child(
                    div()
                        .id(("preference", i))
                        .debug_selector(move || {
                            match i {
                                2 => "settings-row-downloads",
                                3 => "settings-row-gallery",
                                4 => "settings-row-sizes",
                                5 => "settings-row-high-refresh",
                                6 => "settings-row-smooth-scroll",
                                _ => "settings-row-performance",
                            }
                            .into()
                        })
                        .py(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .border_b_1()
                        .border_color(t.c("border.subtle"))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.c("state.hover")))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(format!("{}  {label}", i + 1)),
                                )
                                .child(
                                    div()
                                        .mt(px(6.))
                                        .text_size(px(11.5))
                                        .text_color(t.c("text.muted"))
                                        .child(description.to_owned()),
                                )
                                .child(
                                    div()
                                        .mt(px(6.))
                                        .text_size(px(11.))
                                        .text_color(t.c("accent.fg"))
                                        .child(state),
                                ),
                        )
                        .child(
                            div()
                                .w(px(34.))
                                .h(px(20.))
                                .flex_shrink_0()
                                .p(px(3.))
                                .rounded(px(10.))
                                .flex()
                                .items_center()
                                .bg(t.c(if enabled {
                                    "button.primary.bg"
                                } else {
                                    "surface.field"
                                }))
                                .border_1()
                                .border_color(t.c("border.default"))
                                .when(enabled, |d| d.justify_end())
                                .child(div().size(px(12.)).rounded(px(6.)).bg(t.c(if enabled {
                                    "text.on-accent"
                                } else {
                                    "text.secondary"
                                }))),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.change_preference(i, cx);
                        })),
                );
            }
        }
        div().id("settings-backdrop").absolute().inset_0().occlude()
            .flex().items_center().justify_center().px(px(16.)).py(px(24.))
            .bg(t.c("surface.backdrop"))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, w, cx| { cx.stop_propagation(); this.close_settings(w, cx); }))
            .child(div().id("settings-panel").debug_selector(|| "settings-panel".into()).track_focus(&self.settings_focus)
                .w(px(880.)).h(px(620.)).max_w_full().max_h_full().flex().flex_col().overflow_hidden()
                .rounded(px(12.)).border_1().border_color(t.c("border.dialog")).bg(t.c("surface.dialog")).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, w, cx| {
                    if event.keystroke.modifiers.modified() { return; }
                    if event.keystroke.key == "escape" {
                        this.close_settings(w, cx); cx.stop_propagation();
                    } else if matches!(event.keystroke.key.as_str(), "up" | "down") {
                        this.settings_category = (this.settings_category + if event.keystroke.key == "down" { 1 } else { category_count - 1 }) % category_count;
                        this.settings_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify(); cx.stop_propagation();
                    } else if let Ok(number) = event.keystroke.key.parse::<usize>() && (1..=8).contains(&number) {
                        this.change_preference(number - 1, cx); cx.stop_propagation();
                    }
                }))
                .child(div().flex_shrink_0().px(px(20.)).py(px(14.)).bg(t.c("surface.chrome"))
                    .border_b_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().flex_1().min_w_0()
                        .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).child("設定"))
                        .child(div().mt(px(4.)).text_size(px(11.5)).text_color(t.c("text.muted")).child("変更はすべてのウィンドウに反映され、自動保存されます。")))
                    .child(icon_button("close-settings", "icons/x.svg", true, t).on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))))
                .child(div().flex_1().min_h_0().flex().overflow_hidden().child(nav).child(body))
                .child(div().flex_shrink_0().px(px(20.)).py(px(10.)).bg(t.c("surface.chrome"))
                    .border_t_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().text_size(px(11.)).text_color(t.c("text.muted")).child("↑↓ カテゴリ · 1〜8 切替 · Esc 閉じる"))
                    .child(div().flex_shrink_0().id("settings-done").debug_selector(|| "settings-done".into()).h(px(28.)).px(px(14.)).rounded(px(6.)).flex().items_center()
                        .border_1().border_color(t.c("button.border")).bg(t.c("button.bg")).hover(|s| s.bg(t.c("state.hover")))
                        .cursor_pointer().child("完了").on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx))))))
    }

    fn change_preference(&mut self, index: usize, cx: &mut Context<Self>) {
        self.settings_category = match index {
            0..=1 => 0,
            2..=4 => 1,
            5..=6 => 2,
            _ => 3,
        };
        let first = [0, 2, 5, 7][self.settings_category];
        let error_row = usize::from(self.preferences.read(cx).error.is_some());
        self.settings_scroll
            .scroll_to_item(1 + error_row + index - first);
        cx.notify();
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
                    7 => s.performance_overlay = !s.performance_overlay,
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

// Bounded layout geometry only: no image work or filesystem access.
fn theme_preview(mode: ThemeMode, appearance: gpui::WindowAppearance) -> impl IntoElement {
    let palette = Theme::resolve(mode, appearance);
    div()
        .h(px(68.))
        .flex()
        .overflow_hidden()
        .rounded(px(4.))
        .border_1()
        .border_color(palette.c("border.subtle"))
        .child(div().w(px(22.)).h_full().bg(palette.c("surface.chrome")))
        .child(
            div()
                .flex_1()
                .h_full()
                .p(px(8.))
                .bg(palette.c("surface.dialog"))
                .child(
                    div()
                        .h(px(4.))
                        .w_full()
                        .mb(px(9.))
                        .bg(palette.c("accent.fg")),
                )
                .children(
                    (0..3).map(|_| div().h(px(3.)).mb(px(7.)).bg(palette.c("border.default"))),
                ),
        )
}
fn density_preview(compact: bool, t: Theme) -> impl IntoElement {
    div()
        .h(px(104.))
        .p(px(6.))
        .rounded(px(4.))
        .bg(t.c("surface.field"))
        .children(
            ["Documents", "報告書.pdf", "メモ.txt"]
                .into_iter()
                .map(move |label| {
                    div()
                        .h(px(if compact { 23. } else { 30. }))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .border_b_1()
                        .border_color(t.c("border.subtle"))
                        .child(
                            div()
                                .w(px(8.))
                                .h(px(10.))
                                .border_1()
                                .border_color(t.c("text.muted")),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(t.c("text.secondary"))
                                .child(label),
                        )
                }),
        )
}
