//! Bounded scroll trajectories, conservative wheel classification, and short UI feedback.
use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(100);

#[derive(Default, Debug)]
pub(crate) struct ScrollMotion {
    segment: Option<Segment>,
}

#[derive(Debug)]
struct Segment {
    start: Instant,
    from: f32,
    target: f32,
    velocity: f32,
    duration: Duration,
}

impl ScrollMotion {
    pub fn active(&self) -> bool {
        self.segment.is_some()
    }
    pub fn cancel(&mut self) {
        self.segment = None;
    }
    pub fn sample(&self, now: Instant, max: f32) -> Option<(f32, f32)> {
        let segment = self.segment.as_ref()?;
        let t = (now.saturating_duration_since(segment.start).as_secs_f32()
            / segment.duration.as_secs_f32())
        .min(1.);
        let distance = segment.target - segment.from;
        let tangent = segment.velocity * segment.duration.as_secs_f32();
        let position = segment.from
            + distance * (3. * t * t - 2. * t * t * t)
            + tangent * (t * t * t - 2. * t * t + t);
        let clamped = position.clamp(0., max.max(0.));
        let velocity = if t == 1. || position != clamped {
            0.
        } else {
            (distance * (6. * t - 6. * t * t) + tangent * (3. * t * t - 4. * t + 1.))
                / segment.duration.as_secs_f32()
        };
        Some((clamped, velocity))
    }
    /// Advances only mathematical state. The caller paints the result on a frame.
    pub fn retarget(&mut self, now: Instant, painted: f32, delta: f32, max: f32) -> bool {
        if !delta.is_finite() || delta == 0. {
            return false;
        }
        let (from, velocity) = self
            .sample(now, max)
            .unwrap_or((painted.clamp(0., max.max(0.)), 0.));
        let old_target = self.segment.as_ref().map_or(from, |s| s.target);
        let same_direction = (old_target - from) * delta > 0.;
        let target =
            (if same_direction { old_target } else { from } + delta).clamp(0., max.max(0.));
        let distance = target - from;
        if distance.abs() < 0.001 {
            self.cancel();
            return false;
        }
        // Repeated notches carry velocity into a slightly longer bounded glide.
        // Precise touchpad momentum bypasses this trajectory entirely.
        let duration = if same_direction {
            Duration::from_millis((100. + distance.abs() * 0.4).clamp(100., 180.) as u64)
        } else {
            DURATION
        };
        let speed = if same_direction && velocity * distance > 0. {
            velocity
                .abs()
                .min(3. * distance.abs() / duration.as_secs_f32())
                * distance.signum()
        } else {
            0.
        };
        self.segment = Some(Segment {
            start: now,
            from,
            target,
            velocity: speed,
            duration,
        });
        true
    }
    pub fn step(&mut self, now: Instant, max: f32) -> Option<f32> {
        let (position, _) = self.sample(now, max)?;
        if self
            .segment
            .as_ref()
            .is_some_and(|s| now.saturating_duration_since(s.start) >= s.duration)
            || position <= 0. && self.segment.as_ref().unwrap().target <= 0.
            || position >= max.max(0.) && self.segment.as_ref().unwrap().target >= max.max(0.)
        {
            self.cancel();
        }
        Some(position)
    }
}

#[derive(Default)]
pub(crate) struct WheelClassifier {
    last: Option<Instant>,
    direct: bool,
    lines: Option<u32>,
}
impl WheelClassifier {
    pub fn smooth(&mut self, now: Instant, delta: Option<f32>, lines: Option<u32>) -> bool {
        let interval = self.last.map(|last| now.saturating_duration_since(last));
        if interval.is_some_and(|dt| dt >= Duration::from_millis(150)) {
            self.direct = false;
        }
        let changed = self.last.is_some() && self.lines != lines;
        let notch = lines
            .filter(|&lines| lines > 0 && lines != u32::MAX)
            .zip(delta)
            .is_some_and(|(lines, delta)| {
                let normalized = delta / lines as f32;
                normalized.is_finite()
                    && normalized != 0.
                    && (normalized - normalized.round()).abs() < 0.001
            });
        // Fast, normalized POINTER notches are still a wheel. Rate alone must
        // not turn smoothing off in the middle of a spin.
        self.direct |= changed || !notch;
        self.last = Some(now);
        self.lines = lines;
        !self.direct
    }
}

