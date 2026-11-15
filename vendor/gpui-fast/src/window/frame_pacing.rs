use crate::ThermalState;
use scheduler::Instant;
use std::time::Duration;

const FRAME_INTERVAL_TOLERANCE: Duration = Duration::from_millis(1);

/// Keeps throttled frames on a stable cadence despite callback delivery jitter.
#[derive(Default)]
pub(super) struct FramePacer {
    last_frame: Option<Instant>,
}

impl FramePacer {
    pub(super) fn should_defer(
        &mut self,
        now: Instant,
        minimum_interval: Option<Duration>,
    ) -> bool {
        if let Some(interval) = minimum_interval
            && let Some(last_frame) = self.last_frame
        {
            let deadline = last_frame + interval;
            if now + FRAME_INTERVAL_TOLERANCE < deadline {
                return true;
            }
            // Correct small dispatch delays without shifting subsequent deadlines,
            // but bound the correction to a quarter interval. Larger delays restart
            // the cadence so recovery cannot produce closely spaced catch-up frames.
            self.last_frame = Some(if now.saturating_duration_since(deadline) <= interval / 4 {
                deadline
            } else {
                now
            });
        } else {
            self.last_frame = Some(now);
        }
        false
    }
}

pub(super) fn minimum_frame_interval(
    should_throttle: bool,
    inactive_without_high_rate_input: bool,
    inactive_frame_interval: Option<Duration>,
    low_power_frame_interval: Option<Duration>,
    thermal_frame_interval: Option<Duration>,
    thermal_state: ThermalState,
    power_mode: crate::PowerMode,
    runtime_frame_interval: Option<Duration>,
) -> Option<Duration> {
    if !should_throttle {
        return None;
    }

    [
        runtime_frame_interval,
        inactive_frame_interval.filter(|_| inactive_without_high_rate_input),
        low_power_frame_interval.filter(|_| power_mode == crate::PowerMode::LowPower),
        thermal_frame_interval.filter(|_| {
            matches!(
                thermal_state,
                ThermalState::Critical | ThermalState::Serious
            )
        }),
    ]
    .into_iter()
    .flatten()
    .max()
}
