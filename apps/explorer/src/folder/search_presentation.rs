//! Visible-row decoration only. Matching and filesystem work stay unchanged.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn name_text(name: &str, ranges: Vec<Range<usize>>, t: Theme) -> AnyElement {
    if ranges.is_empty() {
        return name.to_owned().into_any_element();
    }
    let style = gpui::HighlightStyle {
        background_color: Some(t.c("match.bg").into()),
        color: Some(t.c("match.fg").into()),
        ..Default::default()
    };
    gpui::StyledText::new(name.to_owned())
        .with_highlights(
            cluster_ranges(name, &ranges)
                .into_iter()
                .map(|r| (r, style)),
        )
        .into_any_element()
}

// A combining mark or part of a ZWJ emoji must not split a displayed cluster.
fn cluster_ranges(name: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    let mut index = 0;
    for (start, g) in name.grapheme_indices(true) {
        let end = start + g.len();
        while ranges.get(index).is_some_and(|r| r.end <= start) {
            index += 1;
        }
        if ranges.get(index).is_some_and(|r| r.start < end) {
            if let Some(last) = result.last_mut()
                && last.end == start
            {
                last.end = end;
            } else {
                result.push(start..end);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_expand_to_whole_display_clusters() {
        assert_eq!(cluster_ranges("e\u{301}.txt", &[0..1]), vec![0..3]);
        let emoji = "👩\u{200d}💻";
        assert_eq!(cluster_ranges(emoji, &[7..11]), vec![0..11]);
        assert_eq!(cluster_ranges("a_b", &[0..1, 2..3]), vec![0..1, 2..3]);
    }
}

impl FolderView {
    pub(super) fn type_icon(
        &self,
        path: &Path,
        dir: bool,
        logical: f32,
        t: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let physical = (logical * window.scale_factor()).ceil() as u32;
        let image = self
            .icons
            .update(cx, |icons, _| icons.get_type(path, dir, physical));
        let generic = image.is_none();
        div()
            .size(px(logical))
            .flex_shrink_0()
            .when(generic, |d| {
                d.child(
                    svg()
                        .path(if dir {
                            "icons/folder.svg"
                        } else {
                            "icons/file.svg"
                        })
                        .size_full()
                        .text_color(t.c("text.muted")),
                )
            })
            .when_some(image, |d, image| d.child(img(image).size_full()))
            .into_any_element()
    }
}
