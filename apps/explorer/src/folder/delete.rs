//! Selection-based deletion. The Windows adapter owns permanent-delete confirmation;
//! keeping one native confirmation preserves system-specific recycle/elevation warnings.
use super::*;
use crate::file_operations::{self, DeleteKind};

impl FolderView {
    pub(super) fn delete_selected(
        &mut self,
        _: &DeleteFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_delete(DeleteKind::Recycle, window, cx);
    }

    pub(super) fn delete_permanently(
        &mut self,
        _: &DeletePermanently,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_delete(DeleteKind::Permanent, window, cx);
    }

    fn begin_delete(&mut self, kind: DeleteKind, window: &mut Window, cx: &mut Context<Self>) {
        // path() is deliberately None for Home and read-only archive locations. TextInput
        // owns Delete while editing a filename, search term or address.
        if self.path().is_none() || self.input_focused(window, cx) || self.marquee_in_progress() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.cancel_scroll_motion();
        self.context_menu = None;
        window.focus(&self.focus, cx);
        match file_operations::delete(paths.clone(), kind, window) {
            Ok(receiver) => {
                self.notice = Some("削除しています…".into());
                // The snapshot, not a later selection, determines both the operation and
                // which tabs refresh after success, cancellation or a partial failure.
                self.observe_mutation_result(paths, receiver, "削除しました", cx);
            }
            Err(error) => self.notice = Some(error.to_string()),
        }
        cx.notify();
    }
}
