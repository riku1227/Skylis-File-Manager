#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod assets;
mod explorer;
mod folder;
mod native_shell;
mod search;
mod settings;
mod settings_data;
mod shell_icons;
mod size_service;
mod theme;
mod theme_tokens;
mod actions {
    gpui::actions!(
        explorer,
        [
            SelectPrev,
            SelectLeft,
            SelectRight,
            SelectNext,
            SelectFirst,
            SelectLast,
            PageUp,
            PageDown,
            OpenSelected,
            GoBack,
            GoForward,
            GoUp,
            Refresh,
            ToggleHidden,
            Quit,
            NewTab,
            CloseTab,
            NextTab,
            PrevTab,
            CycleTheme,
            ToggleDensity,
            FocusSearch,
            EditAddress,
            OpenSettings,
            ShowContextMenu,
            QuickPreview,
            SelectAllFiles,
            CopyFiles,
            CutFiles,
            PasteFiles,
            DetachTab,
            MergeWindowTabs,
            ClearSearch
        ]
    );
}
use actions::*;
use gpui::{
    App, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
fn main() {
    let preferences = settings::load();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            let preferences = cx.new(|cx| settings::Preferences::new(preferences, cx));
            cx.set_global(settings::SharedPreferences(preferences));
            let ctx = Some("Explorer");
            cx.bind_keys([
                KeyBinding::new("up", SelectPrev, ctx),
                KeyBinding::new("left", SelectLeft, ctx),
                KeyBinding::new("right", SelectRight, ctx),
                KeyBinding::new("down", SelectNext, ctx),
                KeyBinding::new("home", SelectFirst, ctx),
                KeyBinding::new("end", SelectLast, ctx),
                KeyBinding::new("pageup", PageUp, ctx),
                KeyBinding::new("pagedown", PageDown, ctx),
                KeyBinding::new("enter", OpenSelected, ctx),
                KeyBinding::new("backspace", GoBack, ctx),
                KeyBinding::new("alt-left", GoBack, ctx),
                KeyBinding::new("alt-right", GoForward, ctx),
                KeyBinding::new("alt-up", GoUp, ctx),
                KeyBinding::new("f5", Refresh, ctx),
                KeyBinding::new("ctrl-r", Refresh, ctx),
                KeyBinding::new("ctrl-h", ToggleHidden, ctx),
                KeyBinding::new("ctrl-f", FocusSearch, ctx),
                KeyBinding::new("ctrl-l", EditAddress, ctx),
                KeyBinding::new("alt-d", EditAddress, ctx),
                KeyBinding::new("cmd-l", EditAddress, ctx),
                KeyBinding::new("escape", ClearSearch, ctx),
                KeyBinding::new("ctrl-t", NewTab, Some("ExplorerTabs")),
                KeyBinding::new("ctrl-w", CloseTab, Some("ExplorerTabs")),
                KeyBinding::new("ctrl-tab", NextTab, Some("ExplorerTabs")),
                KeyBinding::new("ctrl-shift-tab", PrevTab, Some("ExplorerTabs")),
                KeyBinding::new("ctrl-shift-l", CycleTheme, Some("ExplorerTabs")),
                KeyBinding::new("cmd-t", NewTab, Some("ExplorerTabs")),
                KeyBinding::new("cmd-w", CloseTab, Some("ExplorerTabs")),
                KeyBinding::new("cmd-f", FocusSearch, ctx),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            cx.bind_keys([
                KeyBinding::new("ctrl-,", OpenSettings, Some("ExplorerTabs")),
                KeyBinding::new("cmd-,", OpenSettings, Some("ExplorerTabs")),
            ]);
            cx.bind_keys([
                KeyBinding::new("shift-f10", ShowContextMenu, ctx),
                KeyBinding::new("menu", ShowContextMenu, ctx),
            ]);
            cx.bind_keys([
                KeyBinding::new("ctrl-shift-n", DetachTab, Some("ExplorerTabs")),
                KeyBinding::new("cmd-shift-n", DetachTab, Some("ExplorerTabs")),
            ]);
            cx.bind_keys([KeyBinding::new("space", QuickPreview, ctx)]);
            cx.bind_keys([
                KeyBinding::new("ctrl-a", SelectAllFiles, ctx),
                KeyBinding::new("cmd-a", SelectAllFiles, ctx),
                KeyBinding::new("ctrl-c", CopyFiles, ctx),
                KeyBinding::new("ctrl-x", CutFiles, ctx),
                KeyBinding::new("ctrl-v", PasteFiles, ctx),
            ]);
            cx.bind_keys([
                KeyBinding::new("cmd-c", CopyFiles, ctx),
                KeyBinding::new("cmd-v", PasteFiles, ctx),
            ]);
            search::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Explorer".into()),
                        appears_transparent: cfg!(any(target_os = "windows", target_os = "macos")),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| cx.new(|cx| explorer::Explorer::new(w, cx)),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}

mod media;

mod file_operations;
