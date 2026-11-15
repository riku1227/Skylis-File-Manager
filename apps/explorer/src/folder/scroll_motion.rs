//! Public GPUI capture hooks and one demand-driven callback chain for the visible folder.
use super::*;
use crate::motion::{ScrollMotion, WheelClassifier};
use gpui::{Bounds, HitboxBehavior, ScrollDelta, ScrollEvent, ScrollSource, canvas, point};
use std::{cell::Cell, rc::Rc, time::Instant};

#[derive(Default)]
pub(super) struct FolderMotion {
    pub trajectory: ScrollMotion,
    classifier: WheelClassifier,
    painted: Rc<Cell<f32>>,
    unpainted_step: Rc<Cell<bool>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    generation: u64,
    pending: Option<u64>,
    owner: Option<gpui::WindowId>,
    last_frame: Option<Instant>,
    layout: Option<(bool, bool, usize)>,
}

impl FolderView {
    /// Called before hiding, moving, or re-laying out a folder. An already queued
    /// callback is harmless and is not replaced until it has drained.
    pub fn cancel_scroll_motion(&mut self) {
        self.stop_wheel_motion();
        self.motion.classifier = WheelClassifier::default();
        self.cancel_marquee();
        self.motion.last_frame = None;
    }

    fn stop_wheel_motion(&mut self) {
        if self.motion.trajectory.active() || self.motion.unpainted_step.get() {
            let handle = self.scroll.0.borrow().base_handle.clone();
            // Hitboxes still describe the last paint, not the mathematical now.
            handle.set_offset(point(handle.offset().x, px(-self.motion.painted.get())));
            self.motion.trajectory.cancel();
            self.motion.unpainted_step.set(false);
        }
        self.motion.generation = self.motion.generation.wrapping_add(1);
    }

    pub(super) fn attach_motion_window(&mut self, window: &Window) {
        self.cancel_scroll_motion();
        self.motion.owner = Some(window.window_handle().window_id());
        self.motion.pending = None;
    }

    fn smooth_scroll_enabled(&self, cx: &App) -> bool {
        !cx.reduce_motion()
            && cx
                .try_global::<crate::settings::SharedPreferences>()
                .is_none_or(|p| p.0.read(cx).value.smooth_scroll)
    }

    pub(super) fn check_motion_layout(&mut self) {
        let layout = (self.gallery, self.compact, self.gallery_columns);
        if self
            .motion
            .layout
            .is_some_and(|previous| previous != layout)
        {
            self.cancel_scroll_motion();
        }
        self.motion.layout = Some(layout);
    }

    fn capture_scroll(
        &mut self,
        event: &ScrollEvent,
        line_height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.context_menu.is_some() || self.name_editor.is_some() || self.marquee_in_progress() {
            self.stop_wheel_motion();
            return;
        }
        let delta = match event.delta {
            ScrollDelta::Lines(lines) if event.source == ScrollSource::Pointer && lines.x == 0. => {
                Some(lines.y)
            }
            _ => None,
        };
        let now = cx.background_executor().now();
        let classified = self.motion.classifier.smooth(
            now,
            delta,
            crate::motion_preferences::wheel_scroll_lines(cx),
        );
        if !classified || !self.smooth_scroll_enabled(cx) {
            self.stop_wheel_motion();
            // Default bubbling handles precise/pan/ambiguous input exactly once.
            return;
        }
        let distance = -f32::from(event.delta.pixel_delta(line_height).y);
        let handle = self.scroll.0.borrow().base_handle.clone();
        let max = f32::from(handle.max_offset().y).abs();
        let painted = -f32::from(handle.offset().y);
        // Preserve existing propagation at an edge.
        if (painted + distance).clamp(0., max) == painted {
            self.stop_wheel_motion();
            return;
        }
        if self.motion.trajectory.retarget(now, painted, distance, max) {
            self.request_motion_frame(window, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn request_motion_frame(&mut self, window: &Window, cx: &Context<Self>) {
        if self.motion.pending.is_some() {
            return;
        }
        let generation = self.motion.generation;
        let owner = window.window_handle().window_id();
        self.motion.pending = Some(generation);
        self.motion
            .last_frame
            .get_or_insert_with(|| cx.background_executor().now());
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |view, cx| {
                if view.motion.owner != Some(owner) || view.motion.pending != Some(generation) {
                    return;
                }
                view.motion.pending = None;
                if view.motion.generation != generation {
                    // The old clock belongs to the cancelled request. Even if
                    // new demand arrived before it drained, never carry idle
                    // time into the next gesture's first frame.
                    view.motion.last_frame = None;
                    if view.motion.trajectory.active() || view.marquee_needs_scroll() {
                        view.request_motion_frame(window, cx);
                    }
                    return;
                }
                if !window.is_window_active() || !window.is_visible() {
                    view.cancel_scroll_motion();
                    return;
                }
                let now = cx.background_executor().now();
                let elapsed = view
                    .motion
                    .last_frame
                    .replace(now)
                    .map(|last| now.saturating_duration_since(last))
                    .unwrap_or_default();
                if !view.smooth_scroll_enabled(cx) {
                    view.stop_wheel_motion();
                }
                let handle = view.scroll.0.borrow().base_handle.clone();
                let max = f32::from(handle.max_offset().y).abs();
                let mut changed = false;
                if let Some(offset) = view.motion.trajectory.step(now, max) {
                    handle.set_offset(point(handle.offset().x, px(-offset)));
                    view.motion.unpainted_step.set(true);
                    changed = true;
                }
                let marquee_moved = view.step_marquee_scroll(elapsed);
                changed |= marquee_moved;
                if changed {
                    cx.notify();
                }
                if view.motion.trajectory.active() || view.marquee_needs_scroll() {
                    view.request_motion_frame(window, cx);
                } else {
                    view.motion.last_frame = None;
                }
            });
        });
    }

    /// Painted before the virtual list so later occluding content excludes it
    /// from scroll hit testing. It has no layout, worker, or timer of its own.
    pub(super) fn motion_capture(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scroll = self.scroll.clone();
        let painted = self.motion.painted.clone();
        let unpainted_step = self.motion.unpainted_step.clone();
        let bounds = self.motion.bounds.clone();
        let view = cx.entity().downgrade();
        let on_down = cx.listener(|view, _: &MouseDownEvent, _, _| view.stop_wheel_motion());
        let on_scroll = cx.listener(|view, input: &(ScrollEvent, Pixels), window, cx| {
            view.capture_scroll(&input.0, input.1, window, cx);
        });
        canvas(
            move |b, window, _| {
                let previous = bounds.replace(b);
                (
                    window.insert_hitbox(b, HitboxBehavior::Normal),
                    previous != b,
                )
            },
            move |_, (hitbox, resized), window, cx| {
                // This records the very same offset used to build this draw's rows.
                painted.set(-f32::from(scroll.0.borrow().base_handle.offset().y));
                unpainted_step.set(false);
                if resized {
                    let _ = view.update(cx, |view, _| view.cancel_scroll_motion());
                }
                let line_height = window.line_height();
                window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture && hitbox.should_handle_scroll(window)
                    {
                        on_scroll(&(event.clone(), line_height), window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        on_down(event, window, cx);
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }
}
