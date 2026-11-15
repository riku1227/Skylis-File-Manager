use gpui::{
    AppContext, Context, IntoElement, Modifiers, MouseMoveEvent, PlatformInput, PowerMode, Render,
    RequestFrameOptions, TestAppContext, ThermalState, Window, WindowOptions, point, prelude::*,
    px,
};
use scheduler::Instant;
use std::{cell::Cell, rc::Rc, time::Duration};

// Compile the actual production gate/pacer here; do not copy their algorithms.
#[path = "../src/window/frame_pacing.rs"]
mod frame_pacing;
use frame_pacing::{FramePacer, minimum_frame_interval};

struct CountedView(Rc<Cell<usize>>);

impl Render for CountedView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        gpui::div()
            .size_full()
            .on_mouse_move(|_, window, _| window.refresh())
    }
}

struct DiagnosticsRoot {
    content: gpui::Entity<CountedView>,
    hud: gpui::Entity<CountedView>,
}

impl Render for DiagnosticsRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .child(self.content.clone())
            .child(self.hud.clone())
    }
}

fn diagnostics_window(cx: &mut TestAppContext) -> gpui::WindowHandle<DiagnosticsRoot> {
    cx.app
        .borrow_mut()
        .open_window(
            WindowOptions {
                inactive_frame_interval: None,
                ..Default::default()
            },
            |_, cx| {
                let content = cx.new(|_| CountedView(Rc::default()));
                let hud = cx.new(|_| CountedView(Rc::default()));
                cx.new(|_| DiagnosticsRoot { content, hud })
            },
        )
        .unwrap()
}

fn diagnostics_count(
    window: gpui::WindowHandle<DiagnosticsRoot>,
    cx: &mut TestAppContext,
) -> Option<u64> {
    window
        .update(cx, |_, window, _| window.frame_diagnostics_count())
        .unwrap()
}

