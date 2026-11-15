//! OS motion policy sampled only at startup, activation and settings opening.
//!
//! This never changes the user's saved smooth-scroll preference. A failed query
//! retains the last successful motion value; an unknown value leaves GPUI's
//! existing default in place.

mod cache;
#[cfg(target_os = "macos")]
#[path = "motion_preferences/macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "motion_preferences/windows.rs"]
mod platform;

use cache::Cached;
use gpui::{App, Global};

pub use cache::ReduceMotionStatus;

#[derive(Default)]
struct MotionPreferences {
    reduce_motion: Cached<bool>,
    wheel_scroll_lines: Cached<u32>,
}

impl Global for MotionPreferences {}

/// Install the global cache and sample the operating system before opening UI.
pub fn init(cx: &mut App) {
    refresh(cx);
}

/// Re-sample on activation or settings opening. No polling task is installed.
pub fn refresh(cx: &mut App) {
    let reduce_motion = platform::reduce_motion();
    let wheel_scroll_lines = platform::wheel_scroll_lines();
    let preferences = cx.default_global::<MotionPreferences>();
    update_sample(
        &mut preferences.reduce_motion,
        reduce_motion,
        "reduce motion",
    );
    update_sample(
        &mut preferences.wheel_scroll_lines,
        wheel_scroll_lines,
        "wheel scroll lines",
    );
    if let Some(reduce_motion) = preferences.reduce_motion.value {
        cx.set_reduce_motion(reduce_motion);
    }
}

/// Whether the OS requests reduced motion, including the last successful sample.
pub fn reduce_motion_status(cx: &App) -> ReduceMotionStatus {
    cx.try_global::<MotionPreferences>()
        .map(|preferences| preferences.reduce_motion.status())
        .unwrap_or_default()
}

/// A current usable Windows line count for notch normalization, otherwise None.
/// Zero, page scrolling, unsupported platforms and failed queries are ineligible.
pub fn wheel_scroll_lines(cx: &App) -> Option<u32> {
    cx.try_global::<MotionPreferences>()
        .and_then(|preferences| preferences.wheel_scroll_lines.wheel_scroll_lines())
}

/// Supply deterministic OS normalization in input tests without changing the OS.
#[cfg(test)]
pub fn set_wheel_scroll_lines_for_test(cx: &mut App, lines: u32) {
    cx.default_global::<MotionPreferences>()
        .wheel_scroll_lines
        .update(Ok(lines));
}

fn update_sample<T>(cache: &mut Cached<T>, sample: Result<T, String>, label: &str) {
    // Retain the diagnostic in the cache, and report each distinct failure only
    // once until a successful sample. Activation must not flood stderr.
    if let Err(error) = &sample
        && cache.error.as_ref() != Some(error)
    {
        eprintln!("Skylis: OS {label} unavailable: {error}");
    }
    cache.update(sample);
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn reduce_motion() -> Result<bool, String> {
        Err("OS motion preferences are not supported on this platform".into())
    }

    pub fn wheel_scroll_lines() -> Result<u32, String> {
        Err("OS wheel normalization is not supported on this platform".into())
    }
}
