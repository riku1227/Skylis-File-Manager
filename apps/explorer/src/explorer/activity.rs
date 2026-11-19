use super::*;
use crate::{
    motion::{self, Presence},
    operation_center::{
        self, JobBackend, JobKind, JobSnapshot, JobStatus, OperationCenter, PermanentDeleteReason,
        ProgressMeasure, PromptAction, PromptId, PromptKind, PromptRequest, PromptResponse,
        TotalState,
    },
    settings_data::ActivityDisplay,
};
use gpui::{
    AnyElement, AnyWindowHandle, Bounds, Div, MouseButton, Stateful, Task, WindowId, WindowOptions,
    actions, point, size,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const AUTO_DELAY: Duration = Duration::from_millis(500);
const CONFIRM_DELAY: Duration = Duration::from_millis(400);
const TITLE: &str = "ファイル操作";

actions!(activity, [ToggleActivity]);

pub(super) struct ActivityState {
    pub center: Entity<OperationCenter>,
    pub popup_open: bool,
    popup: Presence<()>,
    popup_focus: FocusHandle,
    popup_previous: Option<FocusHandle>,
    prompt: Option<PromptSheet>,
    sheet: Presence<PromptSheet>,
    sheet_focus: FocusHandle,
    sheet_previous: Option<FocusHandle>,
    auto_timer: Option<(Instant, Task<()>)>,
}

#[derive(Clone)]
struct PromptSheet {
    request: PromptRequest,
    shown_at: Instant,
    selected: usize,
    apply_to_all: bool,
}

impl ActivityState {
    pub(super) fn new(window: &Window, cx: &mut Context<Explorer>) -> Self {
        let center = operation_center::init(cx);
        let handle = window.window_handle();
        center.update(cx, |center, _| center.register_window(handle));
        cx.observe(&center, |_, _, cx| cx.notify()).detach();
        Self {
            center,
            popup_open: false,
            popup: Presence::default(),
            popup_focus: cx.focus_handle(),
            popup_previous: None,
            prompt: None,
            sheet: Presence::default(),
            sheet_focus: cx.focus_handle(),
            sheet_previous: None,
            auto_timer: None,
        }
    }

    pub(super) fn modal(&self) -> bool {
        self.prompt.is_some()
    }
}

pub(crate) fn render_pill(
    center: &Entity<OperationCenter>,
    window_id: WindowId,
    t: Theme,
    cx: &App,
) -> Option<AnyElement> {
    let (mut active, mut waiting, mut errors) = (0, 0, 0);
    for job in center.read(cx).snapshots() {
        match job.status {
            JobStatus::WaitingForUser(_) => waiting += 1,
            status if status.is_active() => active += 1,
            _ if job.unread_error => errors += 1,
            _ => {}
        }
    }
    if active + waiting + errors == 0 {
        center.read(cx).clear_pill_bounds(window_id);
        return None;
    }
    let label = [
        (waiting, "要確認"),
        (active, "ファイル操作"),
        (errors, "エラー"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, text)| format!("{text} {count}"))
    .collect::<Vec<_>>()
    .join(" · ");
    let bounds_center = center.clone();
    Some(
        div()
            .id("activity-pill")
            .debug_selector(|| "activity-pill".into())
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .relative()
            .flex_shrink_0()
            .h(px(21.))
            .px(px(8.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            .bg(t.c("surface.chip-selected"))
            .hover(|d| d.bg(t.c("state.hover")))
            .text_color(t.c(if errors > 0 {
                "danger.fg"
            } else if waiting > 0 {
                "meter.warn"
            } else {
                "accent.fg"
            }))
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        bounds_center.read(cx).record_pill_bounds(window_id, bounds)
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(svg().path("icons/activity.svg").size(px(12.)))
            .child(label)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleActivity), cx))
            .into_any_element(),
    )
}

