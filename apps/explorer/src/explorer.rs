use crate::{
    actions::*,
    folder::{FolderView, icon_button},
    shell_icons::ShellIcons,
    theme::{Theme, ThemeMode},
};
use gpui::{App, Context, Entity, FocusHandle, Focusable, Window, div, img, prelude::*, px, svg};
use std::path::PathBuf;

/// Each tab owns its entity and asynchronous listing. Switching never redirects a pending batch.
pub struct Explorer {
    tabs: Vec<Entity<FolderView>>,
    active: usize,
    mode: ThemeMode,
    compact: bool,
    icons: Entity<ShellIcons>,
}
impl Explorer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let mut path = None;
        let mut mode = ThemeMode::System;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--theme" && i + 1 < args.len() {
                i += 1;
                mode = match args[i].to_str() {
                    Some("dark") => ThemeMode::Dark,
                    Some("light") => ThemeMode::Light,
                    _ => ThemeMode::System,
                };
            } else if args[i] == "--path" && i + 1 < args.len() {
                i += 1;
                path = Some(PathBuf::from(&args[i]));
            } else if !args[i].to_string_lossy().starts_with('-') {
                path = Some(PathBuf::from(&args[i]));
            }
            i += 1;
        }
        let icons = cx.new(ShellIcons::new);
        let start = path.unwrap_or_else(explorer_core::platform::home_dir);
        let folder = cx.new(|cx| FolderView::new(start, mode, icons.clone(), window, cx));
        cx.observe(&folder, |_, _, cx| cx.notify()).detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        Self {
            tabs: vec![folder],
            active: 0,
            mode,
            compact: false,
            icons,
        }
    }
    fn switch(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
        window.focus(&self.tabs[index].focus_handle(cx), cx);
        cx.notify();
    }
    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.tabs[self.active].read(cx).path().to_path_buf();
        let folder = cx.new(|cx| FolderView::new(path, self.mode, self.icons.clone(), window, cx));
        cx.observe(&folder, |_, _, cx| cx.notify()).detach();
        folder.update(cx, |folder, _| folder.compact = self.compact);
        self.tabs.push(folder);
        self.switch(self.tabs.len() - 1, window, cx);
    }
    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 {
            return;
        }
        self.tabs.remove(index);
        self.active = after_close(self.active, index, self.tabs.len());
        self.switch(self.active, window, cx);
    }
    fn close_tab(&mut self, _: &CloseTab, w: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, w, cx)
    }
    fn next_tab(&mut self, _: &NextTab, w: &mut Window, cx: &mut Context<Self>) {
        self.switch((self.active + 1) % self.tabs.len(), w, cx)
    }
    fn prev_tab(&mut self, _: &PrevTab, w: &mut Window, cx: &mut Context<Self>) {
        self.switch((self.active + self.tabs.len() - 1) % self.tabs.len(), w, cx)
    }
    fn cycle_theme(&mut self, _: &CycleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.mode = self.mode.next();
        for tab in &self.tabs {
            tab.update(cx, |tab, cx| {
                tab.mode = self.mode;
                cx.notify()
            })
        }
        cx.notify();
    }
    fn density(&mut self, _: &ToggleDensity, _: &mut Window, cx: &mut Context<Self>) {
        self.compact = !self.compact;
        for tab in &self.tabs {
            tab.update(cx, |tab, cx| {
                tab.compact = self.compact;
                cx.notify()
            })
        }
        cx.notify();
    }
}
fn after_close(active: usize, closed: usize, remaining: usize) -> usize {
    if closed < active {
        active - 1
    } else {
        active.min(remaining - 1)
    }
}
impl Focusable for Explorer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tabs[self.active].focus_handle(cx)
    }
}
impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::resolve(self.mode, window.appearance());
        window.set_window_title(&self.tabs[self.active].read(cx).label());
        let mut tabs = div()
            .id("tabs")
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .max_w_full()
            .overflow_x_scroll();
        for (i, tab) in self.tabs.iter().enumerate() {
            let current = i == self.active;
            let path = tab.read(cx).path().to_path_buf();
            let image = self.icons.update(cx, |icons, cx| {
                icons.get(&path, true, (14. * window.scale_factor()).ceil() as u32, cx)
            });
            tabs = tabs.child(
                div()
                    .id(("tab", i))
                    .h(px(28.))
                    .w(px(180.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.c(if current {
                        "tab.active.border"
                    } else {
                        "tab.border"
                    }))
                    .bg(t.c(if current { "tab.active.bg" } else { "tab.bg" }))
                    .text_color(t.c(if current { "text.primary" } else { "tab.fg" }))
                    .when(current, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                    .when(!current, |d| d.hover(|s| s.bg(t.c("tab.hover.bg"))))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .when_some(image, |d, image| d.child(img(image).size_full())),
                    )
                    .child(div().flex_1().truncate().child(tab.read(cx).label()))
                    .child(
                        div()
                            .id(("close-tab", i))
                            .size(px(18.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .hover(|s| s.bg(t.c("state.pressed")))
                            .child(
                                svg()
                                    .path("icons/x.svg")
                                    .size(px(12.))
                                    .text_color(t.c("text.secondary")),
                            )
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                this.close(i, w, cx)
                            })),
                    )
                    .on_click(cx.listener(move |this, _, w, cx| this.switch(i, w, cx))),
            );
        }
        div()
            .key_context("ExplorerTabs")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.c("surface.canvas"))
            .text_color(t.c("text.primary"))
            .font_family("Segoe UI")
            .text_size(px(12.))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::cycle_theme))
            .on_action(cx.listener(Self::density))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(38.))
                    .flex_shrink_0()
                    .pl(px(if cfg!(target_os = "macos") { 80. } else { 10. }))
                    .child(tabs.mt(px(6.)))
                    .child(
                        icon_button("new-tab", "icons/plus.svg", true, t)
                            .size(px(28.))
                            .mt(px(6.))
                            .on_click(cx.listener(|this, _, w, cx| this.new_tab(&NewTab, w, cx))),
                    )
                    .child(
                        div()
                            .id("titlebar-drag")
                            .flex_1()
                            .min_w(px(32.))
                            .h_full()
                            .window_control_area(gpui::WindowControlArea::Drag),
                    )
                    .when(cfg!(target_os = "windows"), |d| {
                        d.child(window_controls(window.is_maximized(), t))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.tabs[self.active].clone()),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_tabs_preserves_active_neighbor() {
        assert_eq!(after_close(3, 0, 3), 2);
        assert_eq!(after_close(2, 2, 2), 1);
        assert_eq!(after_close(0, 2, 2), 0);
        assert_eq!(after_close(0, 0, 1), 0);
    }
}

/// Native Windows hit tests retain drag, double-click zoom, resize and Snap behavior.
/// The backend performs system-button actions; registering an on_click too would toggle twice.
fn window_controls(maximized: bool, t: Theme) -> impl IntoElement {
    let button = |id: &'static str, icon: &'static str, area: gpui::WindowControlArea| {
        div()
            .id(id)
            .w(px(46.))
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(area)
            .hover(|s| {
                s.bg(t.c(if area == gpui::WindowControlArea::Close {
                    "danger.solid"
                } else {
                    "state.hover"
                }))
            })
            .child(
                svg()
                    .path(icon)
                    .size(px(12.))
                    .text_color(t.c("text.primary")),
            )
    };
    div()
        .flex()
        .items_start()
        .h_full()
        .flex_shrink_0()
        .child(button(
            "window-minimize",
            "icons/minus.svg",
            gpui::WindowControlArea::Min,
        ))
        .child(button(
            "window-maximize",
            if maximized {
                "icons/restore.svg"
            } else {
                "icons/maximize.svg"
            },
            gpui::WindowControlArea::Max,
        ))
        .child(button(
            "window-close",
            "icons/x.svg",
            gpui::WindowControlArea::Close,
        ))
}
