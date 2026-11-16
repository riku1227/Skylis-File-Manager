//! Keep AppKit's titlebar move gesture separate from GPUI tab dragging.
#[cfg(target_os = "macos")]
use gpui::MouseButton;
use gpui::{
    Bounds, InteractiveElement, Pixels, Styled, TitlebarOptions, WindowBounds, WindowOptions, div,
    px, size,
};

/// Every Explorer window, including detached tabs, must use the same drag policy.
pub(crate) fn window_options(bounds: Bounds<Pixels>, title: &str) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            appears_transparent: cfg!(any(target_os = "windows", target_os = "macos")),
            ..Default::default()
        }),
        // AppKit otherwise claims the transparent titlebar before GPUI can drag tabs.
        // This preserves is_movable, including the native window tiling commands.
        app_owns_titlebar_drag: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq)]
enum TitlebarAction {
    Move,
    DoubleClick,
    Ignore,
}

#[cfg(any(target_os = "macos", test))]
fn titlebar_action(click_count: usize, dragging: bool) -> TitlebarAction {
    if dragging {
        TitlebarAction::Ignore
    } else if click_count == 2 {
        TitlebarAction::DoubleClick
    } else if click_count == 1 {
        TitlebarAction::Move
    } else {
        TitlebarAction::Ignore
    }
}

pub(crate) fn drag_region() -> impl gpui::IntoElement {
    let region = div()
        .id("titlebar-drag")
        .flex_1()
        .min_w(px(32.))
        .h_full()
        .window_control_area(gpui::WindowControlArea::Drag);
    #[cfg(target_os = "macos")]
    let region = region.on_mouse_down(MouseButton::Left, |event, window, cx| {
        match titlebar_action(event.click_count, cx.has_active_drag()) {
            TitlebarAction::Move => window.start_window_move(),
            // Let AppKit honor the user's zoom/minimize/do-nothing preference.
            TitlebarAction::DoubleClick => window.titlebar_double_click(),
            TitlebarAction::Ignore => {}
        }
    });
    region
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_windows_keep_native_controls_and_platform_drag_policy() {
        let bounds = Bounds::default();
        for title in ["Explorer", "Skylis"] {
            let options = window_options(bounds, title);
            assert_eq!(options.app_owns_titlebar_drag, cfg!(target_os = "macos"));
            assert!(options.is_movable && options.is_resizable && options.is_minimizable);
            assert_eq!(options.window_min_size, Some(size(px(900.), px(600.))));
            let titlebar = options.titlebar.unwrap();
            assert_eq!(titlebar.title.as_deref(), Some(title));
            assert_eq!(
                titlebar.appears_transparent,
                cfg!(any(target_os = "windows", target_os = "macos"))
            );
        }
    }

    #[test]
    fn blank_titlebar_routes_move_and_double_click_but_never_an_active_tab_drag() {
        assert_eq!(titlebar_action(1, false), TitlebarAction::Move);
        assert_eq!(titlebar_action(2, false), TitlebarAction::DoubleClick);
        assert_eq!(titlebar_action(3, false), TitlebarAction::Ignore);
        for count in [1, 2, 3] {
            assert_eq!(titlebar_action(count, true), TitlebarAction::Ignore);
        }
    }
}
