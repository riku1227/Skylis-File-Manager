//! Rectangle selection is a view overlay, never a file drag. Per-frame selection
//! is evaluated only for rendered items; paths are materialized once on release.
use super::selection_geometry::{Layout, Rect, edge_scroll};
use super::*;
use gpui::{Bounds, Point, canvas, fill, point, size};
use std::{cell::Cell, collections::BTreeSet, rc::Rc, time::Duration};

#[derive(Clone, Copy)]
enum Mode {
    Replace,
    Add,
    Toggle,
}
impl Mode {
    fn selected(self, baseline: bool, hit: bool) -> bool {
        match self {
            Self::Replace => hit,
            Self::Add => baseline || hit,
            Self::Toggle => baseline ^ hit,
        }
    }
}
pub(super) struct Marquee {
    start: (f32, f32),
    pointer: Point<Pixels>,
    start_pointer: Point<Pixels>,
    viewport: Bounds<Pixels>,
    layout: Layout,
    mode: Mode,
    baseline: BTreeSet<PathBuf>,
    dragged: bool,
}
#[derive(Default)]
pub(super) struct MarqueeState {
    gesture: Option<Marquee>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}
impl FolderView {
    fn marquee_layout(&self) -> Layout {
        Layout {
            width: f32::from(self.marquee.bounds.get().size.width),
            row_height: if self.gallery {
                if self.compact { 156. } else { 172. }
            } else if self.compact {
                24.
            } else {
                28.
            },
            top_padding: if self.gallery { 0. } else { 2. },
            columns: self.gallery_columns,
            gallery: self.gallery,
            tile_height: if self.compact { 126. } else { 142. },
        }
    }
    fn marquee_offset(&self) -> f32 {
        -f32::from(self.scroll.0.borrow().base_handle.offset().y)
    }
    fn marquee_rect(&self, m: &Marquee) -> Rect {
        let x = f32::from(m.pointer.x - m.viewport.origin.x).clamp(0., m.layout.width);
        let y = f32::from(m.pointer.y - m.viewport.origin.y)
            .clamp(0., f32::from(m.viewport.size.height))
            + self.marquee_offset();
        Rect::between(m.start, (x, y))
    }
    fn marquee_item_rect(&self, ix: usize, layout: Layout) -> Option<Rect> {
        let row = *self.entry_rows.get(ix)?;
        let col = if self.gallery {
            self.gallery_rows
                .get(row)?
                .1
                .iter()
                .position(|&i| i == ix)?
        } else {
            0
        };
        Some(layout.item_rect(row, col))
    }
    pub(super) fn item_is_selected(&self, ix: usize) -> bool {
        let path = &self.entries[self.visible[ix]].path;
        if let Some(m) = &self.marquee.gesture
            && m.dragged
        {
            let hit = self
                .marquee_item_rect(ix, m.layout)
                .is_some_and(|item| item.intersects(self.marquee_rect(m)));
            return m.mode.selected(m.baseline.contains(path), hit);
        }
        self.multi_selection.contains(path)
    }
    pub(super) fn cancel_marquee_if_layout_changed(&mut self) -> bool {
        if self.marquee.gesture.as_ref().is_some_and(|m| {
            m.layout != self.marquee_layout() || m.viewport != self.marquee.bounds.get()
        }) {
            return self.cancel_marquee();
        }
        false
    }
    pub(super) fn cancel_marquee(&mut self) -> bool {
        // Selection is not mutated while dragging, so cancellation restores it for free.
        self.marquee.gesture.take().is_some()
    }
    fn start_marquee(&mut self, event: &MouseDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.preview_open || self.context_menu.is_some() || self.is_home() {
            return;
        }
        let viewport = self.marquee.bounds.get();
        if !viewport.contains(&event.position) {
            return;
        }
        self.cancel_marquee();
        w.focus(&self.focus, cx);
        let layout = self.marquee_layout();
        let start = (
            f32::from(event.position.x - viewport.origin.x),
            f32::from(event.position.y - viewport.origin.y) + self.marquee_offset(),
        );
        // Defensive hit-test: interactive children stop propagation, but decorations
        // and padding must never accidentally turn a file drag into a rectangle.
        let probe = Rect {
            x: start.0,
            y: start.1,
            width: 0.1,
            height: 0.1,
        };
        if self.marquee_hits(probe, layout).next().is_some() {
            return;
        }
        let mode = if event.modifiers.control || event.modifiers.platform {
            Mode::Toggle
        } else if event.modifiers.shift {
            Mode::Add
        } else {
            Mode::Replace
        };
        self.marquee.gesture = Some(Marquee {
            start,
            pointer: event.position,
            start_pointer: event.position,
            viewport,
            layout,
            mode,
            baseline: self.multi_selection.clone(),
            dragged: false,
        });
        self.preview_scope.cancel();
        cx.stop_propagation();
        cx.notify();
    }
    // Only candidate virtual rows are visited, including offscreen ones on commit.
    fn marquee_hits(&self, rect: Rect, layout: Layout) -> impl Iterator<Item = usize> + '_ {
        let count = if self.gallery {
            self.gallery_rows.len()
        } else {
            self.display_rows.len()
        };
        layout.rows_in(rect, count).flat_map(move |row| {
            let indices: &[usize] = if self.gallery {
                &self.gallery_rows[row].1
            } else {
                match &self.display_rows[row] {
                    DisplayRow::Entry(ix) => std::slice::from_ref(ix),
                    _ => &[],
                }
            };
            indices.iter().enumerate().filter_map(move |(col, &ix)| {
                layout.item_rect(row, col).intersects(rect).then_some(ix)
            })
        })
    }
    pub(super) fn move_marquee(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marquee.gesture.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_marquee(cx);
            return;
        }
        let layout = self.marquee_layout();
        let m = self.marquee.gesture.as_mut().unwrap();
        if layout != m.layout || self.marquee.bounds.get() != m.viewport {
            self.cancel_marquee();
            cx.notify();
            return;
        }
        m.pointer = event.position;
        let dx = f32::from(m.pointer.x - m.start_pointer.x);
        let dy = f32::from(m.pointer.y - m.start_pointer.y);
        m.dragged |= dx * dx + dy * dy >= 16.;
        let scroll = m.dragged
            && edge_scroll(
                f32::from(m.pointer.y - m.viewport.origin.y),
                f32::from(m.viewport.size.height),
            ) != 0.;
        if scroll {
            self.request_motion_frame(window, cx);
        }
        cx.notify();
    }
    pub(super) fn step_marquee_scroll(&mut self, elapsed: Duration) -> bool {
        let Some(m) = &self.marquee.gesture else {
            return false;
        };
        if !m.dragged
            || m.layout != self.marquee_layout()
            || m.viewport != self.marquee.bounds.get()
        {
            return false;
        }
        let delta = crate::motion::edge_distance(
            edge_scroll(
                f32::from(m.pointer.y - m.viewport.origin.y),
                f32::from(m.viewport.size.height),
            ),
            elapsed,
        );
        let handle = self.scroll.0.borrow().base_handle.clone();
        let old = -f32::from(handle.offset().y);
        let next = (old + delta).clamp(0., f32::from(handle.max_offset().y).abs());
        if next == old {
            return false;
        }
        handle.set_offset(point(handle.offset().x, px(-next)));
        true
    }
    pub(super) fn release_marquee(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(m) = self.marquee.gesture.as_mut() {
            m.pointer = position;
            let dx = f32::from(position.x - m.start_pointer.x);
            let dy = f32::from(position.y - m.start_pointer.y);
            m.dragged |= dx * dx + dy * dy >= 16.;
        }
        self.finish_marquee(cx);
    }
    pub(super) fn finish_marquee(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.marquee.gesture.take() else {
            return;
        };
        if m.layout != self.marquee_layout() || m.viewport != self.marquee.bounds.get() {
            cx.notify();
            return;
        }
        if m.dragged {
            let rect = self.marquee_rect(&m);
            let hits: Vec<_> = self.marquee_hits(rect, m.layout).collect();
            // Include both axes: horizontal gallery drags must focus the tile
            // next to the pointer, with the anchor next to the starting point.
            let nearest = |p: (f32, f32)| {
                hits.iter().copied().min_by(|&a, &b| {
                    let distance = |ix| {
                        let r = self.marquee_item_rect(ix, m.layout).unwrap();
                        let dx = (r.x + r.width / 2.) - p.0;
                        let dy = (r.y + r.height / 2.) - p.1;
                        dx * dx + dy * dy
                    };
                    distance(a).total_cmp(&distance(b))
                })
            };
            let cursor = nearest((
                f32::from(m.pointer.x - m.viewport.origin.x),
                f32::from(m.pointer.y - m.viewport.origin.y) + self.marquee_offset(),
            ));
            let anchor = nearest(m.start);
            self.multi_selection = match m.mode {
                Mode::Replace => BTreeSet::new(),
                _ => m.baseline,
            };
            for &ix in &hits {
                let path = self.entries[self.visible[ix]].path.clone();
                if matches!(m.mode, Mode::Toggle) && self.multi_selection.remove(&path) {
                    continue;
                }
                self.multi_selection.insert(path);
            }
            self.selected = cursor
                .filter(|&ix| {
                    self.multi_selection
                        .contains(&self.entries[self.visible[ix]].path)
                })
                .or_else(|| {
                    self.visible
                        .iter()
                        .position(|&i| self.multi_selection.contains(&self.entries[i].path))
                });
            self.selection_anchor = anchor.map(|ix| self.entries[self.visible[ix]].path.clone());
            self.preview_scope.cancel();
        } else if matches!(m.mode, Mode::Replace) {
            self.multi_selection.clear();
            self.selected = None;
            self.selection_anchor = None;
        }
        cx.notify();
    }
    pub(super) fn marquee_needs_scroll(&self) -> bool {
        let Some(m) = &self.marquee.gesture else {
            return false;
        };
        if !m.dragged
            || m.layout != self.marquee_layout()
            || m.viewport != self.marquee.bounds.get()
        {
            return false;
        }
        let speed = edge_scroll(
            f32::from(m.pointer.y - m.viewport.origin.y),
            f32::from(m.viewport.size.height),
        );
        let handle = self.scroll.0.borrow().base_handle.clone();
        let offset = -f32::from(handle.offset().y);
        let max = f32::from(handle.max_offset().y).abs();
        (speed < 0. && offset > 0.) || (speed > 0. && offset < max)
    }
    pub(super) fn marquee_active(&self) -> bool {
        self.marquee.gesture.as_ref().is_some_and(|m| m.dragged)
    }
    pub(super) fn marquee_in_progress(&self) -> bool {
        self.marquee.gesture.is_some()
    }
    pub(super) fn selection_surface(
        &self,
        content: impl IntoElement,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bounds = self.marquee.bounds.clone();
        let viewport = self.marquee.gesture.as_ref().map(|m| m.viewport);
        let view = cx.entity().downgrade();
        let on_move = cx
            .listener(|view, event: &gpui::MouseMoveEvent, w, cx| view.move_marquee(event, w, cx));
        let rect = self
            .marquee
            .gesture
            .as_ref()
            .filter(|m| m.dragged)
            .map(|m| self.marquee_rect(m));
        let offset = self.marquee_offset();
        div()
            .id("selection-surface")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start_marquee))
            .child(self.motion_capture(cx))
            .child(content)
            .child(
                canvas(
                    move |b, _, cx| {
                        bounds.set(b);
                        // Bounds are finalized after Render, including height-only
                        // changes. Repaint without the gesture even without input.
                        if viewport.is_some_and(|old| old != b) {
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |view, cx| {
                                    if view.cancel_marquee_if_layout_changed() {
                                        cx.notify();
                                    }
                                });
                            });
                        }
                        b
                    },
                    move |b, _, w, _| {
                        // Window-level capture keeps an active selection responsive
                        // when the pointer jumps beyond the folder's root hitbox.
                        // Register before mouse-down too: multiple input events can
                        // arrive before the first active-gesture frame is painted.
                        w.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, w, cx| {
                            if phase == gpui::DispatchPhase::Capture {
                                on_move(event, w, cx);
                            }
                        });
                        if let Some(rect) = rect.filter(|_| viewport == Some(b)) {
                            let x = rect.x.max(0.);
                            let y = (rect.y - offset).max(0.);
                            let right = (rect.x + rect.width).min(f32::from(b.size.width));
                            let bottom =
                                (rect.y + rect.height - offset).min(f32::from(b.size.height));
                            if right > x && bottom > y {
                                let region = Bounds::new(
                                    b.origin + point(px(x), px(y)),
                                    size(px(right - x), px(bottom - y)),
                                );
                                w.paint_quad(fill(region, t.c("selection.bg").opacity(0.35)));
                                let border = t.c("focus.ring");
                                for edge in [
                                    Bounds::new(region.origin, size(region.size.width, px(1.))),
                                    Bounds::new(
                                        point(region.origin.x, region.bottom() - px(1.)),
                                        size(region.size.width, px(1.)),
                                    ),
                                    Bounds::new(region.origin, size(px(1.), region.size.height)),
                                    Bounds::new(
                                        point(region.right() - px(1.), region.origin.y),
                                        size(px(1.), region.size.height),
                                    ),
                                ] {
                                    w.paint_quad(fill(edge, border));
                                }
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
