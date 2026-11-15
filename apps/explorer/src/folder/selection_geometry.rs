//! Content-space geometry for virtualized rubber-band selection.
//!
//! Candidate rows are calculated directly from the drag rectangle. Callers only
//! inspect those rows, then intersect each actual tile to exclude gaps/headers.
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub(super) fn between(start: (f32, f32), end: (f32, f32)) -> Self {
        if ![start.0, start.1, end.0, end.1]
            .into_iter()
            .all(f32::is_finite)
        {
            return Self::default();
        }
        Self {
            x: start.0.min(end.0),
            y: start.1.min(end.1),
            width: (end.0 - start.0).abs(),
            height: (end.1 - start.1).abs(),
        }
    }

    /// Merely touching an edge does not select the neighbouring item.
    pub(super) fn intersects(self, other: Self) -> bool {
        self.has_area()
            && other.has_area()
            && self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    fn has_area(self) -> bool {
        self.width > 0.
            && self.height > 0.
            && [self.x, self.y, self.x + self.width, self.y + self.height]
                .into_iter()
                .all(f32::is_finite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Layout {
    /// Width of the scrollable content, excluding the scrollbar gutter.
    pub width: f32,
    pub row_height: f32,
    pub top_padding: f32,
    pub columns: usize,
    pub gallery: bool,
    pub tile_height: f32,
}

impl Layout {
    pub(super) fn item_rect(self, row: usize, column: usize) -> Rect {
        if !self.width.is_finite()
            || !self.row_height.is_finite()
            || self.width <= 0.
            || self.row_height <= 0.
        {
            return Rect::default();
        }
        if self.gallery {
            if self.columns == 0 || column >= self.columns || !self.tile_height.is_finite() {
                return Rect::default();
            }
            let tile_width =
                ((self.width - 20. - 8. * (self.columns - 1) as f32) / self.columns as f32).max(0.);
            Rect {
                x: 10. + column as f32 * (tile_width + 8.),
                y: row as f32 * self.row_height + 22.,
                width: tile_width,
                height: self.tile_height.max(0.),
            }
        } else {
            if !self.top_padding.is_finite() || column != 0 {
                return Rect::default();
            }
            Rect {
                x: 12.,
                y: self.top_padding + row as f32 * self.row_height,
                width: (self.width - 24.).max(0.),
                height: self.row_height,
            }
        }
    }

    /// A bounded candidate range, independent of the total number of entries.
    /// Gallery row headers and gaps are excluded by `item_rect().intersects()`.
    pub(super) fn rows_in(self, rect: Rect, row_count: usize) -> Range<usize> {
        let padding = if self.gallery { 0. } else { self.top_padding };
        if !rect.has_area()
            || row_count == 0
            || !self.row_height.is_finite()
            || self.row_height <= 0.
            || !padding.is_finite()
            || !self.width.is_finite()
            || self.width <= 0.
            || rect.x >= self.width
            || rect.x + rect.width <= 0.
        {
            return 0..0;
        }
        let start = ((rect.y - padding) / self.row_height).floor().max(0.) as usize;
        let end = ((rect.y + rect.height - padding) / self.row_height)
            .ceil()
            .max(0.) as usize;
        start.min(row_count)..end.min(row_count)
    }
}

/// Signed pixels per second. The 24px edge ramp reaches 1125px/s
/// and stays bounded when the cursor is outside the viewport.
pub(super) fn edge_scroll(pointer_y: f32, viewport_height: f32) -> f32 {
    if !pointer_y.is_finite() || !viewport_height.is_finite() || viewport_height <= 0. {
        return 0.;
    }
    let edge = 24_f32.min(viewport_height / 2.);
    if pointer_y < edge {
        -1125. * ((edge - pointer_y) / edge).clamp(0., 1.)
    } else if pointer_y > viewport_height - edge {
        1125. * ((pointer_y - (viewport_height - edge)) / edge).clamp(0., 1.)
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Layout {
        Layout {
            width: 500.,
            row_height: 28.,
            top_padding: 2.,
            columns: 1,
            gallery: false,
            tile_height: 0.,
        }
    }

    fn gallery() -> Layout {
        Layout {
            width: 420.,
            row_height: 172.,
            top_padding: 0.,
            columns: 3,
            gallery: true,
            tile_height: 142.,
        }
    }

    #[test]
    fn rectangles_work_in_every_drag_direction_and_require_positive_overlap() {
        let expected = Rect {
            x: 10.,
            y: 20.,
            width: 60.,
            height: 80.,
        };
        for (start, end) in [
            ((10., 20.), (70., 100.)),
            ((70., 100.), (10., 20.)),
            ((70., 20.), (10., 100.)),
            ((10., 100.), (70., 20.)),
        ] {
            assert_eq!(Rect::between(start, end), expected);
        }
        assert!(!expected.intersects(Rect::between((70., 20.), (90., 100.))));
        assert!(!expected.intersects(Rect::between((10., 100.), (70., 110.))));
        assert!(!expected.intersects(Rect::between((30., 30.), (30., 60.))));
        assert!(expected.intersects(Rect::between((69., 99.), (90., 110.))));
        assert!(!expected.intersects(Rect::between((f32::NAN, 0.), (20., 30.))));
    }

    #[test]
    fn list_gutters_padding_and_row_boundaries_are_not_selected() {
        let layout = list();
        assert_eq!(
            layout.item_rect(1, 0),
            Rect {
                x: 12.,
                y: 30.,
                width: 476.,
                height: 28.
            }
        );
        for drag in [
            Rect::between((0., 2.), (12., 30.)),
            Rect::between((488., 2.), (500., 30.)),
            Rect::between((0., 0.), (500., 2.)),
        ] {
            assert!(!layout.item_rect(0, 0).intersects(drag));
        }
        assert_eq!(
            layout.rows_in(Rect::between((20., 30.), (50., 58.)), 100),
            1..2
        );
        assert_eq!(
            layout.rows_in(Rect::between((20., -50.), (50., 2.)), 100),
            0..0
        );
    }

    #[test]
    fn gallery_only_hits_intersected_columns_and_skips_headers_and_gaps() {
        let layout = gallery();
        // Tiles are 128px wide with 8px gaps: 10..138, 146..274, 282..410.
        let rectangle = Rect::between((150., 24.), (273., 160.));
        let hit_columns: Vec<_> = (0..3)
            .filter(|&column| layout.item_rect(0, column).intersects(rectangle))
            .collect();
        assert_eq!(hit_columns, vec![1]);
        for gap in [
            Rect::between((10., 0.), (410., 22.)),
            Rect::between((138., 22.), (146., 164.)),
            Rect::between((10., 164.), (410., 194.)),
        ] {
            assert!(layout.rows_in(gap, 10).all(|row| {
                (0..3).all(|column| !layout.item_rect(row, column).intersects(gap))
            }));
        }
    }

    #[test]
    fn candidate_rows_are_constant_sized_even_for_a_large_virtual_list() {
        let layout = list();
        let rectangle = Rect::between((20., 2. + 28. * 50_000.), (100., 2. + 28. * 50_003.));
        assert_eq!(layout.rows_in(rectangle, 1_000_000), 50_000..50_003);
        assert_eq!(layout.rows_in(rectangle, 50_001), 50_000..50_001);
        assert_eq!(layout.rows_in(rectangle, 100), 100..100);
        assert_eq!(layout.rows_in(rectangle, 0), 0..0);
        assert_eq!(
            layout.rows_in(Rect::between((500., 5.), (600., 100.)), 100),
            0..0
        );
    }

    #[test]
    fn compact_and_narrow_layouts_have_safe_item_bounds() {
        let mut layout = gallery();
        layout.row_height = 156.;
        layout.tile_height = 126.;
        assert_eq!(layout.item_rect(2, 1).y, 334.);
        assert_eq!(layout.item_rect(2, 1).height, 126.);
        layout.width = 12.;
        assert_eq!(layout.item_rect(0, 0).width, 0.);
        layout.columns = 0;
        assert_eq!(layout.item_rect(0, 0), Rect::default());
        layout.row_height = 0.;
        assert_eq!(
            layout.rows_in(Rect::between((0., 0.), (10., 10.)), 100),
            0..0
        );
    }

    #[test]
    fn edge_scrolling_is_proportional_directional_and_bounded() {
        for (position, expected) in [
            (-500., -1125.),
            (0., -1125.),
            (12., -562.5),
            (24., 0.),
            (300., 0.),
            (576., 0.),
            (588., 562.5),
            (600., 1125.),
            (900., 1125.),
        ] {
            assert_eq!(edge_scroll(position, 600.), expected);
        }
        assert_eq!(edge_scroll(10., 20.), 0.);
        assert_eq!(edge_scroll(0., 20.), -1125.);
        assert_eq!(edge_scroll(20., 20.), 1125.);
        assert_eq!(edge_scroll(1., 0.), 0.);
        assert_eq!(edge_scroll(f32::NAN, 100.), 0.);
    }
}
