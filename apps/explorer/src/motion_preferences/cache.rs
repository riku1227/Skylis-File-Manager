//! Pure preference sampling state; no platform calls or GPUI dependency.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReduceMotionStatus {
    #[default]
    Unknown,
    Enabled,
    Disabled,
}

#[derive(Debug, Default)]
pub(crate) struct Cached<T> {
    pub value: Option<T>,
    pub error: Option<String>,
}

impl<T> Cached<T> {
    pub fn update(&mut self, sample: Result<T, String>) {
        match sample {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

impl Cached<bool> {
    pub fn status(&self) -> ReduceMotionStatus {
        match self.value {
            None => ReduceMotionStatus::Unknown,
            Some(true) => ReduceMotionStatus::Enabled,
            Some(false) => ReduceMotionStatus::Disabled,
        }
    }
}

impl Cached<u32> {
    pub fn wheel_scroll_lines(&self) -> Option<u32> {
        // Keep a successful value for diagnostics across failures, but do not
        // classify input with settings that could have changed in the meantime.
        if self.error.is_some() {
            return None;
        }
        self.value.filter(|&lines| lines != 0 && lines != u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_failure_keeps_reduced_motion_unknown() {
        let mut cache = Cached::<bool>::default();
        cache.update(Err("unavailable".into()));
        assert_eq!(cache.value, None);
        assert_eq!(cache.status(), ReduceMotionStatus::Unknown);
        assert_eq!(cache.error.as_deref(), Some("unavailable"));
    }

    #[test]
    fn reduced_motion_retains_last_success_across_failure() {
        let mut cache = Cached::<bool>::default();
        cache.update(Ok(true));
        cache.update(Err("temporary failure".into()));
        assert_eq!(cache.value, Some(true));
        assert_eq!(cache.status(), ReduceMotionStatus::Enabled);
    }

    #[test]
    fn recovery_updates_reduced_motion_and_clears_diagnostic() {
        let mut cache = Cached::<bool>::default();
        cache.update(Ok(true));
        cache.update(Err("temporary failure".into()));
        cache.update(Ok(false));
        assert_eq!(cache.value, Some(false));
        assert_eq!(cache.status(), ReduceMotionStatus::Disabled);
        assert_eq!(cache.error, None);
    }

    #[test]
    fn unknown_wheel_setting_does_not_enable_classification() {
        let mut cache = Cached::<u32>::default();
        assert_eq!(cache.wheel_scroll_lines(), None);
        cache.update(Err("unavailable".into()));
        assert_eq!(cache.wheel_scroll_lines(), None);
        assert_eq!(cache.error.as_deref(), Some("unavailable"));
    }

    #[test]
    fn zero_and_page_scrolling_disable_classification() {
        let mut cache = Cached::<u32>::default();
        cache.update(Ok(3));
        assert_eq!(cache.wheel_scroll_lines(), Some(3));
        cache.update(Ok(0));
        assert_eq!(cache.wheel_scroll_lines(), None);
        cache.update(Ok(u32::MAX));
        assert_eq!(cache.wheel_scroll_lines(), None);
        cache.update(Ok(1));
        assert_eq!(cache.wheel_scroll_lines(), Some(1));
    }

    #[test]
    fn failed_wheel_refresh_retains_value_but_disables_classification() {
        let mut cache = Cached::<u32>::default();
        cache.update(Ok(3));
        cache.update(Err("temporary failure".into()));
        assert_eq!(cache.value, Some(3));
        assert_eq!(cache.wheel_scroll_lines(), None);
        cache.update(Ok(6));
        assert_eq!(cache.wheel_scroll_lines(), Some(6));
        assert_eq!(cache.error, None);
    }
}
