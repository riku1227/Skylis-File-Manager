use super::*;
use crate::motion;
use gpui::{Animation, AnimationExt};

use crate::settings_data::{ActivityDisplay, FileTransferEngine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsCategory {
    Appearance,
    Folders,
    Motion,
    Developer,
    WindowsIntegration,
    MacPermissions,
    FileOperations,
    Startup,
}

fn categories() -> Vec<(
    SettingsCategory,
    &'static str,
    &'static str,
    std::ops::Range<usize>,
)> {
    use SettingsCategory::*;
    let mut pages = vec![
        (Appearance, "外観と表示", "icons/desktop.svg", 0..2),
        (
            Folders,
            "フォルダーと一覧",
            "icons/folder-outline.svg",
            2..5,
        ),
        (Motion, "描画とスクロール", "icons/activity.svg", 5..7),
        (Developer, "開発者向け", "icons/code.svg", 7..8),
    ];
    if cfg!(windows) {
        pages.push((WindowsIntegration, "Windows連携", "icons/link.svg", 8..8));
    }
    if cfg!(target_os = "macos") {
        pages.push((MacPermissions, "macOSアクセス権限", "icons/link.svg", 8..8));
    }
    pages.push((FileOperations, "ファイル操作", "icons/activity.svg", 8..8));
    pages.push((Startup, "起動", "icons/desktop.svg", 8..9));
    pages
}

impl SettingsCategory {
    pub(super) fn index(self) -> usize {
        categories()
            .iter()
            .position(|(category, ..)| *category == self)
            .expect("settings category available on this platform")
    }
}

impl Explorer {
    pub(super) fn select_settings_category(&mut self, category: usize) {
        if self.settings_category != category {
            self.settings_category = category;
            self.settings_content_generation = self.settings_content_generation.wrapping_add(1);
        }
    }
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.tabs[self.active].update(cx, |tab, _| tab.activate_name_search());
        window.focus(&self.tabs[self.active].focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (progress, live) = self
            .settings_presence
            .shown()
            .map_or((1., true), |shown| (shown.progress, shown.live));
        let narrow = window.viewport_size().width < px(560.);
        let session_error = crate::session::error(cx);
        let p = self.preferences.read(cx);
        let value = p.value.clone();
        let error = match (p.error.clone(), session_error) {
            (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
            (a, b) => a.or(b),
        };
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

        let mut rows = rows.to_vec();
        rows.push((
            "前回のタブを復元",
            "起動時に前回のウィンドウとタブを開きます。サイズと位置は常に記憶します",
            value.restore_tabs,
            if value.restore_tabs {
                "有効"
            } else {
                "無効"
            },
        ));
        let categories = categories();
        let category_count = categories.len();
        let setting_count = rows.len();
        let numbered_page = !categories[self.settings_category].3.is_empty();
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
        for (index, (_, title, icon, _)) in categories.iter().enumerate() {
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
                        this.select_settings_category(index);
                        this.settings_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );
        }
        let (category, title, _, range) = &categories[self.settings_category];
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
        if *category == SettingsCategory::FileOperations {
            body = body
                .child(div().mb(px(8.)).child("ファイル転送エンジン"))
                .child(div().flex().flex_wrap().gap(px(6.)).children([
                    (FileTransferEngine::OsIntegrated, "標準連携"),
                    (FileTransferEngine::OsDialog, "OS標準ダイアログ（退避用）"),
                ].into_iter().enumerate().map(|(index, (engine, label))| {
                    setting_choice("file-transfer-engine", index, label, engine == value.file_transfer_engine, t)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preferences.update(cx, |p, cx| p.change(|s| s.file_transfer_engine = engine, cx));
                        }))
                })))
                .child(div().mt(px(8.)).text_color(t.c("text.muted"))
                    .child({
                        #[cfg(windows)]
                        {
                            "標準連携では、通常のコピー・移動・削除の進捗と中止をアプリ内で操作できます（競合・権限昇格・エラー時は Windows のダイアログを表示します）。OS標準ダイアログを選ぶと、従来の Windows 進捗ウィンドウで操作します。"
                        }
                        #[cfg(target_os = "macos")]
                        {
                            "macOSでは、ゴミ箱への移動をアプリ内で進捗表示・中止できます。コピーと移動はFinderに委譲します。"
                        }
                        #[cfg(not(any(windows, target_os = "macos")))]
                        {
                            "このOSではネイティブファイル転送・削除は未対応です。"
                        }
                    }))
                .child(div().mt(px(4.)).text_color(t.c("text.muted"))
                    .child("独自転送エンジンは今後の段階実装で追加予定です。"))
                .child(div().mt(px(24.)).mb(px(8.)).child("アクティビティ表示"))
                .child(div().flex().flex_wrap().gap(px(6.)).children([
                    (ActivityDisplay::StatusBar, "アプリ内（ステータスバー）"),
                    (ActivityDisplay::Window, "別ウィンドウ（約500ms以上で自動表示）"),
                ].into_iter().enumerate().map(|(index, (display, label))| {
                    setting_choice("activity-display", index, label, display == value.activity_display, t)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preferences.update(cx, |p, cx| p.change(|s| s.activity_display = display, cx));
                        }))
                })));
        }
        if *category == SettingsCategory::WindowsIntegration {
            use crate::folder_integration::Integration;
            body = body.child(div().text_color(t.c("text.secondary")).child("現在のユーザーに適用します。それぞれ個別に元へ戻せます。登録後はSkylisの実行ファイルを移動しないでください。"))
                .when_some(integration_error, |d, error| d.child(div().mt(px(12.)).text_color(t.c("danger.fg")).child(error)));
            for (index, kind, title, description) in [
                (
                    0usize,
                    Integration::Folders,
                    "フォルダーをSkylisで開く",
                    "ほかのアプリから通常のフォルダーを開く要求に対応します。explorer.exeを直接指定するアプリや特殊な場所は対象外です。",
                ),
                (
                    1,
                    Integration::WinE,
                    "Win+EでSkylisを開く",
                    "WindowsキーとEでSkylisのホームを開きます。フォルダーの関連付けとは独立した設定です。",
                ),
                (
                    2,
                    Integration::Switch,
                    "Explorerに切り替えメニューを追加",
                    "フォルダー背景の右クリック（Windows 11では「その他のオプション」）に「Skylisへ切り替え」を追加します。読み込み成功後、呼び出し元が単一タブと確認できたときだけ元のウィンドウを閉じます。確認できない場合は残します。",
                ),
            ] {
                let status = integration.as_ref().map(|s| s.get(kind));
                let enabled = status.is_some_and(|s| s.enabled);
                let mut buttons = div().flex().flex_wrap().gap(px(8.)).mt(px(10.));
                for (button, label, install, available) in [
                    (0usize, "有効にする", true, !enabled),
                    (
                        1,
                        "元の設定に戻す",
                        false,
                        status.is_some_and(|s| s.can_restore),
                    ),
                ] {
                    buttons = buttons.child(
                        div()
                            .id(("shell-integration", index * 2 + button))
                            .flex_shrink_0()
                            .px(px(12.))
                            .py(px(7.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(t.c(if install {
                                "button.primary.border"
                            } else {
                                "button.border"
                            }))
                            .bg(t.c(if install {
                                "button.primary.bg"
                            } else {
                                "button.bg"
                            }))
                            .text_color(t.c(if install {
                                "text.on-accent"
                            } else {
                                "button.fg"
                            }))
                            .when(!integration_busy && available, |d| {
                                d.cursor_pointer().hover(|s| {
                                    s.bg(t.c(if install {
                                        "button.primary.hover.bg"
                                    } else {
                                        "button.hover.bg"
                                    }))
                                })
                            })
                            .opacity(if integration_busy || !available {
                                0.45
                            } else {
                                1.
                            })
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !integration_busy && available {
                                    this.preferences.update(cx, |p, cx| {
                                        p.set_folder_integration(kind, install, cx)
                                    });
                                }
                            })),
                    );
                }
                body = body.child(
                    div()
                        .mt(px(12.))
                        .p(px(12.))
                        .rounded(px(8.))
                        .bg(t.c("surface.card"))
                        .border_1()
                        .border_color(t.c("border.card"))
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .mt(px(5.))
                                                .text_size(px(11.5))
                                                .text_color(t.c("text.secondary"))
                                                .child(description),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .px(px(7.))
                                        .py(px(3.))
                                        .rounded(px(10.))
                                        .text_size(px(12.))
                                        .bg(t.c(if enabled {
                                            "accent.soft"
                                        } else {
                                            "surface.field"
                                        }))
                                        .text_color(t.c(if enabled {
                                            "accent.fg"
                                        } else {
                                            "text.secondary"
                                        }))
                                        .when(enabled, |d| {
                                            d.child(
                                                div()
                                                    .size(px(5.))
                                                    .rounded_full()
                                                    .bg(t.c("accent.fg")),
                                            )
                                        })
                                        .child(if integration_busy {
                                            "確認・変更中…"
                                        } else if enabled {
                                            "有効"
                                        } else {
                                            "無効"
                                        }),
                                ),
                        )
                        .child(buttons),
                );
            }
        }
        if *category == SettingsCategory::MacPermissions {
            use crate::macos_permissions::{Action, Observation};
            let observation = self.tabs[self.active]
                .read(cx)
                .permission_observation
                .observation;
            let status = match observation {
                Observation::Unknown => {
                    "未確認：フルディスクアクセスの許可状態は自動判定できません。"
                }
                Observation::Denied => {
                    "このフォルダーへのアクセスが拒否されました。下の手順でアクセス権限を確認できます。"
                }
                Observation::Accessible => {
                    "このフォルダーは開けます。フルディスクアクセスの許可状態は未確認です。"
                }
                Observation::OtherFailure => {
                    "このフォルダーで別のエラーが発生しました。フルディスクアクセスの許可状態は未確認です。"
                }
            };
            body = body.child(div().id("permission-status").debug_selector(|| "permission-status".into()).mb(px(16.)).child(status))
                .child(div().mb(px(16.)).child("「ファイルとフォルダ」は書類・デスクトップなどの場所ごとの許可です。「フルディスクアクセス」は保護されたデータにもアクセスできる広い権限なので、必要な場合だけ許可してください。フォルダーの所有者や読み取り権限が原因の場合は、この設定だけでは開けません。"))
                .child(div().mb(px(16.)).child("システム設定 → プライバシーとセキュリティ → フルディスクアクセスで、＋からSkylis.appを追加し、スイッチをオンにします。下のFinderボタンで、いま実行中のアプリを確認して追加してください。debug版とrelease版は保存場所が異なります。変更後はSkylisを終了して再起動し、対象フォルダーを開いてください。F5でそのフォルダーを再読み込みできます。"))
                .child(div().mb(px(16.)).child("署名方式を変更した後は、一度だけ現在のSkylis.appを追加し直す必要がある場合があります。証明書による安定した署名を使わない開発版では、更新後にも再追加が必要になることがあります。macOSの確認がすべて不要になるわけではありません。"));
            for (index, label, action) in [
                (0, "システム設定を開く", Action::OpenPrivacy),
                (1, "実行中の.appをFinderに表示", Action::RevealBundle),
            ] {
                body = body.child(
                    div()
                        .id(("macos-permission-action", index as usize))
                        .p(px(12.))
                        .mb(px(8.))
                        .rounded(px(6.))
                        .bg(t.c("button.bg"))
                        .border_1()
                        .border_color(t.c("button.border"))
                        .hover(|style| style.bg(t.c("button.hover.bg")))
                        .opacity(if p.permission_busy { 0.45 } else { 1. })
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preferences
                                .update(cx, |p, cx| p.permission_action(action, cx));
                        })),
                );
            }
            if let Some(message) = &p.permission_message {
                body = body.child(div().mt(px(8.)).child(message.clone()));
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
                                .child(if i == 0 { "テーマ" } else { "表示密度" }),
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
                                7 => "settings-row-performance",
                                _ => "settings-row-restore-tabs",
                            }
                            .into()
                        })
                        .py(px(14.))
                        .px(px(10.))
                        .rounded(px(6.))
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
                                        .child(label),
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
                        .child(motion::switch(
                            ("settings-switch", i),
                            enabled,
                            34.,
                            t.c("surface.field"),
                            t.c("button.primary.bg"),
                            t.c("text.secondary"),
                            t.c("text.on-accent"),
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.change_preference(i, cx);
                        })),
                );
            }
        }
        div().id("settings-backdrop").absolute().inset_0().occlude()
            .flex().items_center().justify_center().px(px(16.)).py(px(24.))
            .child(div().absolute().inset_0().bg(t.c("surface.backdrop")).opacity(progress))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, w, cx| { cx.stop_propagation(); this.close_settings(w, cx); }))
            .child(div().id("settings-panel").debug_selector(|| "settings-panel".into())
                .w(px(880.)).h(px(620.)).max_w_full().max_h_full()
                .child(div().id("settings-panel-content").when(live, |d| d.track_focus(&self.settings_focus)).size_full().flex().flex_col().overflow_hidden()
                .rounded(px(12.)).border_1().border_color(t.c("border.dialog")).bg(t.c("surface.dialog")).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, w, cx| {
                    if event.keystroke.modifiers.modified() { return; }
                    if event.keystroke.key == "escape" {
                        this.close_settings(w, cx); cx.stop_propagation();
                    } else if matches!(event.keystroke.key.as_str(), "up" | "down") {
                        this.select_settings_category((this.settings_category + if event.keystroke.key == "down" { 1 } else { category_count - 1 }) % category_count);
                        this.settings_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify(); cx.stop_propagation();
                    } else if let Ok(number) = event.keystroke.key.parse::<usize>() && (1..=setting_count).contains(&number) {
                        this.change_preference(number - 1, cx); cx.stop_propagation();
                    }
                }))
                .child(div().flex_shrink_0().px(px(20.)).py(px(14.)).bg(t.c("surface.chrome"))
                    .border_b_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().flex_1().min_w_0()
                        .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).child("設定"))
                        .child(div().mt(px(4.)).text_size(px(11.5)).text_color(t.c("text.muted")).child("変更はすべてのウィンドウに反映され、自動保存されます。")))
                    .child(icon_button("close-settings", "icons/x.svg", true, t).on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))))
                .child(div().flex_1().min_h_0().flex().overflow_hidden().child(nav).child(body.with_animation(("settings-content", self.settings_content_generation), Animation::new(motion::CATEGORY_REVEAL), |d, p| d.opacity(motion::ease_out_cubic(p)))))
                .child(div().flex_shrink_0().px(px(20.)).py(px(10.)).bg(t.c("surface.chrome"))
                    .border_t_1().border_color(t.c("border.subtle")).flex().items_center().justify_between()
                    .child(div().text_size(px(11.)).text_color(t.c("text.muted")).child(if numbered_page { format!("↑↓ カテゴリ · 1〜{setting_count} 設定を切替 · Esc 閉じる") } else { "↑↓ カテゴリ · Esc 閉じる".into() }))
                    .child(div().flex_shrink_0().id("settings-done").debug_selector(|| "settings-done".into()).h(px(28.)).px(px(14.)).rounded(px(6.)).flex().items_center()
                        .border_1().border_color(t.c("button.border")).bg(t.c("button.bg")).hover(|s| s.bg(t.c("state.hover")))
                        .cursor_pointer().child("完了").on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))))
                .opacity(progress).relative().top(px(-motion::MODAL_OFFSET * (1. - progress)))))
            // A closing modal absorbs pointer input for its last few frames.
            .when(!live, |d| d.child(motion::ghost_shield()))
    }

    fn change_preference(&mut self, index: usize, cx: &mut Context<Self>) {
        use SettingsCategory::*;
        let (category, first) = match index {
            0..=1 => (Appearance, 0),
            2..=4 => (Folders, 2),
            5..=6 => (Motion, 5),
            7 => (Developer, 7),
            8 => (Startup, 8),
            _ => return,
        };
        self.select_settings_category(category.index());
        let session_error = crate::session::error(cx).is_some();
        let error_row = usize::from(self.preferences.read(cx).error.is_some() || session_error);
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
                    8 => s.restore_tabs = !s.restore_tabs,
                    _ => (),
                },
                cx,
            )
        });
    }
}

fn setting_choice(
    id: &'static str,
    index: usize,
    label: &'static str,
    selected: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id((id, index))
        .debug_selector(move || format!("{id}-{index}").into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .tab_index(0)
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.c(if selected {
            "accent.fg"
        } else {
            "button.border"
        }))
        .bg(t.c(if selected {
            "surface.chip-selected"
        } else {
            "button.bg"
        }))
        .focus(|d| d.bg(t.c("surface.chip-selected")))
        .cursor_pointer()
        .hover(|d| d.bg(t.c("state.hover")))
        .child(label)
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
