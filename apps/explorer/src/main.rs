#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod archive_jobs;
mod assets;
mod diagnostics;
mod directory_watch;
mod explorer;
mod file_open;
#[cfg(windows)]
mod file_operation_progress;
mod folder;
mod folder_integration;
mod launch_request;
mod macos_permissions;
mod motion;
mod motion_preferences;
mod name_search;
mod native_shell;
mod operation_center;
mod search;
mod session;
mod session_data;
mod settings;
mod settings_data;
mod shell_handoff;
mod shell_icons;
mod size_service;
mod theme;
mod theme_tokens;
mod window_chrome;
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
            OpenPermissions,
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
use gpui::{App, Bounds, KeyBinding, prelude::*, px, size};
fn main() {
    if launch_request::current().shell_command_server {
        shell_handoff::run_command_server();
        return;
    }
    let preferences = settings::load();
    let previous = session::load();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            motion_preferences::init(cx);
            let preferences = cx.new(|cx| settings::Preferences::new(preferences, cx));
            cx.set_global(settings::SharedPreferences(preferences));
            let startup = previous.0.startup(
                cx.global::<settings::SharedPreferences>()
                    .0
                    .read(cx)
                    .value
                    .restore_tabs,
                session::explicit_launch(launch_request::current()),
            );
            session::init(previous, cx);
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
                KeyBinding::new(
                    "ctrl-shift-j",
                    explorer::activity::ToggleActivity,
                    Some("ExplorerTabs"),
                ),
                KeyBinding::new(
                    "cmd-shift-j",
                    explorer::activity::ToggleActivity,
                    Some("ExplorerTabs"),
                ),
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
            folder::keyboard::bind_keys(cx);
            explorer::bind_workflow_keys(cx);
            explorer::activity::bind_trigger_keys(cx);
            folder::display_settings::bind_keys(cx);
            folder::search_filters::bind_keys(cx);
            folder::context_menu::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| explorer::request_quit(cx));
            operation_center::init(cx);
            if startup.is_empty() {
                let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
                cx.open_window(
                    window_chrome::window_options(bounds, "Explorer"),
                    |w, cx| cx.new(|cx| explorer::Explorer::new(w, cx)),
                )
                .expect("failed to open window");
            } else {
                let restore = !session::explicit_launch(launch_request::current());
                for saved in startup {
                    let options = session::window_options(&saved, cx);
                    cx.open_window(options, move |w, cx| {
                        cx.new(|cx| {
                            if restore {
                                explorer::Explorer::restore(saved, w, cx)
                            } else {
                                explorer::Explorer::new(w, cx)
                            }
                        })
                    })
                    .expect("failed to restore window");
                }
            }
            cx.activate(true);
        });
}

mod media;

mod file_operations;
