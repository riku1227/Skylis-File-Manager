use super::*;

impl FolderView {
    pub(super) fn home_places(&self) -> Vec<Place> {
        let query = self.query.to_lowercase();
        self.drives
            .iter()
            .chain(self.favorites.iter())
            .filter(|place| {
                place.label.to_lowercase().contains(&query)
                    || place.path.to_string_lossy().to_lowercase().contains(&query)
                    || (place.icon == platform::PlaceIcon::Home
                        && "ユーザーフォルダー".contains(&query))
            })
            .cloned()
            .collect()
    }

    pub(super) fn render_home(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let places = self.home_places();
        let focused = self.focus.is_focused(window);
        let available = window.viewport_size().width - SIDEBAR_WIDTH - px(64.);
        let columns = if available >= px(870.) { 3. } else { 2. };
        let width = (available - px(12.) * (columns - 1.)) / columns;
        let mut page = div()
            .id("home-page")
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(24.));
        for (title, drives) in [("ドライブ", true), ("フォルダー", false)] {
            let mut cards = div().flex().flex_wrap().gap(px(12.));
            let mut count = 0;
            for (index, place) in places.iter().enumerate() {
                let drive = place.icon == platform::PlaceIcon::Drive;
                if drive != drives {
                    continue;
                }
                count += 1;
                let selected = self.selected == Some(index);
                let mut info = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .truncate()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(if place.icon == platform::PlaceIcon::Home {
                                "ユーザーフォルダー".into()
                            } else if drive {
                                format!("ドライブ ({})", place.label)
                            } else {
                                place.label.clone()
                            }),
                    );
                if drive {
                    info = match self.drive_space.get(&place.path) {
                        Some(Some(space)) => {
                            let used = if space.total > 0 {
                                (1. - space.available as f32 / space.total as f32).clamp(0., 1.)
                            } else {
                                0.
                            };
                            info.child(
                                div()
                                    .h(px(5.))
                                    .rounded(px(3.))
                                    .overflow_hidden()
                                    .bg(t.c("meter.track"))
                                    .child(div().h_full().w(gpui::relative(used)).bg(t.c(
                                        if used > 0.9 {
                                            "meter.warn"
                                        } else {
                                            "meter.fill"
                                        },
                                    ))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.c("text.secondary"))
                                    .child(format!(
                                        "空き {} / {}",
                                        format::size(space.available),
                                        format::size(space.total)
                                    )),
                            )
                        }
                        value => info.child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.c("text.muted"))
                                .child(if value.is_none() {
                                    "容量を取得中…"
                                } else {
                                    "容量を取得できません"
                                }),
                        ),
                    };
                } else {
                    info = info.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(t.c("text.muted"))
                            .child(place.path.display().to_string()),
                    );
                }
                cards = cards.child(
                    div()
                        .id(("home-card", index))
                        .w(width)
                        .h(px(if drive { 92. } else { 72. }))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .px(px(16.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(t.c(if selected && focused {
                            "selection.border"
                        } else {
                            "border.subtle"
                        }))
                        .bg(t.c(if selected && focused {
                            "selection.bg"
                        } else if selected {
                            "selection.inactive.bg"
                        } else {
                            "surface.card"
                        }))
                        .when(!selected, |d| d.hover(|s| s.bg(t.c("state.hover"))))
                        .child(self.object_icon(&place.path, true, 32., window, cx))
                        .child(info)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, w, cx| {
                                w.focus(&this.focus, cx);
                                this.selected = Some(index);
                                cx.notify();
                            }),
                        )
                        .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                            if event.click_count() == 2 {
                                this.open_entry(index, w, cx);
                            }
                        })),
                );
            }
            if count > 0 {
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.c("text.secondary"))
                                .child(title),
                        )
                        .child(cards),
                );
            }
        }
        if places.is_empty() {
            page = page.child(empty_state(
                "一致する場所はありません".into(),
                t.c("text.muted"),
            ));
        }
        page
    }
}
