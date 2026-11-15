use super::*;

impl FolderView {
    pub(super) fn discover_wsl(&mut self, cx: &mut Context<Self>) {
        if !cfg!(windows) {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let _ = std::thread::Builder::new()
            .name("wsl-discovery".into())
            .spawn(move || {
                let _ = tx.send_blocking(explorer_core::wsl::installed_distributions());
            });
        self._wsl_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(distributions) => this.wsl = distributions,
                        Err(error) => {
                            this.notice = Some(format!("Linuxの一覧を取得できません: {error}"))
                        }
                    }
                    cx.notify();
                });
            }
        }));
    }
    pub(super) fn render_wsl(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div().when(!self.wsl.is_empty(), |d| {
            d.child(
                div()
                    .px(px(12.))
                    .mt(px(18.))
                    .mb(px(6.))
                    .text_size(px(11.))
                    .text_color(t.c("text.muted"))
                    .child("LINUX"),
            )
            .children(self.wsl.iter().enumerate().map(|(ix, distro)| {
                let path = distro.path.clone();
                let active = self.path().is_some_and(|p| p.starts_with(&path));
                div()
                    .id(("wsl", ix))
                    .mx(px(8.))
                    .px(px(10.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(t.c("nav.current.bg")))
                    .hover(|s| s.bg(t.c("state.hover")))
                    // Static asset: requesting a Shell icon for the UNC path may start WSL.
                    .child(
                        svg()
                            .path("icons/linux.svg")
                            .size(px(16.))
                            .text_color(t.c("icon.default")),
                    )
                    .child(div().truncate().child(distro.name.clone()))
                    .map(|d| Self::file_drop_target(d, Some(path.clone()), t, cx))
                    .on_click(
                        cx.listener(move |this, _, w, cx| this.navigate_to(path.clone(), w, cx)),
                    )
            }))
        })
    }
}
