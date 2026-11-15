//! Elapsed-time FPS sampling; a cumulative submission counter is not a refresh rate.
use std::time::{Duration, Instant};

pub(super) const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct FrameRate {
    at: Instant,
    count: u64,
}

impl FrameRate {
    pub fn new(at: Instant, count: u64) -> Self {
        Self { at, count }
    }
    pub fn sample(&mut self, at: Instant, count: u64) -> Option<f64> {
        let elapsed = at.saturating_duration_since(self.at);
        if elapsed.is_zero() {
            return None;
        }
        let delta = count.checked_sub(self.count);
        self.at = at;
        self.count = count;
        delta.map(|frames| frames as f64 / elapsed.as_secs_f64())
    }
}

/// Shared by the app's HUDs, including failed reads, so more windows do not
/// create more OS sampling work within the same interval.
#[derive(Default)]
pub(super) struct ResidentCache {
    at: Option<Instant>,
    bytes: Option<u64>,
}
impl ResidentCache {
    pub fn sample(&mut self, at: Instant, query: impl FnOnce() -> Option<u64>) -> Option<u64> {
        if self
            .at
            .is_none_or(|last| at.saturating_duration_since(last) >= SAMPLE_INTERVAL)
        {
            self.at = Some(at);
            self.bytes = query();
        }
        self.bytes
    }
}

pub(super) fn labels(fps: Option<f64>, bytes: Option<u64>) -> (String, String) {
    (
        fps.filter(|value| value.is_finite() && *value >= 0.)
            .map(|value| format!("FPS（描画） {value:.0}"))
            .unwrap_or_else(|| "FPS（描画） —".into()),
        bytes
            .map(|value| format!("メモリ {:.1} MiB", value as f64 / (1024. * 1024.)))
            .unwrap_or_else(|| "メモリ 取得不可".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn elapsed_time_and_idle_drive_rate_not_display_hz_or_fixed_tick_count() {
        let now = Instant::now();
        let mut rate = FrameRate::new(now, 10);
        assert_eq!(
            rate.sample(now + Duration::from_millis(1500), 130),
            Some(80.)
        );
        assert_eq!(
            rate.sample(now + Duration::from_millis(2750), 130),
            Some(0.)
        );
    }
    #[test]
    fn reset_and_zero_elapsed_never_create_a_spike_or_infinity() {
        let now = Instant::now();
        let mut rate = FrameRate::new(now, 50);
        assert_eq!(rate.sample(now, 50), None);
        assert_eq!(rate.sample(now + Duration::from_secs(1), 1), None);
        assert_eq!(rate.sample(now + Duration::from_secs(2), 121), Some(120.));
    }
    #[test]
    fn unknown_metrics_are_not_formatted_as_zero() {
        assert_eq!(
            labels(None, None),
            ("FPS（描画） —".into(), "メモリ 取得不可".into())
        );
        assert_eq!(
            labels(Some(0.), Some(1_572_864)),
            ("FPS（描画） 0".into(), "メモリ 1.5 MiB".into())
        );
    }
    #[test]
    fn memory_cache_shares_reads_and_expires_failures() {
        let now = Instant::now();
        let mut cache = ResidentCache::default();
        assert_eq!(cache.sample(now, || Some(100)), Some(100));
        assert_eq!(
            cache.sample(now + Duration::from_millis(900), || panic!(
                "duplicate query"
            )),
            Some(100)
        );
        assert_eq!(cache.sample(now + Duration::from_secs(1), || None), None);
        assert_eq!(
            cache.sample(now + Duration::from_millis(1500), || panic!(
                "failed result must be cached"
            )),
            None
        );
        assert_eq!(
            cache.sample(now + Duration::from_secs(2), || Some(200)),
            Some(200)
        );
    }
}
