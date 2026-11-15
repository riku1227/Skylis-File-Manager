//! Bounded scroll trajectories and conservative wheel classification, independent of GPUI.
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
