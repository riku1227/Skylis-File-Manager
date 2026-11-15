use super::*;
use crate::size_service::{self, SizeRequest, SizeUpdate, SizeUpdateKind};
use explorer_core::FolderSizeProgress;

#[derive(Clone)]
enum SizeStatus {
    Queued,
    Update(SizeUpdateKind),
}

#[derive(Default)]
pub(super) struct SizeState {
    suspended: bool,
    results: HashMap<PathBuf, SizeStatus>,
    token: CancelToken,
    generation: u64,
    automatic: Option<bool>,
    sender: Option<async_channel::Sender<SizeUpdate>>,
    task: Option<Task<()>>,
}

impl Drop for SizeState {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

impl FolderView {
    pub(super) fn update_size_mode(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if self
            .sizes
            .automatic
            .is_some_and(|previous| previous != automatic)
        {
            self.cancel_sizes(cx);
        }
        self.sizes.automatic = Some(automatic);
    }

    pub(super) fn stop_sizes(&mut self, cx: &mut Context<Self>) {
        self.cancel_sizes(cx);
        self.sizes.suspended = true;
    }
    pub(super) fn cancel_sizes(&mut self, cx: &mut Context<Self>) {
        self.sizes.suspended = false;
        self.sizes.token.cancel();
        self.sizes.token = CancelToken::new();
        self.sizes.generation = self.sizes.generation.wrapping_add(1);
        self.sizes.results.clear();
        self.sizes.sender = None;
        self.sizes.task = None;
        cx.notify();
    }

    pub(super) fn request_size(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.start_size(path, false, cx);
    }

    fn start_size(&mut self, path: PathBuf, automatic: bool, cx: &mut Context<Self>) {
        if matches!(
            self.sizes.results.get(&path),
            Some(SizeStatus::Queued | SizeStatus::Update(SizeUpdateKind::Progress(_)))
        ) {
            return;
        }
        if self.sizes.sender.is_none() {
            let (tx, rx) = async_channel::bounded::<SizeUpdate>(32);
            self.sizes.sender = Some(tx);
            self.sizes.task = Some(cx.spawn(async move |view, cx| {
                while let Ok(first) = rx.recv().await {
                    let mut batch = vec![first];
                    while batch.len() < 32 {
                        match rx.try_recv() {
                            Ok(update) => batch.push(update),
                            Err(_) => break,
                        }
                    }
                    if view
                        .update(cx, |view, cx| {
                            for update in batch {
                                if update.generation == view.sizes.generation
                                    && !view.sizes.token.is_cancelled()
                                {
                                    view.sizes
                                        .results
                                        .insert(update.path, SizeStatus::Update(update.kind));
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        let request = SizeRequest {
            path: path.clone(),
            generation: self.sizes.generation,
            cancel: self.sizes.token.clone(),
            automatic,
            updates: self
                .sizes
                .sender
                .as_ref()
                .expect("receiver initialized")
                .clone(),
        };
        let status = match size_service::request(request) {
            Ok(()) => SizeStatus::Queued,
            Err(error) => SizeStatus::Update(SizeUpdateKind::Failed(error)),
        };
        self.sizes.results.insert(path, status);
        cx.notify();
    }

    /// Call for directory rows only; it submits automatic work for visible rows.
    pub(super) fn size_cell(
        &mut self,
        path: &Path,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let automatic = cx
            .try_global::<crate::settings::SharedPreferences>()
            .is_some_and(|preferences| preferences.0.read(cx).value.automatic_sizes);
        self.update_size_mode(automatic, cx);
        if automatic && !self.sizes.suspended && !self.sizes.results.contains_key(path) {
            self.start_size(path.to_path_buf(), true, cx);
        }
        let (label, tooltip, clickable) = match self.sizes.results.get(path) {
            None | Some(SizeStatus::Update(SizeUpdateKind::ManualOnly)) => (
                "サイズ計算".to_owned(), "クリックしてこのフォルダーのサイズを計算します。ネットワークやクラウドは自動計算しません".to_owned(), true),
            Some(SizeStatus::Queued) => ("待機中…".into(), "バックグラウンドで順次計算します".into(), false),
            Some(SizeStatus::Update(SizeUpdateKind::Progress(progress))) => (
                size_label(*progress, false), "計算中の下限値です。フォルダー移動で中止します".into(), false),
            Some(SizeStatus::Update(SizeUpdateKind::Complete(progress))) => (
                size_label(*progress, true), format!("{} ファイル・{} フォルダー。読み取り不可 {}、リンク／再解析点 {}、深さ制限 {}。論理サイズ（ディスク使用量ではありません）", progress.files, progress.directories, progress.skipped_errors, progress.skipped_links, progress.depth_limited), false),
            Some(SizeStatus::Update(SizeUpdateKind::Failed(error))) => ("再計算".into(), error.clone(), true),
        };
        let path = path.to_path_buf();
        div()
            .id(gpui::SharedString::from(format!(
                "folder-size:{}",
                path.display()
            )))
            .text_size(px(12.))
            .text_color(t.c(if clickable { "accent.fg" } else { "text.muted" }))
            .truncate()
            .child(label)
            .tooltip(move |_, cx| {
                cx.new(|_| Tip {
                    text: tooltip.clone(),
                    theme: t,
                })
                .into()
            })
            .when(clickable, |d| {
                d.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.request_size(path.clone(), cx);
                    }))
            })
            .into_any_element()
    }
}

fn size_label(progress: FolderSizeProgress, complete: bool) -> String {
    let size = format::size(progress.bytes);
    if !complete || progress.is_partial() || progress.skipped_links != 0 {
        format!("≥ {size}")
    } else {
        size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_or_excluded_totals_are_labelled_as_lower_bounds() {
        let complete = FolderSizeProgress {
            bytes: 1024,
            ..Default::default()
        };
        assert!(!size_label(complete, true).starts_with('≥'));
        assert!(size_label(complete, false).starts_with('≥'));
        assert!(
            size_label(
                FolderSizeProgress {
                    skipped_links: 1,
                    ..complete
                },
                true
            )
            .starts_with('≥')
        );
        assert!(
            size_label(
                FolderSizeProgress {
                    skipped_errors: 1,
                    ..complete
                },
                true
            )
            .starts_with('≥')
        );
    }
}