#[gpui::test]
fn diagnostics_are_opt_in_local_and_reset_only_when_the_view_changes(cx: &mut TestAppContext) {
    cx.use_platform_frame_requests();
    let first = diagnostics_window(cx);
    let second = diagnostics_window(cx);
    let platform = cx.test_window(first.into());
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(diagnostics_count(first, cx), None);
    assert_eq!(diagnostics_count(second, cx), None);
    let wakes = platform.frame_wake_count();
    first
        .update(cx, |view, window, _| {
            window.set_frame_diagnostics_view(Some(view.hud.entity_id()));
        })
        .unwrap();
    assert_eq!(diagnostics_count(first, cx), Some(0));
    assert_eq!(platform.frame_wake_count(), wakes, "enabling only observes");
    platform.simulate_frame_request(RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(diagnostics_count(first, cx), Some(1));
    assert_eq!(diagnostics_count(second, cx), None);
    first
        .update(cx, |view, window, _| {
            window.set_frame_diagnostics_view(Some(view.hud.entity_id()));
            assert_eq!(window.frame_diagnostics_count(), Some(1));
            window.set_frame_diagnostics_view(Some(view.content.entity_id()));
            assert_eq!(window.frame_diagnostics_count(), Some(0));
            window.set_frame_diagnostics_view(None);
        })
        .unwrap();
    platform.simulate_frame_request(RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(diagnostics_count(first, cx), None);
}

#[gpui::test]
fn diagnostics_exclude_only_hud_redraws_and_keep_coalesced_application_demand(
    cx: &mut TestAppContext,
) {
    cx.use_platform_frame_requests();
    let window = diagnostics_window(cx);
    let platform = cx.test_window(window.into());
    platform.simulate_frame_request(RequestFrameOptions::default());
    window
        .update(cx, |view, window, cx| {
            window.set_frame_diagnostics_view(Some(view.hud.entity_id()));
            view.hud.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    let before = platform.presentation_count();
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(platform.presentation_count(), before + 1);
    assert_eq!(diagnostics_count(window, cx), Some(0));
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(
        diagnostics_count(window, cx),
        Some(0),
        "idle callback is not a frame"
    );
    window
        .update(cx, |view, _, cx| {
            view.hud.update(cx, |_, cx| cx.notify());
            view.content.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(diagnostics_count(window, cx), Some(1));
    window
        .update(cx, |view, _, cx| view.hud.update(cx, |_, cx| cx.notify()))
        .unwrap();
    platform.simulate_frame_request(RequestFrameOptions {
        force_render: true,
        ..Default::default()
    });
    assert_eq!(
        diagnostics_count(window, cx),
        Some(2),
        "forced refresh is independent demand"
    );
    window
        .update(cx, |view, _, cx| view.hud.update(cx, |_, cx| cx.notify()))
        .unwrap();
    platform.simulate_frame_request(RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(
        diagnostics_count(window, cx),
        Some(3),
        "OS presentation is independent demand"
    );
}

#[gpui::test]
fn diagnostics_count_presentations_not_synchronous_draws_and_preserve_pending_causes(
    cx: &mut TestAppContext,
) {
    cx.use_platform_frame_requests();
    let window = diagnostics_window(cx);
    let platform = cx.test_window(window.into());
    platform.simulate_frame_request(RequestFrameOptions::default());
    window
        .update(cx, |view, window, cx| {
            window.set_frame_diagnostics_view(Some(view.hud.entity_id()));
            view.content.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    window.update(cx, |_, w, cx| w.draw(cx).clear(cx)).unwrap();
    assert_eq!(diagnostics_count(window, cx), Some(0));
    window
        .update(cx, |view, _, cx| view.hud.update(cx, |_, cx| cx.notify()))
        .unwrap();
    window.update(cx, |_, w, cx| w.draw(cx).clear(cx)).unwrap();
    assert_eq!(diagnostics_count(window, cx), Some(0));
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(
        diagnostics_count(window, cx),
        Some(1),
        "HUD draw cannot erase application work"
    );

    window
        .update(cx, |view, _, cx| view.hud.update(cx, |_, cx| cx.notify()))
        .unwrap();
    window.update(cx, |_, w, cx| w.draw(cx).clear(cx)).unwrap();
    window.update(cx, |_, w, cx| w.draw(cx).clear(cx)).unwrap();
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(
        diagnostics_count(window, cx),
        Some(1),
        "clean synchronous draw is neutral"
    );
    platform.simulate_frame_request(RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(
        diagnostics_count(window, cx),
        Some(2),
        "a later native presentation counts"
    );
}

#[gpui::test]
fn runtime_limit_validates_without_changing_or_waking_previous_limit(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| CountedView(Rc::default()));
    let platform = cx.test_window(window.into());
    window
        .update(cx, |_, window, _| {
            assert_eq!(window.frame_rate_limit(), None);
            window.set_frame_rate_limit(Some(60.0)).unwrap();
        })
        .unwrap();
    let wakes = platform.frame_wake_count();
    for invalid in [
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        f64::MAX,
    ] {
        window
            .update(cx, |_, window, _| {
                assert!(window.set_frame_rate_limit(Some(invalid)).is_err());
                assert_eq!(window.frame_rate_limit(), Some(60.0));
            })
            .unwrap();
    }
    window
        .update(cx, |_, window, _| {
            window.set_frame_rate_limit(Some(60.0)).unwrap()
        })
        .unwrap();
    assert_eq!(platform.frame_wake_count(), wakes);
}

#[gpui::test]
fn runtime_limit_is_local_to_each_window(cx: &mut TestAppContext) {
    cx.use_platform_frame_requests();
    let mut windows = Vec::new();
    for limit in [Some(1.0 / 3600.0), None] {
        let draws = Rc::new(Cell::new(0));
        let window = cx
            .app
            .borrow_mut()
            .open_window(
                WindowOptions {
                    inactive_frame_interval: None,
                    ..Default::default()
                },
                |window, cx| {
                    window.set_frame_rate_limit(limit).unwrap();
                    cx.new(|_| CountedView(draws.clone()))
                },
            )
            .unwrap();
        windows.push((limit, cx.test_window(window.into())));
    }
    for (limit, platform) in windows {
        platform.simulate_frame_request(RequestFrameOptions::default());
        platform.simulate_frame_request(RequestFrameOptions {
            force_render: true,
            ..Default::default()
        });
        assert_eq!(
            platform.presentation_count(),
            if limit.is_some() { 1 } else { 2 }
        );
    }
}

// The production frame callback must retain all demand while throttled, including
// forced redraws with a clean scene and the high-rate-input VRR maintenance path.
#[gpui::test]
fn runtime_limit_retains_demand_and_releasing_it_wakes_the_platform(cx: &mut TestAppContext) {
    cx.use_platform_frame_requests();
    for (dirty, callback, forced) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let draws = Rc::new(Cell::new(0));
        let window = cx
            .app
            .borrow_mut()
            .open_window(
                WindowOptions {
                    inactive_frame_interval: None,
                    ..Default::default()
                },
                |window, cx| {
                    window.set_frame_rate_limit(Some(1.0 / 3600.0)).unwrap();
                    cx.new(|_| CountedView(draws.clone()))
                },
            )
            .unwrap();
        let platform = cx.test_window(window.into());
        platform.simulate_frame_request(RequestFrameOptions::default());
        let draws_before = draws.get();
        let callbacks = Rc::new(Cell::new(0));
        window
            .update(cx, |_, window, _| {
                if dirty {
                    window.refresh();
                }
                if callback {
                    let callbacks = callbacks.clone();
                    window.on_next_frame(move |window, _| {
                        callbacks.set(callbacks.get() + 1);
                        window.refresh();
                    });
                }
            })
            .unwrap();
        let wakes = platform.frame_wake_count();
        platform.simulate_frame_request(RequestFrameOptions {
            force_render: forced,
            ..Default::default()
        });
        assert_eq!(draws.get(), draws_before);
        assert_eq!(callbacks.get(), 0);
        assert!(platform.frame_wake_count() > wakes);

        let wakes = platform.frame_wake_count();
        window
            .update(cx, |_, window, _| {
                window.set_frame_rate_limit(None).unwrap()
            })
            .unwrap();
        assert!(platform.frame_wake_count() > wakes);
        platform.simulate_frame_request(RequestFrameOptions::default());
        assert_eq!(
            draws.get(),
            draws_before + usize::from(dirty || callback || forced)
        );
        assert_eq!(callbacks.get(), usize::from(callback));
        let wakes = platform.frame_wake_count();
        platform.simulate_frame_request(RequestFrameOptions::default());
        assert_eq!(
            platform.frame_wake_count(),
            wakes,
            "idle must not re-arm the frame source"
        );
    }
}

#[gpui::test]
fn runtime_limit_does_not_block_mandatory_presentation(cx: &mut TestAppContext) {
    cx.use_platform_frame_requests();
    let draws = Rc::new(Cell::new(0));
    let window = cx
        .app
        .borrow_mut()
        .open_window(
            WindowOptions {
                inactive_frame_interval: None,
                ..Default::default()
            },
            |window, cx| {
                window.set_frame_rate_limit(Some(1.0 / 3600.0)).unwrap();
                cx.new(|_| CountedView(draws.clone()))
            },
        )
        .unwrap();
    let platform = cx.test_window(window.into());
    platform.simulate_frame_request(RequestFrameOptions::default());
    let before = draws.get();
    window.update(cx, |_, window, _| window.refresh()).unwrap();
    platform.simulate_frame_request(RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(draws.get(), before + 1);
}

#[gpui::test]
fn runtime_limit_also_gates_high_rate_input_maintenance_presents(cx: &mut TestAppContext) {
    cx.use_platform_frame_requests();
    let draws = Rc::new(Cell::new(0));
    let window = cx
        .app
        .borrow_mut()
        .open_window(
            WindowOptions {
                inactive_frame_interval: None,
                ..Default::default()
            },
            |window, cx| {
                window.set_frame_rate_limit(Some(1.0 / 3600.0)).unwrap();
                cx.new(|_| CountedView(draws.clone()))
            },
        )
        .unwrap();
    let mut platform = cx.test_window(window.into());
    platform.simulate_frame_request(RequestFrameOptions::default());
    // Use real input dispatch to enter the existing short VRR sustain period.
    for _ in 0..8 {
        platform.simulate_input(PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(10.0), px(10.0)),
            pressed_button: None,
            modifiers: Modifiers::default(),
        }));
        platform.simulate_frame_request(RequestFrameOptions {
            require_presentation: true,
            ..Default::default()
        });
    }
    let presents = platform.presentation_count();
    let renders = draws.get();
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(platform.presentation_count(), presents);
    window
        .update(cx, |_, window, _| {
            window.set_frame_rate_limit(None).unwrap()
        })
        .unwrap();
    platform.simulate_frame_request(RequestFrameOptions::default());
    assert_eq!(
        platform.presentation_count(),
        presents + 1,
        "VRR demand must remain present after releasing the cap"
    );
    assert_eq!(
        draws.get(),
        renders,
        "VRR maintenance should reuse the clean scene"
    );
}

fn active_interval(user_interval: Option<Duration>) -> Option<Duration> {
    minimum_frame_interval(
        true,
        false,
        None,
        None,
        None,
        ThermalState::Nominal,
        crate::PowerMode::Balanced,
        user_interval,
    )
}

#[test]
fn runtime_limit_keeps_long_running_presentation_within_sixty_fps_budget() {
    let start = Instant::now();
    let interval = active_interval(Some(Duration::from_secs_f64(1.0 / 60.0)));
    for hz in [59.94, 60.0, 90.0, 119.88, 120.0, 144.0, 165.0, 240.0] {
        for jitter in [false, true] {
            let mut pacer = FramePacer::default();
            let mut presentations = Vec::new();
            for tick in 0..=(hz * 60.0) as usize {
                let jitter_seconds = if jitter {
                    [0.0, 0.0007, -0.0004, 0.0002][tick % 4]
                } else {
                    0.0
                };
                let elapsed = Duration::from_secs_f64(tick as f64 / hz + jitter_seconds);
                if !pacer.should_defer(start + elapsed, interval) {
                    presentations.push(elapsed);
                }
            }
            // Sliding ten-second windows allow only their single boundary frame.
            for second in 0..=50 {
                let from = Duration::from_secs(second);
                let to = from + Duration::from_secs(10);
                let count = presentations
                    .iter()
                    .filter(|at| **at >= from && **at <= to)
                    .count();
                assert!(
                    count <= 601,
                    "{hz}Hz jitter={jitter}: {count} presents in 10s"
                );
            }
            assert!(presentations.len() >= 2400, "cap must still make progress");
        }
    }
}

#[test]
fn runtime_limit_none_keeps_native_cadence_and_changes_do_not_catch_up() {
    let start = Instant::now();
    let mut pacer = FramePacer::default();
    for tick in 0..240 {
        assert!(!pacer.should_defer(
            start + Duration::from_secs_f64(tick as f64 / 240.0),
            active_interval(None)
        ));
    }
    let capped_at = start + Duration::from_secs(1);
    let interval = active_interval(Some(Duration::from_secs_f64(1.0 / 60.0)));
    assert!(pacer.should_defer(capped_at, interval));
    let after_stall = start + Duration::from_secs(30);
    assert!(!pacer.should_defer(after_stall, interval));
    assert!(pacer.should_defer(after_stall + Duration::from_millis(4), interval));
    assert!(!pacer.should_defer(
        after_stall + Duration::from_millis(5),
        active_interval(None)
    ));
    assert!(pacer.should_defer(after_stall + Duration::from_millis(9), interval));
}

#[test]
fn runtime_limit_combines_with_power_thermal_and_inactive_limits() {
    let options = WindowOptions::default();
    for (inactive, power, thermal, hz, expected_us) in [
        (
            false,
            crate::PowerMode::Balanced,
            ThermalState::Nominal,
            60.0,
            16_666,
        ),
        (
            false,
            crate::PowerMode::LowPower,
            ThermalState::Nominal,
            60.0,
            33_333,
        ),
        (
            false,
            crate::PowerMode::Balanced,
            ThermalState::Serious,
            120.0,
            16_667,
        ),
        (
            true,
            crate::PowerMode::HighPower,
            ThermalState::Nominal,
            60.0,
            33_333,
        ),
        (
            true,
            crate::PowerMode::LowPower,
            ThermalState::Critical,
            10.0,
            100_000,
        ),
    ] {
        let limit = Some(Duration::from_secs_f64(1.0 / hz));
        let interval = minimum_frame_interval(
            true,
            inactive,
            options.inactive_frame_interval,
            options.low_power_frame_interval,
            options.thermal_frame_interval,
            thermal,
            power,
            limit,
        );
        assert_eq!(interval.unwrap().as_micros(), expected_us);
        assert_eq!(
            minimum_frame_interval(
                false,
                inactive,
                options.inactive_frame_interval,
                options.low_power_frame_interval,
                options.thermal_frame_interval,
                thermal,
                power,
                limit
            ),
            None
        );
    }
}
