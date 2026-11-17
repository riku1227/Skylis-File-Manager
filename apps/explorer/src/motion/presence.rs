//! Enter and exit motion for transient surfaces (menus, popups, modals).
//!
//! A surface is logically closed the moment its owner clears its state; input
//! and focus never wait for motion. `Presence` only keeps the last content
//! mounted as a non-interactive ghost until the exit spring settles. One spring
//! state serves both directions, so reopening mid-exit continues from the
//! painted position and velocity instead of restarting.
use gpui::{IntoElement, SpringConfig, SpringState, div, prelude::*};
use std::time::Instant;

/// Displacement below which a presence spring counts as arrived. Coarser than
/// GPUI's default: the last 2% of opacity is invisible, yet its long tail would
/// keep a ghost mounted and frames running for tens of milliseconds.
const SETTLED: f32 = 0.02;

/// The pair of springs that moves one kind of surface in and out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Transition {
    pub enter: SpringConfig,
    pub exit: SpringConfig,
}

pub(crate) struct Presence<T> {
    content: Option<T>,
    live: bool,
    spring: SpringState,
    stepped_at: Option<Instant>,
}

impl<T> Default for Presence<T> {
    fn default() -> Self {
        Self {
            content: None,
            live: false,
            spring: REST,
            stepped_at: None,
        }
    }
}

const REST: SpringState = SpringState {
    position: 0.,
    velocity: 0.,
};

/// What to draw this frame.
pub(crate) struct Shown<'a, T> {
    pub content: &'a T,
    /// 0 = fully absent, 1 = fully present. Clamped for direct use as opacity.
    pub progress: f32,
    /// False while the surface is only a fading ghost: attach no focus or
    /// dismissal handlers and cover it with [`ghost_shield`].
    pub live: bool,
}

impl<T> Presence<T> {
    /// Advances toward the owner's current state. Call once per render with
    /// `Some(content)` while the surface is open. Returns whether another frame
    /// is needed; the caller requests it from its window.
    pub fn sync(
        &mut self,
        current: Option<T>,
        transition: Transition,
        now: Instant,
        reduce_motion: bool,
    ) -> bool {
        let was_live = self.live;
        self.live = current.is_some();
        if let Some(content) = current {
            if self.content.is_none() {
                self.spring = REST;
                self.stepped_at = None;
            }
            self.content = Some(content);
        }
        if self.content.is_none() {
            return false;
        }
        // A direction change starts from the painted state. Time that passed
        // while the surface sat idle must not be credited to the new motion.
        let elapsed = match self.stepped_at {
            Some(at) if was_live == self.live => now.saturating_duration_since(at).as_secs_f32(),
            _ => 0.,
        };
        let (config, target) = if self.live {
            (transition.enter, 1.)
        } else {
            (transition.exit, 0.)
        };
        if !reduce_motion {
            self.spring = config.step(self.spring, target, elapsed);
        }
        if reduce_motion || config.is_settled(self.spring, target, SETTLED) {
            self.spring = SpringState {
                position: target,
                velocity: 0.,
            };
            self.stepped_at = None;
            if !self.live {
                self.content = None;
            }
            return false;
        }
        self.stepped_at = Some(now);
        true
    }

    pub fn shown(&self) -> Option<Shown<'_, T>> {
        self.content.as_ref().map(|content| Shown {
            content,
            progress: self.spring.position.clamp(0., 1.),
            live: self.live,
        })
    }

    pub fn mounted(&self) -> bool {
        self.content.is_some()
    }
}

