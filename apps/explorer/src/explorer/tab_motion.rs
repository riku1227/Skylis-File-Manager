use super::*;
use crate::motion::spring;
use gpui::{EntityId, Pixels, RenderImage, SharedString, SpringState, img};
use std::{sync::Arc, time::Instant};

const TAB_WIDTH: f32 = 180.;
const TAB_PITCH: f32 = TAB_WIDTH + 4.;
const MAX_CLOSING_TABS: usize = 8;

#[derive(Default)]
pub(super) struct TabStripMotion {
    slots: Vec<Slot>,
    reveal: Option<EntityId>,
}

struct Slot {
    id: EntityId,
    width: Option<WidthMotion>,
    face: Option<TabFace>,
    closing: bool,
    exit_opacity: f32,
}

struct WidthMotion {
    spring: SpringState,
    painted_at: Option<Instant>,
}

#[derive(Clone)]
struct TabFace {
    label: SharedString,
    image: Option<Arc<RenderImage>>,
    icon_path: Option<&'static str>,
    active: bool,
}

impl Slot {
    fn new(id: EntityId) -> Self {
        Self {
            id,
            width: None,
            face: None,
            closing: false,
            exit_opacity: 1.,
        }
    }

    fn fraction(&self) -> f32 {
        self.width
            .as_ref()
            .map_or(1., |width| width.spring.position.clamp(0., 1.))
    }

    fn opacity(&self) -> f32 {
        let fraction = self.fraction();
        if self.closing {
            ((fraction - 0.4) / 0.6).clamp(0., self.exit_opacity)
        } else {
            ((fraction - 0.5) / 0.5).clamp(0., 1.)
        }
    }

    fn pitch(&self, snap: &impl Fn(Pixels) -> Pixels) -> Pixels {
        snap(px(TAB_PITCH * self.fraction()))
    }
}

impl TabStripMotion {
    pub(super) fn new(ids: impl IntoIterator<Item = EntityId>) -> Self {
        Self {
            slots: ids.into_iter().map(Slot::new).collect(),
            reveal: None,
        }
    }

    fn index_before_live(&self, index: usize) -> usize {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| !slot.closing)
            .nth(index)
            .map_or(self.slots.len(), |(index, _)| index)
    }

    /// Moving a tab between positions/windows does not create a closing copy.
    pub(super) fn insert(&mut self, index: usize, id: EntityId) {
        self.slots
            .insert(self.index_before_live(index), Slot::new(id));
    }

    pub(super) fn open(&mut self, index: usize, id: EntityId, reduce_motion: bool) {
        let mut slot = Slot::new(id);
        if !reduce_motion {
            slot.width = Some(WidthMotion {
                spring: SpringState {
                    position: 0.,
                    velocity: 0.,
                },
                painted_at: None,
            });
        }
        self.slots.insert(self.index_before_live(index), slot);
        self.reveal = Some(id);
    }

    pub(super) fn remove(&mut self, id: EntityId) {
        self.slots.retain(|slot| slot.id != id);
        if self.reveal == Some(id) {
            self.reveal = None;
        }
    }

    pub(super) fn close(&mut self, id: EntityId, reduce_motion: bool) {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) else {
            return;
        };
        if reduce_motion || slot.face.is_none() || slot.fraction() == 0. {
            self.remove(id);
            return;
        }
        // Closing while still entering reverses the width from the last painted
        // position and velocity. Its face can only get dimmer.
        slot.exit_opacity = slot.opacity();
        slot.closing = true;
        let width = slot.width.get_or_insert(WidthMotion {
            spring: SpringState {
                position: 1.,
                velocity: 0.,
            },
            painted_at: None,
        });
        width.painted_at = None;
        let closing_count = self.slots.iter().filter(|slot| slot.closing).count();
        if closing_count > MAX_CLOSING_TABS
            && let Some(old) = self
                .slots
                .iter()
                .position(|slot| slot.closing && slot.id != id)
        {
            self.slots.remove(old);
        }
    }

    fn advance(&mut self, now: Instant, reduce_motion: bool) -> bool {
        let mut moving = false;
        self.slots.retain_mut(|slot| {
            let Some(width) = &mut slot.width else {
                return true;
            };
            if !reduce_motion {
                let config = if slot.closing {
                    spring::SNAPPY
                } else {
                    spring::BRISK
                };
                let target = if slot.closing { 0. } else { 1. };
                let elapsed = width
                    .painted_at
                    .replace(now)
                    .map_or(0., |last| now.saturating_duration_since(last).as_secs_f32());
                width.spring = config.step(width.spring, target, elapsed);
                if !config.is_settled(width.spring, target, 0.02) {
                    moving = true;
                    return true;
                }
            }
            slot.width = None;
            !slot.closing
        });
        moving
    }

    pub(super) fn drop_slot(&self, x: Pixels, window: &Window) -> usize {
        self.insertion_at(x, |width| window.pixel_snap(width))
    }

    pub(super) fn live_tab_at(&self, x: Pixels, window: &Window) -> Option<EntityId> {
        let snap = |width| window.pixel_snap(width);
        let mut offset = px(0.);
        for slot in &self.slots {
            let edge = offset + slot.pitch(&snap);
            if x >= offset && x < edge {
                // A folded tab is clipped to its visible face; its trailing gap is inert.
                return (!slot.closing && x < offset + snap(px(TAB_WIDTH * slot.fraction())))
                    .then_some(slot.id);
            }
            offset = edge;
        }
        None
    }

    fn insertion_at(&self, x: Pixels, snap: impl Fn(Pixels) -> Pixels) -> usize {
        let mut offset = px(0.);
        let mut live = 0;
        for slot in &self.slots {
            let pitch = slot.pitch(&snap);
            if !slot.closing {
                if x < offset + pitch / 2. {
                    return live;
                }
                live += 1;
            }
            offset += pitch;
        }
        live
    }
}