impl Explorer {
    pub(super) fn show_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.activity.modal() {
            return;
        }
        if self.settings_open {
            self.close_settings(window, cx);
        }
        if self.preferences.read(cx).value.activity_display == ActivityDisplay::Window
            || self.activity.center.read(cx).activity_window.is_some()
        {
            self.pop_out(window, cx);
            return;
        }
        if self.activity.popup_open {
            return;
        }
        self.activity.popup_open = true;
        self.activity.popup_previous = window.focused(cx);
        window.focus(&self.activity.popup_focus, cx);
        cx.notify();
    }

    pub(super) fn close_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.activity.popup_open {
            return;
        }
        self.activity.popup_open = false;
        if self.activity.popup_focus.contains_focused(window, cx) {
            let focus = self
                .activity
                .popup_previous
                .take()
                .unwrap_or_else(|| self.tabs[self.active].focus_handle(cx));
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(super) fn toggle_activity(
        &mut self,
        _: &ToggleActivity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.activity.modal() || self.settings_open {
            return;
        }
        if self.preferences.read(cx).value.activity_display == ActivityDisplay::Window
            || self.activity.center.read(cx).activity_window.is_some()
        {
            self.pop_out(window, cx);
        } else if self.activity.popup_open {
            self.close_activity(window, cx);
        } else {
            self.show_activity(window, cx);
        }
    }

    fn pop_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match open_activity_window(
            &self.activity.center,
            self.preferences.clone(),
            window,
            true,
            cx,
        ) {
            Ok(()) => self.close_activity(window, cx),
            Err(error) => {
                self.tab_error = Some(format!("ファイル操作ウィンドウを開けません: {error}"));
                cx.notify();
            }
        }
    }

    fn auto_deadline(&self, window: &Window, cx: &App) -> Option<Instant> {
        let center = self.activity.center.read(cx);
        if self.preferences.read(cx).value.activity_display != ActivityDisplay::Window
            || center.activity_window.is_some()
            || center.auto_presented
            || center.target_window(None) != Some(window.window_handle())
        {
            return None;
        }
        center
            .snapshots()
            .filter(|job| job.status.is_active())
            .map(|job| job.submitted_at + AUTO_DELAY)
            .min()
    }

    fn sync_auto_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let deadline = self.auto_deadline(window, cx);
        if self.activity.auto_timer.as_ref().map(|(at, _)| *at) == deadline {
            return;
        }
        self.activity.auto_timer = deadline.map(|deadline| {
            let delay = deadline.saturating_duration_since(cx.background_executor().now());
            let timer = cx.background_executor().timer(delay);
            let task = cx.spawn_in(window, async move |this, cx| {
                timer.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.activity.auto_timer = None;
                    let now = cx.background_executor().now();
                    if this.auto_deadline(window, cx).is_some_and(|at| now >= at) {
                        match open_activity_window(
                            &this.activity.center,
                            this.preferences.clone(),
                            window,
                            false,
                            cx,
                        ) {
                            Ok(()) => this.close_activity(window, cx),
                            Err(error) => {
                                this.activity
                                    .center
                                    .update(cx, |center, _| center.auto_presented = true);
                                this.tab_error =
                                    Some(format!("ファイル操作ウィンドウを開けません: {error}"));
                                cx.notify();
                            }
                        }
                    }
                });
            });
            (deadline, task)
        });
    }

    pub(super) fn sync_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let request = self
            .activity
            .center
            .read(cx)
            .prompt_for(window.window_handle());
        if request.as_ref().map(|r| r.id) != self.activity.prompt.as_ref().map(|p| p.request.id) {
            if let Some(request) = request {
                if self.activity.prompt.is_none() {
                    self.activity.sheet_previous = window.focused(cx);
                }
                self.activity.prompt = Some(PromptSheet {
                    request,
                    shown_at: now,
                    selected: 0,
                    apply_to_all: false,
                });
                window.focus(&self.activity.sheet_focus, cx);
            } else {
                self.activity.prompt = None;
                let focus = self
                    .activity
                    .sheet_previous
                    .take()
                    .unwrap_or_else(|| self.tabs[self.active].focus_handle(cx));
                window.focus(&focus, cx);
            }
        }
        if self.activity.popup_open && self.activity.center.read(cx).activity_window.is_some() {
            self.close_activity(window, cx);
        }
        if self.activity.popup_open && !self.settings_open && window.is_window_active() {
            let center = self.activity.center.clone();
            if center.read(cx).snapshots().any(|job| job.unread_error) {
                cx.defer(move |cx| center.update(cx, |center, cx| center.acknowledge_errors(cx)));
            }
        }
        let reduce = cx.reduce_motion();
        let popup = self.activity.popup.sync(
            self.activity.popup_open.then_some(()),
            motion::POPUP,
            now,
            reduce,
        );
        let sheet =
            self.activity
                .sheet
                .sync(self.activity.prompt.clone(), motion::MODAL, now, reduce);
        if popup || sheet {
            window.request_animation_frame();
        }
        self.sync_auto_window(window, cx);
    }

    pub(super) fn render_activity_popup(
        &self,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let shown = self.activity.popup.shown()?;
        let child = div()
            .id("activity-popover")
            .debug_selector(|| "activity-popover".into())
            .absolute()
            .right(px(8.))
            .bottom(px(32. - 6. * (1. - shown.progress)))
            .w(px(420.))
            .max_w_full()
            .max_h(
                (window.viewport_size().height - px(44.))
                    .min(px(490.))
                    .max(px(50.)),
            )
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(t.c("border.dialog"))
            .bg(t.c("surface.dialog"))
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .opacity(shown.progress)
            .when(shown.live, |d| {
                d.track_focus(&self.activity.popup_focus)
                    .tab_group()
                    .tab_stop(false)
                    .on_mouse_down_out(cx.listener(
                        |this, event: &gpui::MouseDownEvent, window, cx| {
                            if this.activity.popup_open
                                && !this.activity.center.read(cx).pill_contains(
                                    window.window_handle().window_id(),
                                    event.position,
                                )
                            {
                                this.close_activity(window, cx);
                            }
                        },
                    ))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.modifiers.control
                            || event.keystroke.modifiers.alt
                            || event.keystroke.modifiers.platform
                        {
                            return;
                        }
                        match event.keystroke.key.as_str() {
                            "escape" => this.close_activity(window, cx),
                            "up" => window.focus_prev(cx),
                            "down" => window.focus_next(cx),
                            "tab" if event.keystroke.modifiers.shift => window.focus_prev(cx),
                            "tab" => window.focus_next(cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(px(10.))
                    .gap(px(5.))
                    .border_b_1()
                    .border_color(t.c("border.subtle"))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(TITLE),
                    )
                    .child(button("activity-pop-out", "別ウィンドウで開く", t).when(
                        shown.live,
                        |d| {
                            keyboard_button(d, t).on_click(
                                cx.listener(|this, _, window, cx| this.pop_out(window, cx)),
                            )
                        },
                    ))
                    .child(
                        icon_button("activity-close", "icons/x.svg", true, t)
                            .role(gpui::Role::Button)
                            .aria_label("ファイル操作を閉じる")
                            .when(shown.live, |d| {
                                keyboard_button(d, t).on_click(cx.listener(
                                    |this, _, window, cx| this.close_activity(window, cx),
                                ))
                            }),
                    ),
            )
            .child(ActivityList::render(
                &self.activity.center,
                shown.live,
                Some(window.window_handle()),
                t,
                cx,
            ))
            .when(!shown.live, |d| d.child(motion::ghost_shield()));
        Some(gpui::deferred(child).with_priority(2).into_any_element())
    }

    fn answer_prompt(&mut self, prompt: PromptId, action: PromptAction, cx: &mut Context<Self>) {
        let Some(sheet) = &self.activity.prompt else {
            return;
        };
        if sheet.request.id != prompt {
            return;
        }
        let response = PromptResponse {
            job: sheet.request.job,
            prompt: sheet.request.id,
            action,
            apply_to_all: action != PromptAction::Cancel && sheet.apply_to_all,
        };
        self.activity
            .center
            .update(cx, |center, cx| center.respond(response, cx));
    }

    fn prompt_key(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let now = cx.background_executor().now();
        let Some(sheet) = &mut self.activity.prompt else {
            return;
        };
        let actions = sheet.request.kind.actions();
        let prompt = sheet.request.id;
        match event.keystroke.key.as_str() {
            "escape" => self.answer_prompt(prompt, PromptAction::Cancel, cx),
            "tab" | "right" | "left" => {
                let backwards = event.keystroke.key == "left" || event.keystroke.modifiers.shift;
                let len = actions.len() + usize::from(sheet.request.kind.allow_apply_to_all());
                sheet.selected = (sheet.selected + if backwards { len - 1 } else { 1 }) % len;
                cx.notify();
            }
            "enter" | "space"
                if !event.is_held
                    && now.saturating_duration_since(sheet.shown_at) >= CONFIRM_DELAY =>
            {
                if let Some(&action) = actions.get(sheet.selected) {
                    self.answer_prompt(prompt, action, cx);
                } else {
                    sheet.apply_to_all = !sheet.apply_to_all;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub(super) fn render_activity_sheet(
        &self,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let shown = self.activity.sheet.shown()?;
        let sheet = shown.content;
        let prompt = sheet.request.id;
        let (title, detail) = match &sheet.request.kind {
            PromptKind::ConfirmPermanentDelete {
                representative,
                items,
                reason,
                known_bytes,
            } => {
                let title = match reason {
                    PermanentDeleteReason::RecycleUnavailable => {
                        "ゴミ箱へ移動できません — 完全に削除しますか？"
                    }
                    PermanentDeleteReason::Explicit => "完全に削除しますか？",
                };
                let name = representative
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| representative.display().to_string());
                let summary = match known_bytes {
                    Some(bytes) => {
                        format!(
                            "{items} 項目（合計 {}） · {name}",
                            explorer_core::format::size(*bytes)
                        )
                    }
                    None => format!("{items} 項目 · {name}"),
                };
                let mut lines = Vec::new();
                if *reason == PermanentDeleteReason::RecycleUnavailable {
                    lines.push(
                        "ゴミ箱へ移動できない項目があるため、完全に削除する必要があります。"
                            .to_string(),
                    );
                }
                lines.push(summary);
                if let Some(location) = representative
                    .parent()
                    .map(|parent| format!("場所  {}", parent.display()))
                {
                    lines.push(location);
                }
                (title, lines.join("\n"))
            }
            PromptKind::FileConflict {
                source,
                destination,
                ..
            } => (
                "同じ名前の項目があります",
                format!(
                    "転送元  {}\n保存先  {}",
                    source.display(),
                    destination.display()
                ),
            ),
            PromptKind::ItemError { item, message } => (
                "操作を続けられません",
                format!("{}\n{message}", item.display()),
            ),
        };
        let panel = div()
            .id("activity-confirmation-sheet")
            .debug_selector(|| "activity-confirmation-sheet".into())
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .when(shown.live, |d| {
                d.track_focus(&self.activity.sheet_focus)
                    .on_key_down(cx.listener(Self::prompt_key))
            })
            .w(px(490.))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll()
            .p(px(18.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.c("border.dialog"))
            .bg(t.c("surface.dialog"))
            .shadow_lg()
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .my(px(12.))
                    .text_color(t.c("text.secondary"))
                    .child(detail),
            )
            .child(
                div().flex().flex_wrap().gap(px(8.)).children(
                    sheet
                        .request
                        .kind
                        .actions()
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(index, action)| {
                            let label = match action {
                                PromptAction::Cancel => "中止",
                                PromptAction::Confirm => "完全に削除する",
                                PromptAction::Skip => "スキップ",
                                PromptAction::Replace => "置換",
                                PromptAction::KeepBoth => "両方残す",
                                PromptAction::Retry => "再試行",
                            };
                            button("activity-confirm-action", label, t)
                                .id(("confirm-action", index))
                                .debug_selector(move || {
                                    format!("activity-confirm-action-{index}").into()
                                })
                                .when(action == PromptAction::Confirm, |d| {
                                    d.bg(t.c("danger.solid")).text_color(t.c("text.on-accent"))
                                })
                                .when(sheet.selected == index, |d| {
                                    d.border_color(t.c("accent.fg")).aria_active_descendant()
                                })
                                .when(shown.live, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.answer_prompt(prompt, action, cx)
                                    }))
                                })
                        }),
                ),
            )
            .when(sheet.request.kind.allow_apply_to_all(), |d| {
                d.child(
                    button(
                        "activity-apply-all",
                        if sheet.apply_to_all {
                            "✓ 以降の競合にも適用"
                        } else {
                            "以降の競合にも適用"
                        },
                        t,
                    )
                    .mt(px(10.))
                    .when(sheet.selected == sheet.request.kind.actions().len(), |d| {
                        d.border_color(t.c("accent.fg"))
                    })
                    .when(shown.live, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            if let Some(sheet) = &mut this.activity.prompt {
                                sheet.apply_to_all = !sheet.apply_to_all;
                                cx.notify();
                            }
                        }))
                    }),
                )
            });
        Some(
            gpui::deferred(
                div()
                    .id("activity-confirmation-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(16.))
                    .bg(t.c("surface.backdrop"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .child(
                        panel
                            .relative()
                            .top(px(-motion::MODAL_OFFSET * (1. - shown.progress))),
                    )
                    .opacity(shown.progress)
                    .when(!shown.live, |d| d.child(motion::ghost_shield())),
            )
            .with_priority(5)
            .into_any_element(),
        )
    }
}

fn button(id: &'static str, label: &'static str, t: Theme) -> Stateful<Div> {
    button_with_id(id, move || id.into(), label, t)
}

fn job_button(prefix: &'static str, job: u64, label: &'static str, t: Theme) -> Stateful<Div> {
    button_with_id(
        (prefix, job as usize),
        move || format!("{prefix}-{job}"),
        label,
        t,
    )
}

fn button_with_id(
    id: impl Into<gpui::ElementId>,
    debug: impl Fn() -> String + 'static,
    label: &'static str,
    t: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(debug)
        .role(gpui::Role::Button)
        .aria_label(label)
        .px(px(8.))
        .py(px(4.))
        .rounded(px(5.))
        .border_1()
        .border_color(t.c("button.border"))
        .bg(t.c("button.bg"))
        .hover(|d| d.bg(t.c("state.hover")))
        .cursor_pointer()
        .child(label)
}

fn keyboard_button(button: Stateful<Div>, t: Theme) -> Stateful<Div> {
    button
        .tab_index(0)
        .focus(|d| d.bg(t.c("surface.chip-selected")))
}

struct ActivityList;

impl ActivityList {
    fn render(
        center: &Entity<OperationCenter>,
        interactive: bool,
        clicked_window: Option<AnyWindowHandle>,
        t: Theme,
        cx: &App,
    ) -> AnyElement {
        let jobs = center.read(cx);
        let mut list = div()
            .id("activity-list")
            .debug_selector(|| "activity-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(10.))
            .text_size(px(12.));
        let waiting_for_user = jobs
            .snapshots()
            .any(|job| matches!(job.status, JobStatus::WaitingForUser(_)));
        let mut finished = false;
        for (section, title) in ["要確認", "進行中", "待機中", "最近の操作"]
            .into_iter()
            .enumerate()
        {
            let mut matching = jobs
                .snapshots()
                .filter(|job| Self::section(job.status) == section)
                .peekable();
            if matching.peek().is_none() {
                continue;
            }
            finished |= section == 3;
            list = list
                .child(
                    div()
                        .mb(px(5.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .children(matching.map(|job| {
                    Self::card(
                        center,
                        job,
                        interactive,
                        clicked_window,
                        waiting_for_user,
                        t,
                    )
                }));
        }
        if jobs.snapshots().next().is_none() {
            list = list.child(
                div()
                    .text_color(t.c("text.muted"))
                    .child("ファイル操作はありません"),
            );
        }
        let center = center.clone();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(list)
            .when(finished, |d| {
                d.child(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .border_t_1()
                        .border_color(t.c("border.subtle"))
                        .child(button("activity-clear", "完了済みを消す", t).when(
                            interactive,
                            |button| {
                                keyboard_button(button, t).on_click(move |_, _, cx| {
                                    center.update(cx, |center, cx| center.dismiss_finished(cx))
                                })
                            },
                        )),
                )
            })
            .into_any_element()
    }

    fn section(status: JobStatus) -> usize {
        match status {
            JobStatus::WaitingForUser(_) => 0,
            JobStatus::Running | JobStatus::Paused | JobStatus::Cancelling => 1,
            JobStatus::Queued => 2,
            _ => 3,
        }
    }

    fn card(
        center: &Entity<OperationCenter>,
        job: &JobSnapshot,
        interactive: bool,
        clicked_window: Option<AnyWindowHandle>,
        waiting_for_user: bool,
        t: Theme,
    ) -> impl IntoElement {
        let title = match job.kind {
            JobKind::Copy => "コピー",
            JobKind::Move => "移動",
            JobKind::CopyOrMove => "コピーまたは移動",
            JobKind::Trash => "ゴミ箱へ移動",
            JobKind::PermanentDelete => "完全に削除",
            JobKind::ArchiveCreate(explorer_core::archive::ArchiveFormat::Zip) => "ZIP 圧縮",
            JobKind::ArchiveCreate(explorer_core::archive::ArchiveFormat::SevenZip) => "7z 圧縮",
            JobKind::ArchiveExtract => "解凍",
        };
        let status = match job.status {
            JobStatus::Queued => "待機中",
            JobStatus::Running => "進行中",
            JobStatus::Paused => "一時停止中",
            JobStatus::Cancelling => "中止しています",
            JobStatus::WaitingForUser(_) => "Skylis のメインウィンドウで確認してください",
            JobStatus::Completed => "完了",
            JobStatus::CompletedWithIssues => "一部失敗",
            JobStatus::Failed => "失敗",
            JobStatus::Cancelled => "中止",
        };
        let id = job.id;
        let cancel_center = center.clone();
        let navigation_center = center.clone();
        let origin = job.origin;
        let open_target = if matches!(
            job.status,
            JobStatus::Completed | JobStatus::CompletedWithIssues
        ) {
            job.output
                .as_ref()
                .and_then(|output| {
                    output.parent().map(|parent| {
                        (
                            parent.to_path_buf(),
                            output
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned()),
                        )
                    })
                })
                .or_else(|| job.destination.clone().map(|path| (path, None)))
        } else {
            None
        };
        div()
            .id(("activity-job", id.0 as usize))
            .mb(px(8.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(t.c("surface.raised"))
            .border_1()
            .border_color(t.c("border.subtle"))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(6.))
                    .child(title)
                    .child(div().text_color(t.c("text.secondary")).child(status)),
            )
            .when(job.status == JobStatus::Queued && waiting_for_user, |d| {
                d.child(
                    div()
                        .mt(px(2.))
                        .text_color(t.c("text.muted"))
                        .child("前の操作が確認待ちです"),
                )
            })
            .when_some(job.destination.as_ref(), |d, path| {
                let path = path.to_string_lossy().into_owned();
                d.child(path_text(("activity-destination", id.0 as usize), path, t))
            })
            .when(
                job.status.is_active() && job.status != JobStatus::Queued,
                |d| d.child(Self::progress(job, t)),
            )
            .when_some(job.progress.current_item.clone(), |d, path| {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                d.child(
                    div()
                        .id(("activity-current-item", id.0 as usize))
                        .truncate()
                        .text_color(t.c("text.muted"))
                        .child(name)
                        .tooltip(move |_, cx| cx.new(|_| ActivityTip(path.clone(), t)).into()),
                )
            })
            .when(job.issue_count > 0, |d| {
                let count = job.issue_count;
                d.child(
                    div()
                        .text_color(t.c("danger.fg"))
                        .child(format!("問題 {count} 件")),
                )
                .children(job.issues().iter().take(3).map(|issue| {
                    let text = issue.item.as_ref().map_or_else(
                        || issue.message.clone(),
                        |item| format!("{item} · {}", issue.message),
                    );
                    div().text_color(t.c("danger.fg")).child(text)
                }))
            })
            .when(
                job.cancellable && job.status != JobStatus::Cancelling,
                |d| {
                    d.child(
                        job_button("activity-cancel", id.0, "中止", t)
                            .mt(px(6.))
                            .when(interactive, |button| {
                                keyboard_button(button, t).on_click(move |_, _, cx| {
                                    cancel_center.update(cx, |center, cx| center.cancel(id, cx));
                                })
                            }),
                    )
                },
            )
            .when(
                job.status == JobStatus::Running
                    && !job.cancellable
                    && matches!(job.backend, JobBackend::OsDelegated { .. }),
                |d| {
                    d.child(div().text_color(t.c("text.muted")).child(if cfg!(windows) {
                        "中止は Windows の進捗ウィンドウから"
                    } else {
                        "Finder に委譲中 · アプリからの中止には対応していません"
                    }))
                },
            )
            .when_some(open_target, |d, (folder, select)| {
                d.child(
                    job_button("activity-open-destination", id.0, "保存先を開く", t)
                        .mt(px(6.))
                        .when(interactive, |button| {
                            keyboard_button(button, t).on_click(move |_, _, cx| {
                                open_destination(
                                    &navigation_center,
                                    clicked_window,
                                    origin,
                                    folder.clone(),
                                    select.clone(),
                                    cx,
                                )
                            })
                        }),
                )
            })
    }

    fn progress(job: &JobSnapshot, t: Theme) -> AnyElement {
        let note = match job.progress.measure {
            ProgressMeasure::Measured {
                bytes_done,
                bytes_total: TotalState::Known(total),
                ..
            } => {
                format!(
                    "{} / {}",
                    explorer_core::format::size(bytes_done),
                    explorer_core::format::size(total)
                )
            }
            ProgressMeasure::Measured {
                items_done,
                items_total: TotalState::Known(total),
                bytes_total: TotalState::Unknown,
                ..
            } => format!("{items_done} / {total} 項目"),
            ProgressMeasure::Measured {
                bytes_done,
                items_done,
                ..
            } => {
                format!(
                    "{} · {} 項目",
                    explorer_core::format::size(bytes_done),
                    items_done
                )
            }
            ProgressMeasure::Relative { work_total, .. } if work_total > 0 => {
                "Windows からの進捗".into()
            }
            _ => "進捗の詳細を取得できません".into(),
        };
        let label = if job.progress.hydrating_cloud_likely {
            "クラウドから取得中の可能性があります".into()
        } else {
            note
        };
        div()
            .mt(px(4.))
            .text_color(t.c("text.muted"))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(label)
                    .when_some(job.percent, |d, percent| d.child(format!("{percent}%"))),
            )
            .child(
                div()
                    .mt(px(3.))
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(t.c("surface.field"))
                    .when_some(job.percent, |d, percent| {
                        d.child(
                            div()
                                .h_full()
                                .w(gpui::relative(f32::from(percent) / 100.))
                                .rounded(px(2.))
                                .bg(t.c("accent.fg")),
                        )
                    })
                    .when(job.percent.is_none(), |d| {
                        d.child(
                            div()
                                .h_full()
                                .w(px(18.))
                                .rounded(px(2.))
                                .bg(t.c("accent.fg").opacity(0.6)),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn path_text(id: impl Into<gpui::ElementId>, path: String, t: Theme) -> impl IntoElement {
    div()
        .id(id)
        .truncate()
        .text_color(t.c("text.secondary"))
        .child(path.clone())
        .tooltip(move |_, cx| cx.new(|_| ActivityTip(path.clone(), t)).into())
}

struct ActivityTip(String, Theme);
impl Render for ActivityTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(6.))
            .max_w(px(540.))
            .rounded(px(5.))
            .bg(self.1.c("surface.dialog"))
            .border_1()
            .border_color(self.1.c("border.dialog"))
            .text_color(self.1.c("text.primary"))
            .child(self.0.clone())
    }
}

fn open_destination(
    center: &Entity<OperationCenter>,
    clicked_window: Option<AnyWindowHandle>,
    origin: Option<AnyWindowHandle>,
    path: PathBuf,
    select: Option<String>,
    cx: &mut App,
) {
    let target = clicked_window
        .and_then(|handle| handle.downcast::<Explorer>())
        .or_else(|| {
            center
                .read(cx)
                .target_window(origin)
                .and_then(|handle| handle.downcast::<Explorer>())
        });
    let Some(target) = target else {
        return;
    };
    cx.defer(move |cx| {
        let _ = target.update(cx, |view, window, cx| {
            if view.activity.modal() {
                return;
            }
            if view.settings_open {
                view.close_settings(window, cx);
            }
            view.close_activity(window, cx);
            view.tabs[view.active].update(cx, |tab, cx| {
                tab.navigate_to_and_select(path, select, window, cx)
            });
            window.activate_window();
        });
    });
}

fn open_activity_window(
    center: &Entity<OperationCenter>,
    preferences: Entity<crate::settings::Preferences>,
    owner: &Window,
    focus: bool,
    cx: &mut App,
) -> anyhow::Result<()> {
    if let Some(handle) = center.read(cx).activity_window {
        if handle
            .update(cx, |_, window, _| {
                if focus {
                    window.activate_window();
                }
            })
            .is_ok()
        {
            return Ok(());
        }
    }
    let bounds = Bounds::new(
        owner.bounds().origin + point(px(42.), px(42.)),
        size(px(450.), px(430.)),
    );
    let options = WindowOptions {
        focus,
        window_min_size: Some(size(px(320.), px(220.))),
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("ファイル操作 — Skylis".into()),
            appears_transparent: cfg!(windows),
            ..Default::default()
        }),
        is_minimizable: false,
        ..Default::default()
    };
    let window_center = center.clone();
    let handle = cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            cx.observe(&window_center, |_, _, cx| cx.notify()).detach();
            cx.observe(&preferences, |_, _, cx| cx.notify()).detach();
            cx.observe_window_appearance(window, |_, _, cx| cx.notify())
                .detach();
            let focus_handle = cx.focus_handle();
            window.focus(&focus_handle, cx);
            ActivityWindow {
                center: window_center,
                preferences,
                focus: focus_handle,
            }
        })
    })?;
    center.update(cx, |center, cx| {
        center.activity_window = Some(handle.into());
        center.auto_presented = true;
        if focus {
            center.acknowledge_errors(cx);
        }
        cx.notify();
    });
    Ok(())
}