pub(crate) fn edge_distance(speed: f32, elapsed: Duration) -> f32 {
    speed * elapsed.min(Duration::from_millis(32)).as_secs_f32()
}

// UI feedback only. Motion is named by role, not by widget, so every surface
// of one kind moves alike. All springs are critically damped (no visible
// overshoot), keep their velocity when retargeted and stop requesting frames
// once settled. See docs/implementation/motion.md.
mod presence;
pub(crate) use presence::{Presence, Transition, ghost_shield};

/// Critically damped springs, mass 1. `response` is the approximate time to
/// cover 99% of the distance.
pub(crate) mod spring {
    use gpui::SpringConfig;
    /// ~85ms. Small controls under the pointer, and every exit.
    pub(crate) const SNAPPY: SpringConfig = SpringConfig::new(6400., 160., 1.);
    /// ~110ms. Selection surfaces and popups arriving next to their trigger.
    pub(crate) const BRISK: SpringConfig = SpringConfig::new(3600., 120., 1.);
    /// ~140ms. Large surfaces that cover the window.
    pub(crate) const SMOOTH: SpringConfig = SpringConfig::new(2304., 96., 1.);
}
pub(crate) const MENU: Transition = Transition {
    enter: spring::BRISK,
    exit: spring::SNAPPY,
};
pub(crate) const POPUP: Transition = Transition {
    enter: spring::BRISK,
    exit: spring::SNAPPY,
};
pub(crate) const MODAL: Transition = Transition {
    enter: spring::SMOOTH,
    exit: spring::SNAPPY,
};
/// Opacity-only fades that have no exit: content swaps and separate windows.
// The preview is a GPUI window only where the OS has no native Quick Look.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) const WINDOW_REVEAL: Duration = Duration::from_millis(110);
pub(crate) const CATEGORY_REVEAL: Duration = Duration::from_millis(90);
pub(crate) const PANE_REVEAL: Duration = Duration::from_millis(100);
/// Distance a surface travels from its origin while arriving.
pub(crate) const POPUP_OFFSET: f32 = 6.;
pub(crate) const MODAL_OFFSET: f32 = 8.;
pub(crate) fn ease_out_cubic(p: f32) -> f32 {
    1. - (1. - p).powi(3)
}

