#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod archive_jobs;
mod assets;
mod diagnostics;
mod directory_watch;
mod explorer;
mod file_open;
mod file_operation_progress;
mod folder;
mod folder_integration;
mod launch_request;
mod motion;
mod motion_preferences;
mod native_shell;
mod search;
mod settings;
mod settings_data;
mod shell_handoff;
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
            NewWindow,
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
            RenameSelected,
            DeleteFiles,
            DeletePermanently,
            NewFolderSelected,
            CopySelectedPaths,
            ShowProperties,
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
    if launch_request::current().shell_command_server {
        shell_handoff::run_command_server();
        return;
    }
    let preferences = settings::load();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            motion_preferences::init(cx);
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
                KeyBinding::new("f2", RenameSelected, ctx),
                KeyBinding::new("delete", DeleteFiles, ctx),
                KeyBinding::new("shift-delete", DeletePermanently, ctx),
                KeyBinding::new("ctrl-shift-c", CopySelectedPaths, ctx),
                KeyBinding::new("alt-enter", ShowProperties, ctx),
                KeyBinding::new("ctrl-r", Refresh, ctx),
                KeyBinding::new("ctrl-h", ToggleHidden, ctx),
                KeyBinding::new("ctrl-f", FocusSearch, ctx),
                KeyBinding::new("ctrl-l", EditAddress, ctx),
                KeyBinding::new("alt-d", EditAddress, ctx),
                KeyBinding::new("cmd-l", EditAddress, ctx),
                KeyBinding::new("escape", ClearSearch, ctx),
                KeyBinding::new("ctrl-t", NewTab, Some("ExplorerTabs")),
                KeyBinding::new("ctrl-n", NewWindow, Some("ExplorerTabs")),
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
                KeyBinding::new("ctrl-shift-n", NewFolderSelected, ctx),
                KeyBinding::new("cmd-shift-n", NewFolderSelected, ctx),
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
            folder::display_settings::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| explorer::request_quit(cx));
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