struct ActivityWindow {
    center: Entity<OperationCenter>,
    preferences: Entity<crate::settings::Preferences>,
    focus: FocusHandle,
}

impl Render for ActivityWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.is_window_active() && self.center.read(cx).snapshots().any(|job| job.unread_error)
        {
            let center = self.center.clone();
            cx.defer(move |cx| center.update(cx, |center, cx| center.acknowledge_errors(cx)));
        }
        let value = &self.preferences.read(cx).value;
        let t = Theme::resolve(value.theme.into(), window.appearance());
        apply_frame_limit(window, value.high_refresh_rate);
        div()
            .id("activity-window")
            .track_focus(&self.focus)
            .tab_group()
            .tab_stop(false)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.c("surface.canvas"))
            .text_color(t.c("text.primary"))
            .text_size(px(12.))
            .on_key_down(|event, window, cx| {
                if event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                    || event.keystroke.modifiers.platform
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" => window.remove_window(),
                    "up" => window.focus_prev(cx),
                    "down" => window.focus_next(cx),
                    "tab" if event.keystroke.modifiers.shift => window.focus_prev(cx),
                    "tab" => window.focus_next(cx),
                    _ => return,
                }
                cx.stop_propagation();
            })
            .when(cfg!(windows), |d| {
                d.child(
                    div()
                        .id("activity-titlebar")
                        .h(px(38.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(t.c("border.subtle"))
                        .child(
                            div()
                                .id("activity-window-drag")
                                .flex_1()
                                .h_full()
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .window_control_area(gpui::WindowControlArea::Drag)
                                .child(TITLE),
                        )
                        .child(window_controls(window.is_maximized(), false, t)),
                )
            })
            .child(ActivityList::render(&self.center, true, None, t, cx))
    }
}

#[cfg(test)]
mod tests;
