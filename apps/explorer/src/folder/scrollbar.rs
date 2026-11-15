use super::*;
use gpui::{Bounds, Point, canvas, fill, point, size};

pub(super) fn thumb_geometry(viewport: f32, max_scroll: f32, offset: f32) -> (f32, f32) {
    if viewport <= 0. || max_scroll <= 0. {
        return (0., viewport.max(0.));
    }
    let height = (viewport * viewport / (viewport + max_scroll))
        .max(24.)
        .min(viewport);
    let top = (offset / max_scroll).clamp(0., 1.) * (viewport - height);
    (top, height)
}

impl FolderView {
    pub(super) fn render_scrollbar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let scroll = self.scroll.clone();
        div()
            .id("list-scrollbar")
            .w(px(12.))
            .h_full()
            .flex_shrink_0()
            .cursor_default()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.cancel_scroll_motion();
                    let handle = this.scroll.0.borrow().base_handle.clone();
                    let bounds = handle.bounds();
                    let viewport = f32::from(bounds.size.height);
                    let max_scroll = f32::from(handle.max_offset().y).abs();
                    if max_scroll <= 0. {
                        return;
                    }
                    let offset = -f32::from(handle.offset().y);
                    let local = f32::from(event.position.y - bounds.origin.y);
                    let (top, height) = thumb_geometry(viewport, max_scroll, offset);
                    if local >= top && local <= top + height {
                        this.scrollbar_drag = Some(local - top);
                    } else if event.modifiers.shift {
                        this.scrollbar_drag = Some(height / 2.);
                        this.drag_scrollbar(event.position, cx);
                    } else {
                        let next = (offset + if local < top { -viewport } else { viewport })
                            .clamp(0., max_scroll);
                        handle.set_offset(point(handle.offset().x, px(-next)));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        let handle = scroll.0.borrow().base_handle.clone();
                        let max_scroll = f32::from(handle.max_offset().y).abs();
                        if max_scroll > 0. {
                            let (top, height) = thumb_geometry(
                                f32::from(bounds.size.height),
                                max_scroll,
                                -f32::from(handle.offset().y),
                            );
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.origin.x + px(3.), bounds.origin.y + px(top)),
                                    size(px(6.), px(height)),
                                ),
                                t.c("text.muted"),
                            ));
                        }
                    },
                )
                .size_full(),
            )
    }
    pub(super) fn drag_scrollbar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(grab) = self.scrollbar_drag else {
            return;
        };
        let handle = self.scroll.0.borrow().base_handle.clone();
        let bounds = handle.bounds();
        let viewport = f32::from(bounds.size.height);
        let max_scroll = f32::from(handle.max_offset().y).abs();
        let (_, height) = thumb_geometry(viewport, max_scroll, 0.);
        if viewport > height {
            let ratio = ((f32::from(position.y - bounds.origin.y) - grab) / (viewport - height))
                .clamp(0., 1.);
            handle.set_offset(point(handle.offset().x, px(-ratio * max_scroll)));
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumb_is_bounded_and_maps_both_endpoints() {
        assert_eq!(thumb_geometry(100., 0., 0.), (0., 100.));
        assert_eq!(thumb_geometry(100., 900., 0.), (0., 24.));
        assert_eq!(thumb_geometry(100., 900., 900.), (76., 24.));
        assert_eq!(thumb_geometry(100., 900., 1800.), (76., 24.));
        assert_eq!(thumb_geometry(10., 900., 900.), (0., 10.));
    }
}