impl Explorer {
    pub(super) fn render_tab_strip(
        &mut self,
        t: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let motion = &mut self.tab_motion;
        if motion.advance(cx.background_executor().now(), cx.reduce_motion()) {
            window.request_animation_frame();
        }
        let snap = |width| window.pixel_snap(width);
        let width = motion
            .slots
            .iter()
            .map(|slot| slot.pitch(&snap))
            .sum::<Pixels>();
        if !cx.has_active_drag()
            && let Some(id) = motion.reveal.take()
            && self.tabs[self.active].entity_id() == id
            && let Some(index) = motion
                .slots
                .iter()
                .position(|slot| slot.id == id && !slot.closing)
        {
            self.tab_scroll.scroll_to_item(index + 1); // The measurement canvas is child zero.
            if motion.slots[index].width.is_some() {
                motion.reveal = Some(id);
            }
        }
        let mut strip = div()
            .id("tabs")
            .debug_selector(|| "tab-strip".into())
            .relative()
            .flex()
            .items_center()
            .min_w_0()
            .w(width)
            .h(px(28.))
            .max_w_full()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll)
            .on_scroll(cx.listener(|this, _, _, _| this.tab_motion.reveal = None))
            .child(
                gpui::canvas(
                    {
                        let bounds = self.tab_strip.clone();
                        let scroll = self.tab_scroll.clone();
                        move |rect, _, _| {
                            bounds.set(gpui::Bounds::new(rect.origin - scroll.offset(), rect.size));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let mut live_tabs = self.tabs.iter().enumerate();
        let mut offset = px(0.);
        let mut drop_left = None;
        for slot in &mut motion.slots {
            let id = slot.id;
            let face = if slot.closing {
                let Some(face) = slot.face.clone() else {
                    continue;
                };
                tab_face(("closing-tab", id), face, t)
                    .debug_selector(|| "closing-tab-face".into())
                    .child(close_mark(t))
            } else {
                let (index, tab) = live_tabs.next().expect("each live slot owns a tab");
                let live_id = tab.entity_id();
                debug_assert_eq!(live_id, id);
                if self.drop_slot == Some(index) {
                    drop_left = Some(offset);
                }
                let tab = tab.read(cx);
                let label = tab.label().into();
                let path = tab.path().map(std::path::Path::to_path_buf);
                let icon_path = path.is_none().then(|| {
                    if tab.is_archive() {
                        "icons/file.svg"
                    } else {
                        "icons/home.svg"
                    }
                });
                let face = TabFace {
                    label,
                    image: path.as_deref().and_then(|path| {
                        self.icons.update(cx, |icons, cx| {
                            icons.get_navigation(
                                path,
                                true,
                                (14. * window.scale_factor()).ceil() as u32,
                                cx,
                            )
                        })
                    }),
                    icon_path,
                    active: index == self.active,
                };
                slot.face = Some(face.clone());
                live_tab_face(live_id, face, self.mode, t, cx)
            };
            let pitch = slot.pitch(&snap);
            offset += pitch;
            let shell = div().w(pitch).h(px(28.)).flex_shrink_0();
            strip = strip.child(if slot.width.is_some() {
                shell.child(
                    div()
                        .relative()
                        .w(snap(px(TAB_WIDTH * slot.fraction())))
                        .h_full()
                        .overflow_hidden()
                        .when(slot.closing, |d| d.block_mouse_except_scroll())
                        .child(face.opacity(slot.opacity()).absolute().top_0().left_0()),
                )
            } else {
                shell.child(face)
            });
        }
        debug_assert!(live_tabs.next().is_none());
        if self.drop_slot.is_some() {
            strip = strip.child(
                div()
                    .absolute()
                    .left(drop_left.unwrap_or(offset))
                    .top(px(2.))
                    .w(px(2.))
                    .h(px(24.))
                    .bg(t.c("focus.ring")),
            );
        }
        strip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn face() -> TabFace {
        TabFace {
            label: "Tab".into(),
            image: None,
            icon_path: None,
            active: true,
        }
    }

    #[test]
    fn a_partially_opened_tab_closes_from_its_painted_width_and_opacity() {
        let now = Instant::now();
        let first = 1.into();
        let second = 2.into();
        let mut strip = TabStripMotion::new([first]);
        strip.open(1, second, false);
        assert!(strip.advance(now, false));
        assert_eq!(strip.slots[1].fraction(), 0.);
        let early = now + Duration::from_millis(16);
        assert!(strip.advance(early, false));
        assert!(strip.slots[1].fraction() > 0.);
        assert_eq!(strip.slots[1].opacity(), 0.);
        let middle = now + Duration::from_millis(40);
        assert!(strip.advance(middle, false));
        let slot = &mut strip.slots[1];
        slot.face = Some(face());
        let (fraction, opacity) = (slot.fraction(), slot.opacity());
        assert!(fraction > 0.5 && fraction < 1. && opacity > 0. && opacity < 1.);
        strip.close(second, false);
        assert!(strip.advance(middle, false));
        assert!(strip.slots[1].closing);
        assert_eq!(strip.slots[1].fraction(), fraction);
        assert_eq!(strip.slots[1].opacity(), opacity);
        assert!(!strip.advance(middle + Duration::from_millis(180), false));
        assert_eq!(strip.slots.len(), 1);
    }

    #[test]
    fn drag_slots_follow_painted_widths_and_do_not_count_closing_tabs() {
        let first = 1.into();
        let second = 2.into();
        let third = 3.into();
        let mut strip = TabStripMotion::new([first, second, third]);
        assert_eq!(strip.insertion_at(px(90.), core::convert::identity), 0);
        assert_eq!(strip.insertion_at(px(94.), core::convert::identity), 1);
        assert_eq!(strip.insertion_at(px(368.), core::convert::identity), 2);
        strip.slots[1].face = Some(face());
        strip.close(second, false);
        let now = Instant::now();
        strip.advance(now, false);
        strip.advance(now + Duration::from_millis(22), false);
        let third_start = strip.slots[0].pitch(&core::convert::identity)
            + strip.slots[1].pitch(&core::convert::identity);
        assert!(third_start > px(184.) && third_start < px(368.));
        assert_eq!(
            strip.insertion_at(third_start + px(91.), core::convert::identity),
            1
        );
        assert_eq!(
            strip.insertion_at(third_start + px(93.), core::convert::identity),
            2
        );
        strip.open(1, 4.into(), false);
        assert!(!strip.advance(now, true));
        assert_eq!(
            strip.slots.iter().map(|slot| slot.id).collect::<Vec<_>>(),
            [first, 4.into(), third]
        );
        assert!(strip.slots.iter().all(|slot| slot.fraction() == 1.));
    }
}

fn tab_face(id: (&'static str, EntityId), face: TabFace, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.))
        .w(px(TAB_WIDTH))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.c(if face.active {
            "tab.active.border"
        } else {
            "tab.border"
        }))
        .bg(t.c(if face.active {
            "tab.active.bg"
        } else {
            "tab.bg"
        }))
        .text_color(t.c(if face.active {
            "text.primary"
        } else {
            "tab.fg"
        }))
        .when(face.active, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
        .child(
            div()
                .size(px(14.))
                .flex_shrink_0()
                .when_some(face.image, |d, image| d.child(img(image).size_full()))
                .when_some(face.icon_path, |d, path| {
                    d.child(svg().path(path).size_full().text_color(t.c("icon.default")))
                }),
        )
        .child(div().flex_1().truncate().child(face.label))
}

fn close_mark(t: Theme) -> gpui::Div {
    div()
        .size(px(18.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .child(
            svg()
                .path("icons/x.svg")
                .size(px(12.))
                .text_color(t.c("text.secondary")),
        )
}

fn live_tab_face(
    id: EntityId,
    face: TabFace,
    mode: ThemeMode,
    t: Theme,
    cx: &Context<Explorer>,
) -> gpui::Stateful<gpui::Div> {
    let owner = cx.weak_entity();
    let drag = TabDrag {
        id,
        label: face.label.clone(),
        mode,
    };
    let active = face.active;
    tab_face(("tab", id), face, t)
        .debug_selector(move || format!("tab-{id}"))
        .on_drag(drag, move |drag, _, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.dragging_tab = Some(drag.id);
                this.tab_menu = None;
                cx.notify();
            });
            cx.new(|_| drag.clone())
        })
        .on_mouse_down(
            gpui::MouseButton::Middle,
            cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                if let Some(index) = this.tabs.iter().position(|tab| tab.entity_id() == id) {
                    this.close(index, w, cx);
                }
            }),
        )
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.tab_menu = Some((id, event.position));
                cx.notify();
            }),
        )
        .when(!active, |d| d.hover(|s| s.bg(t.c("tab.hover.bg"))))
        .cursor_pointer()
        .child(
            close_mark(t)
                .id(("close-tab", id))
                .hover(|s| s.bg(t.c("state.pressed")))
                .on_click(cx.listener(move |this, _, w, cx| {
                    cx.stop_propagation();
                    if let Some(index) = this.tabs.iter().position(|tab| tab.entity_id() == id) {
                        this.close(index, w, cx);
                    }
                })),
        )
        .on_click(cx.listener(move |this, _, w, cx| {
            if let Some(index) = this.tabs.iter().position(|tab| tab.entity_id() == id) {
                this.switch(index, w, cx);
            }
        }))
}