/// Topmost child of a ghost. Its controls still exist for a few frames, so
/// pointer input over it is absorbed: a click on a vanishing surface neither
/// activates a stale control nor falls through to whatever lies beneath.
pub(crate) fn ghost_shield() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .debug_selector(|| "motion-ghost-shield".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{MENU, MODAL, POPUP};
    use std::time::Duration;

    fn run(
        presence: &mut Presence<u8>,
        current: Option<u8>,
        start: Instant,
        ms: std::ops::Range<u64>,
    ) -> Option<u64> {
        ms.step_by(4)
            .find(|ms| !presence.sync(current, POPUP, start + Duration::from_millis(*ms), false))
    }

    #[test]
    fn enter_starts_absent_and_exit_keeps_a_ghost_until_settled() {
        let start = Instant::now();
        let mut presence = Presence::default();
        assert!(!presence.sync(None, POPUP, start, false));
        assert!(presence.shown().is_none());

        assert!(presence.sync(Some(7), POPUP, start, false));
        let shown = presence.shown().unwrap();
        assert_eq!((*shown.content, shown.progress, shown.live), (7, 0., true));
        let entered = run(&mut presence, Some(7), start, 4..400).unwrap();
        assert!((60..=160).contains(&entered), "enter took {entered}ms");
        assert_eq!(presence.shown().unwrap().progress, 1.);

        // Idle time is not credited to the exit: the first exit frame is opaque.
        let close = start + Duration::from_secs(60);
        assert!(presence.sync(None, POPUP, close, false));
        let ghost = presence.shown().unwrap();
        assert_eq!((*ghost.content, ghost.progress, ghost.live), (7, 1., false));
        let exited = run(&mut presence, None, close, 4..400).unwrap();
        assert!(exited < entered, "exit {exited}ms, enter {entered}ms");
        assert!(!presence.mounted());
    }

    #[test]
    fn reversal_continues_from_the_painted_position_and_takes_new_content() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut presence = Presence::default();
        presence.sync(Some(1), POPUP, at(0), false);
        presence.sync(Some(1), POPUP, at(40), false);
        let partial = presence.shown().unwrap().progress;
        assert!(partial > 0.2 && partial < 1.);

        presence.sync(None, POPUP, at(48), false);
        presence.sync(None, POPUP, at(64), false);
        let fading = presence.shown().unwrap().progress;
        assert!(fading > 0. && fading < partial);

        presence.sync(Some(2), POPUP, at(72), false);
        let reopened = presence.shown().unwrap();
        assert_eq!((*reopened.content, reopened.live), (2, true));
        assert!(
            (reopened.progress - fading).abs() < 0.05,
            "no restart from 0"
        );
        assert!(run(&mut presence, Some(2), start, 76..400).is_some());
        assert_eq!(presence.shown().unwrap().progress, 1.);
    }

    #[test]
    fn reduced_motion_and_long_pauses_never_leave_a_ghost() {
        let start = Instant::now();
        let mut presence = Presence::default();
        assert!(!presence.sync(Some(1), MENU, start, true));
        assert_eq!(presence.shown().unwrap().progress, 1.);
        assert!(!presence.sync(None, MENU, start, true));
        assert!(!presence.mounted());

        // The owner stopped rendering mid-exit (for example a tab switch).
        presence.sync(Some(1), MODAL, start, false);
        presence.sync(Some(1), MODAL, start + Duration::from_millis(64), false);
        presence.sync(None, MODAL, start + Duration::from_millis(72), false);
        assert!(presence.mounted());
        assert!(!presence.sync(None, MODAL, start + Duration::from_secs(5), false));
        assert!(!presence.mounted());
    }

    #[test]
    fn transitions_do_not_overshoot_and_leave_faster_than_they_arrive() {
        for transition in [MENU, POPUP, MODAL] {
            for (config, from, to) in [(transition.enter, 0., 1.), (transition.exit, 1., 0.)] {
                let state = SpringState {
                    position: from,
                    velocity: 0.,
                };
                for ms in 0..=300 {
                    let position = config.step(state, to, ms as f32 / 1000.).position;
                    assert!((0. ..=1.).contains(&position));
                }
            }
            let settle = |config: SpringConfig| config.settle_time(REST, 1., SETTLED);
            assert!(settle(transition.exit) < settle(transition.enter));
            assert!(settle(transition.enter) < Duration::from_millis(200));
        }
    }
}