pub(crate) fn switch(
    id: impl Into<gpui::ElementId>,
    enabled: bool,
    width: f32,
    off: gpui::Rgba,
    on: gpui::Rgba,
    knob_off: gpui::Rgba,
    knob_on: gpui::Rgba,
) -> impl gpui::IntoElement {
    use gpui::{AnimationExt, SpringAnimation, div, prelude::*, px};
    div()
        .w(px(width))
        .h(px(20.))
        .flex_shrink_0()
        .relative()
        .rounded(px(10.))
        .with_spring(
            id,
            SpringAnimation::new(spring::SNAPPY).to(if enabled { 1. } else { 0. }),
            move |d, p| {
                let p = p.clamp(0., 1.);
                d.bg(off.blend(on.opacity(p))).child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(px(4. + (width - 20.) * p))
                        .debug_selector(|| "motion-switch-knob".into())
                        .size(px(12.))
                        .rounded(px(6.))
                        .bg(knob_off.blend(knob_on.opacity(p))),
                )
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
    }

    #[test]
    fn trajectory_is_monotone_and_ends_within_bound_without_frame_replay() {
        let now = Instant::now();
        for hz in [60., 120., 144., 240.] {
            let mut motion = ScrollMotion::default();
            assert!(motion.retarget(now, 0., 100., 1000.));
            let mut previous = 0.;
            for n in 1..=(hz as usize) {
                let time = now + Duration::from_secs_f32(n as f32 / hz);
                let next = motion.step(time, 1000.).unwrap_or(previous);
                assert!(next >= previous && next <= 100.);
                previous = next;
            }
            near(previous, 100.);
            assert!(!motion.active());
        }
        let mut motion = ScrollMotion::default();
        motion.retarget(now, 0., 100., 1000.);
        near(
            motion
                .sample(now + Duration::from_millis(50), 1000.)
                .unwrap()
                .0,
            50.,
        );
        near(
            motion.step(now + Duration::from_secs(3600), 1000.).unwrap(),
            100.,
        );
    }

    #[test]
    fn same_direction_retarget_inherits_velocity_and_reversal_discards_backlog() {
        let now = Instant::now();
        let middle = now + Duration::from_millis(50);
        let mut motion = ScrollMotion::default();
        motion.retarget(now, 0., 100., 1000.);
        let (_, velocity) = motion.sample(middle, 1000.).unwrap();
        near(velocity, 1500.);
        motion.retarget(middle, 0., 100., 1000.);
        near(motion.sample(middle, 1000.).unwrap().1, velocity);
        near(
            motion
                .step(middle + Duration::from_millis(180), 1000.)
                .unwrap(),
            200.,
        );
        motion.retarget(now, 0., 100., 1000.);
        motion.retarget(middle, 0., -20., 1000.);
        near(motion.sample(middle, 1000.).unwrap().1, 0.);
        near(motion.step(middle + DURATION, 1000.).unwrap(), 30.);
    }

    #[test]
    fn bounds_and_cancel_discard_unpainted_distance() {
        let now = Instant::now();
        let mut motion = ScrollMotion::default();
        motion.retarget(now, 90., 100., 100.);
        near(motion.step(now + DURATION, 95.).unwrap(), 95.);
        motion.retarget(now, 20., 100., 1000.);
        motion.cancel();
        assert_eq!(motion.step(now + DURATION, 1000.), None);
        // Direct input restarts from the displayed offset, never from old target.
        motion.retarget(now, 20., 10., 1000.);
        near(motion.step(now + DURATION, 1000.).unwrap(), 30.);
    }

    #[test]
    fn classifier_requires_normalized_notches_and_recovers_only_after_idle() {
        let now = Instant::now();
        let mut wheel = WheelClassifier::default();
        assert!(wheel.smooth(now, Some(-3.), Some(3)));
        assert!(wheel.smooth(now + Duration::from_millis(10), Some(-3.), Some(3)));
        assert!(wheel.smooth(now + Duration::from_millis(50), Some(-3.), Some(3)));
        assert!(wheel.smooth(now + Duration::from_millis(200), Some(-3.), Some(3)));
        assert!(!wheel.smooth(now + Duration::from_millis(250), Some(-1.), Some(3)));
        assert!(!wheel.smooth(now + Duration::from_millis(300), Some(-3.), Some(3)));
        assert!(wheel.smooth(now + Duration::from_millis(500), Some(-6.), Some(3)));
        assert!(!wheel.smooth(now + Duration::from_millis(550), Some(-6.), Some(6)));
        assert!(wheel.smooth(now + Duration::from_millis(750), Some(-6.), Some(6)));
    }

    #[test]
    fn classifier_never_smooths_ambiguous_inputs() {
        let now = Instant::now();
        for lines in [None, Some(0), Some(u32::MAX)] {
            assert!(!WheelClassifier::default().smooth(now, Some(3.), lines));
        }
        for delta in [
            None,
            Some(0.),
            Some(1.),
            Some(f32::NAN),
            Some(f32::INFINITY),
        ] {
            assert!(!WheelClassifier::default().smooth(now, delta, Some(3)));
        }
    }

    #[test]
    fn marquee_elapsed_distance_is_refresh_independent_and_caps_stalls() {
        for hz in [60., 120., 144., 240.] {
            let distance: f32 = (0..hz as usize)
                .map(|_| edge_distance(1125., Duration::from_secs_f64(1. / hz)))
                .sum();
            assert!((distance - 1125.).abs() < 0.02);
        }
        near(edge_distance(1125., Duration::from_secs(60)), 36.);
        near(edge_distance(-1125., Duration::from_millis(16)), -18.);
    }

    #[test]
    fn retarget_near_a_new_boundary_limits_inherited_velocity_without_overshoot() {
        let now = Instant::now();
        let middle = now + Duration::from_millis(50);
        let mut motion = ScrollMotion::default();
        motion.retarget(now, 0., 1000., 10000.);
        motion.retarget(middle, 0., 1., 501.);
        assert!(motion.sample(middle, 501.).unwrap().1 <= 30.);
        for millis in 0..=100 {
            let (position, velocity) = motion
                .sample(middle + Duration::from_millis(millis), 501.)
                .unwrap();
            assert!((500. ..=501.).contains(&position));
            assert!(velocity >= -0.001);
        }
    }

    #[test]
    fn repeated_notches_end_within_180ms_of_last_input_without_a_queue() {
        let now = Instant::now();
        let mut motion = ScrollMotion::default();
        for n in 0..10 {
            motion.retarget(now + Duration::from_millis(n * 30), 0., 60., 10000.);
        }
        near(
            motion
                .step(now + Duration::from_millis(450), 10000.)
                .unwrap(),
            600.,
        );
        assert!(!motion.active());
    }
}

