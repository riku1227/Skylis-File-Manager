//! Optional low-frequency developer diagnostics, isolated from folder rendering.
mod memory;
mod sample;

use crate::theme::{Theme, ThemeMode};
use gpui::{App, Context, Global, Render, Task, Window, div, prelude::*, px};
use sample::{FrameRate, ResidentCache, SAMPLE_INTERVAL};

#[derive(Default)]
struct ProcessMemory(ResidentCache);
impl Global for ProcessMemory {}

fn resident_memory(cx: &mut App) -> Option<u64> {
    if cx.try_global::<ProcessMemory>().is_none() {
        cx.set_global(ProcessMemory::default());
    }
    let now = cx.background_executor().now();
    cx.global_mut::<ProcessMemory>()
        .0
        .sample(now, memory::resident_bytes)
}

pub struct PerformanceDisplay {
    enabled: bool,
    mode: ThemeMode,
    task: Option<Task<()>>,
    rate: Option<FrameRate>,
    fps: String,
    memory: String,
    display: String,
}

impl PerformanceDisplay {
    pub fn new(mode: ThemeMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_visibility(window, |this, _, window, cx| {
            this.restart_sampling(window, cx);
        })
        .detach();
        cx.observe_window_appearance(window, |this, _, cx| {
            if this.enabled {
                cx.notify();
            }
        })
        .detach();
        let (fps, memory) = sample::labels(None, None);
        Self {
            enabled: false,
            mode,
            task: None,
            rate: None,
            fps,
            memory,
            display: String::new(),
        }
    }

    pub fn configure(
        &mut self,
        enabled: bool,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled_changed = self.enabled != enabled;
        let theme_changed = self.mode != mode;
        self.enabled = enabled;
        self.mode = mode;
        if enabled_changed {
            self.restart_sampling(window, cx);
        } else if theme_changed && enabled {
            cx.notify();
        }
    }

    fn restart_sampling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Dropping the task cancels its pending timer, including hide/show and
        // rapid setting toggles. It holds only a weak reference to this entity.
        self.task = None;
        self.rate = None;
        window.set_frame_diagnostics_view(None);
        if !self.enabled || !window.is_visible() {
            return;
        }
        window.set_frame_diagnostics_view(Some(cx.entity_id()));
        self.rate = Some(FrameRate::new(cx.background_executor().now(), 0));
        self.update_labels(None, window, cx);
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(SAMPLE_INTERVAL).await;
                if view
                    .update_in(cx, |this, window, cx| {
                        if !this.enabled || !window.is_visible() {
                            return false;
                        }
                        let fps = this
                            .rate
                            .as_mut()
                            .zip(window.frame_diagnostics_count())
                            .and_then(|(rate, count)| {
                                rate.sample(cx.background_executor().now(), count)
                            });
                        this.update_labels(fps, window, cx);
                        true
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
            }
        }));
    }

    fn update_labels(&mut self, fps: Option<f64>, window: &Window, cx: &mut Context<Self>) {
        let (fps, memory) = sample::labels(fps, resident_memory(cx));
        let display = window
            .refresh_interval(cx)
            .filter(|interval| !interval.is_zero())
            .map(|interval| format!("画面 {:.0}Hz", 1. / interval.as_secs_f64()))
            .unwrap_or_else(|| "画面 取得不可".into());
        if self.fps != fps || self.memory != memory || self.display != display {
            self.fps = fps;
            self.memory = memory;
            self.display = display;
            cx.notify();
        }
    }
}

impl Render for PerformanceDisplay {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::resolve(self.mode, window.appearance());
        div()
            .id("performance-display")
            .debug_selector(|| "performance-display".into())
            .h(px(24.))
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(16.))
            .border_t_1()
            .border_color(t.c("border.subtle"))
            .bg(t.c("surface.statusbar"))
            .text_size(px(11.))
            .text_color(t.c("text.secondary"))
            .child(div().text_color(t.c("text.muted")).child("開発者"))
            .child(div().flex_1())
            .child(self.fps.clone())
            .child(self.memory.clone())
            .child(self.display.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, RequestFrameOptions, TestAppContext, WindowOptions, WindowVisibility};
    use std::time::Duration;

    #[gpui::test]
    fn sampling_stops_when_disabled_or_hidden_and_restarts_with_a_fresh_baseline(
        cx: &mut TestAppContext,
    ) {
        cx.use_platform_frame_requests();
        let window = cx
            .app
            .borrow_mut()
            .open_window(
                WindowOptions {
                    inactive_frame_interval: None,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| PerformanceDisplay::new(ThemeMode::Light, window, cx)),
            )
            .unwrap();
        let platform = cx.test_window(window.into());
        window
            .update(cx, |view, window, cx| {
                assert!(view.task.is_none());
                assert_eq!(window.frame_diagnostics_count(), None);
                assert!(cx.try_global::<ProcessMemory>().is_none());
                view.configure(true, ThemeMode::Light, window, cx);
                assert!(view.task.is_some());
                assert_eq!(window.frame_diagnostics_count(), Some(0));
            })
            .unwrap();
        cx.executor().run_until_parked();
        platform.simulate_frame_request(RequestFrameOptions::default());
        for _ in 0..3 {
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.executor().run_until_parked();
            platform.simulate_frame_request(RequestFrameOptions::default());
        }
        window
            .read_with(cx, |view, _| assert_eq!(view.fps, "FPS（描画） 0"))
            .unwrap();
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Hidden);
        window
            .update(cx, |view, window, _| {
                assert!(view.task.is_none());
                assert_eq!(window.frame_diagnostics_count(), None);
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.executor().run_until_parked();
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Visible);
        window
            .update(cx, |view, window, cx| {
                assert!(view.task.is_some());
                assert_eq!(view.fps, "FPS（描画） —");
                assert_eq!(window.frame_diagnostics_count(), Some(0));
                view.configure(false, ThemeMode::Light, window, cx);
                assert!(view.task.is_none());
                assert!(view.rate.is_none());
                assert_eq!(window.frame_diagnostics_count(), None);
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.executor().run_until_parked();
        window
            .update(cx, |view, window, _| {
                assert!(view.task.is_none());
                assert_eq!(window.frame_diagnostics_count(), None);
            })
            .unwrap();
    }
}
