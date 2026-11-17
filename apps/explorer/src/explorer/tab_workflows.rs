use super::*;

gpui::actions!(explorer, [ReopenClosedTab, DuplicateTab]);
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("ctrl-shift-t", ReopenClosedTab, Some("ExplorerTabs")),
        gpui::KeyBinding::new("cmd-shift-t", ReopenClosedTab, Some("ExplorerTabs")),
        gpui::KeyBinding::new("ctrl-shift-k", DuplicateTab, Some("ExplorerTabs")),
        gpui::KeyBinding::new("cmd-shift-k", DuplicateTab, Some("ExplorerTabs")),
    ]);
}

pub(super) struct ClosedTab {
    location: Location,
    display: crate::session_data::SavedDisplay,
    index: usize,
}
impl ClosedTab {
    fn capture(folder: &FolderView, index: usize) -> Option<Self> {
        let location = folder.tab_location();
        let valid = match location {
            Location::Home => true,
            Location::Folder(path) => crate::session_data::valid_path(path),
            Location::Archive { archive, prefix } => {
                crate::session_data::valid_path(archive) && prefix.len() <= 4096
            }
        };
        valid.then(|| Self {
            location: location.clone(),
            display: folder.session_display(),
            index,
        })
    }
}

impl Explorer {
    fn tab_workflow_allowed(&self, cx: &App) -> bool {
        !self.settings_open
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| !tab.read(cx).tab_workflow_blocked())
    }
    pub(super) fn remember_closed(&mut self, index: usize, cx: &App) {
        let Some(snapshot) = self
            .tabs
            .get(index)
            .and_then(|tab| ClosedTab::capture(tab.read(cx), index))
        else {
            return;
        };
        if self.closed_tabs.len() == 20 {
            self.closed_tabs.pop_front();
        }
        self.closed_tabs.push_back(snapshot);
    }
    fn insert_tab_snapshot(&mut self, snapshot: ClosedTab, w: &mut Window, cx: &mut Context<Self>) {
        let tab = cx.new(|cx| {
            let mut tab =
                FolderView::new_deferred(snapshot.location, self.mode, self.icons.clone(), w, cx);
            tab.restore_session_display(snapshot.display);
            tab.compact = self.compact;
            tab
        });
        self.watch_tab(&tab, w, cx);
        let index = snapshot.index.min(self.tabs.len());
        self.tabs.insert(index, tab);
        if index <= self.active {
            self.active += 1;
        }
        self.tab_menu = None;
        self.switch(index, w, cx);
    }
    pub(super) fn reopen_closed_tab(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        cx.notify();
        if !self.tab_workflow_allowed(cx) {
            return;
        }
        if let Some(snapshot) = self.closed_tabs.pop_back() {
            self.insert_tab_snapshot(snapshot, w, cx);
        }
    }
    pub(super) fn duplicate_tab_at(
        &mut self,
        index: usize,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = None;
        cx.notify();
        if !self.tab_workflow_allowed(cx) {
            return;
        }
        let Some(snapshot) = self
            .tabs
            .get(index)
            .and_then(|tab| ClosedTab::capture(tab.read(cx), index + 1))
        else {
            self.tab_error = Some(
                "この場所はタブを複製できません（保存可能なパスの上限を確認してください）。".into(),
            );
            cx.notify();
            return;
        };
        self.insert_tab_snapshot(snapshot, w, cx);
    }
}
