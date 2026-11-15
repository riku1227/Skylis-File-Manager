use super::*;
use explorer_core::date_group::{self, DateGroup};

#[derive(Clone, Copy)]
pub(super) enum DisplayRow { Header(DateGroup, usize), Entry(usize) }

impl FolderView {
    pub(super) fn group_for(&self, entry: &Entry) -> DateGroup {
        date_group::classify(entry.modified.map(|t| chrono::DateTime::<chrono::Local>::from(t).date_naive()), self.group_date)
    }
    pub(super) fn rebuild_display_rows(&mut self) {
        self.display_rows.clear();
        self.entry_rows.clear();
        self.entry_rows.resize(self.visible.len(), 0);
        let mut header = None;
        let mut previous = None;
        for (ix, &i) in self.visible.iter().enumerate() {
            if self.grouped {
                let group = self.group_for(&self.entries[i]);
                if previous != Some(group) {
                    header = Some(self.display_rows.len());
                    self.display_rows.push(DisplayRow::Header(group, 0));
                    previous = Some(group);
                }
                if let Some(DisplayRow::Header(_, count)) = header.map(|h| &mut self.display_rows[h]) { *count += 1; }
            }
            self.entry_rows[ix] = self.display_rows.len();
            self.display_rows.push(DisplayRow::Entry(ix));
        }
    }
    pub(super) fn scroll_to_entry(&self, ix: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(self.entry_rows.get(ix).copied().unwrap_or(ix), strategy);
    }
    pub(super) fn apply_grouping(&mut self) {
        if self.grouped {
            let entries = &self.entries;
            // Chronological order includes directories; no folder-first partition within a timeline.
            self.visible.sort_by(|&a, &b| entries[b].modified.cmp(&entries[a].modified)
                .then_with(|| entries[a].name.cmp(&entries[b].name)));
        }
        self.rebuild_display_rows();
    }
}