#[cfg(test)]
mod feedback_tests {
    use super::*;
    use gpui::{div, prelude::*, px};
    struct SwitchProbe {
        enabled: bool,
    }
    impl gpui::Render for SwitchProbe {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div().child(switch(
                "probe",
                self.enabled,
                32.,
                gpui::black().into(),
                gpui::white().into(),
                gpui::white().into(),
                gpui::black().into(),
            ))
        }
    }
    #[gpui::test]
    fn reduced_motion_switch_mount_and_retarget_draw_final_position_without_frames(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.add_window(|_, _| SwitchProbe { enabled: false });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for (enabled, x) in [(false, 4.), (true, 16.), (false, 4.)] {
            window
                .update(cx, |v, _, cx| {
                    v.enabled = enabled;
                    cx.notify();
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            assert_eq!(
                visual.debug_bounds("motion-switch-knob").unwrap().left(),
                px(x)
            );
            assert_eq!(visual.update(|w, cx| w.simulate_next_frame(cx)), 0);
        }
    }

    #[gpui::test]
    fn switch_mount_is_static_and_interrupted_motion_eventually_stops(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(false));
        let window = cx.add_window(|_, _| SwitchProbe { enabled: true });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert_eq!(
            visual.debug_bounds("motion-switch-knob").unwrap().left(),
            px(16.)
        );
        assert_eq!(visual.update(|w, cx| w.simulate_next_frame(cx)), 0);
        for enabled in [false, true, false] {
            window
                .update(cx, |v, _, cx| {
                    v.enabled = enabled;
                    cx.notify();
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            cx.executor().advance_clock(Duration::from_millis(16));
            visual.update(|w, cx| w.simulate_next_frame(cx));
        }
        let mut stopped = false;
        for _ in 0..30 {
            cx.executor().advance_clock(Duration::from_millis(32));
            let frames = visual.update(|w, cx| {
                w.draw(cx).clear(cx);
                w.simulate_next_frame(cx)
            });
            if frames == 0 {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "a settled switch must not schedule further frames");
        assert_eq!(
            visual.debug_bounds("motion-switch-knob").unwrap().left(),
            px(4.)
        );
    }
    #[test]
    fn feedback_springs_converge_without_visible_overshoot() {
        for (spring, perceptual_ms) in [
            (spring::BRISK, 130.),
            (spring::SNAPPY, 100.),
            (spring::SMOOTH, 160.),
        ] {
            let state = gpui::SpringState {
                position: 0.,
                velocity: 0.,
            };
            for step in 0..=100 {
                let v = spring.step(state, 1., step as f32 / 1000.);
                assert!((0. ..=1.).contains(&v.position));
            }
            assert!(spring.step(state, 1., perceptual_ms / 1000.).position > 0.99);
        }
        assert_eq!(ease_out_cubic(0.), 0.);
        assert_eq!(ease_out_cubic(1.), 1.);
    }
}
