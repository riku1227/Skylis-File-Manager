//! Session I/O stays off the UI thread. Only close/quit takes bounded in-memory snapshots.
use crate::session_data::{MAX_BYTES, MAX_WINDOWS, SavedWindow, Session};
use gpui::{App, Bounds, Global, WindowBounds, WindowId, point, px, size};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn path() -> PathBuf {
    crate::settings::settings_path().with_file_name("session.json")
}

pub fn load() -> (Session, Option<String>) {
    let result = (|| {
        let file = match std::fs::File::open(path()) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
            Err(e) => return Err(e.to_string()),
        };
        let mut text = String::new();
        file.take((MAX_BYTES + 1) as u64)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        Session::decode(&text)
    })();
    match result {
        Ok(value) => (value, None),
        Err(e) => (
            Session::default(),
            Some(format!(
                "前回のセッションを読み込めません（元のファイルを保持し保存を停止）: {e}"
            )),
        ),
    }
}

fn save(path: &Path, value: &Session) -> Result<(), String> {
    let text = value.encode()?;
    std::fs::create_dir_all(path.parent().ok_or("保存先がありません")?)
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // SAFETY: owned NUL-terminated paths live through this replacement of our own session file.
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    write.map_err(|e| e.to_string())
}

struct SaveRequest {
    value: Session,
    done: Option<async_channel::Sender<()>>,
}
pub struct Store {
    last: Session,
    writer: async_channel::Sender<SaveRequest>,
    pending: async_channel::Receiver<SaveRequest>,
    pub error: Option<String>,
    writable: bool,
    errors: async_channel::Receiver<String>,
}
impl Global for Store {}

pub fn init(initial: (Session, Option<String>), cx: &mut App) {
    // A separate Shell process must not replace the user's regular session.
    init_at(
        initial,
        path(),
        !explicit_launch(crate::launch_request::current()),
        cx,
    );
}

fn init_at(initial: (Session, Option<String>), save_path: PathBuf, allow_save: bool, cx: &mut App) {
    let (writer, requests) = async_channel::bounded::<SaveRequest>(1);
    let pending = requests.clone();
    let (errors_tx, errors) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("session-writer".into())
        .spawn(move || {
            while let Ok(request) = requests.recv_blocking() {
                if let Err(e) = save(&save_path, &request.value) {
                    let _ = errors_tx.try_send(format!("セッションを保存できません: {e}"));
                }
                if let Some(done) = request.done {
                    let _ = done.try_send(());
                }
            }
        });
    let writable = allow_save && initial.1.is_none() && spawned.is_ok();
    let error = initial.1.or_else(|| spawned.err().map(|e| e.to_string()));
    cx.set_global(Store {
        last: initial.0,
        writer,
        pending,
        writable,
        error,
        errors,
    });
    cx.on_app_quit(|cx| {
        let windows = collect(None, cx);
        if !windows.is_empty() {
            cx.global_mut::<Store>().last.windows = windows;
        }
        let restore = restore_enabled(cx);
        let store = cx.global::<Store>();
        let mut value = store.last.clone();
        if !restore {
            value.without_tabs();
        }
        let writer = store.writer.clone();
        let writable = store.writable;
        async move {
            if writable {
                let (done, received) = async_channel::bounded(1);
                if writer
                    .send(SaveRequest {
                        value,
                        done: Some(done),
                    })
                    .await
                    .is_ok()
                {
                    let _ = received.recv().await;
                }
            }
        }
    })
    .detach();
}

fn restore_enabled(cx: &App) -> bool {
    cx.global::<crate::settings::SharedPreferences>()
        .0
        .read(cx)
        .value
        .restore_tabs
}
pub fn error(cx: &mut App) -> Option<String> {
    cx.try_global::<Store>()?;
    let store = cx.global_mut::<Store>();
    while let Ok(error) = store.errors.try_recv() {
        store.error = Some(error);
    }
    store.error.clone()
}
pub fn restoration_disabled(cx: &mut App) {
    if cx.try_global::<Store>().is_none() {
        return;
    }
    let store = cx.global_mut::<Store>();
    store.last.without_tabs();
    if store.writable {
        let _ = store.pending.try_recv();
        if let Err(e) = store.writer.try_send(SaveRequest {
            value: store.last.clone(),
            done: None,
        }) {
            store.error = Some(e.to_string());
        }
    }
}
fn collect(exclude: Option<WindowId>, cx: &mut App) -> Vec<SavedWindow> {
    cx.windows()
        .into_iter()
        .filter(|h| Some(h.window_id()) != exclude)
        .filter_map(|h| h.downcast::<crate::explorer::Explorer>())
        .take(MAX_WINDOWS)
        .filter_map(|h| {
            h.update(cx, |view, window, cx| view.session_snapshot(window, cx))
                .ok()
        })
        .filter(|w| !w.tabs.is_empty())
        .collect()
}
pub fn closing(last: SavedWindow, id: WindowId, cx: &mut App) {
    if cx.try_global::<Store>().is_none() {
        return;
    }
    let mut windows = collect(Some(id), cx);
    if windows.is_empty() && !last.tabs.is_empty() {
        windows.push(last);
    }
    if windows.is_empty() {
        return;
    }
    let restore = restore_enabled(cx);
    let store = cx.global_mut::<Store>();
    store.last.windows = windows;
    let mut value = store.last.clone();
    if !restore {
        value.without_tabs();
    }
    while let Ok(error) = store.errors.try_recv() {
        store.error = Some(error);
    }
    if store.writable {
        // Only ordinary close requests can be pending while UI input is still active.
        let _ = store.pending.try_recv();
        if let Err(e) = store.writer.try_send(SaveRequest { value, done: None }) {
            store.error = Some(e.to_string());
        }
    }
}

pub fn explicit_launch(r: &crate::launch_request::LaunchRequest) -> bool {
    r.path.is_some()
        || r.switch_explorer
        || r.handoff_token.is_some()
        || r.selected_name.is_some()
        || r.error.is_some()
}

pub fn window_options(saved: &SavedWindow, cx: &App) -> gpui::WindowOptions {
    let display = cx
        .displays()
        .into_iter()
        .find(|d| {
            saved
                .display
                .as_ref()
                .is_some_and(|id| d.uuid().is_ok_and(|uuid| uuid.to_string() == *id))
        })
        .or_else(|| cx.primary_display());
    let origin = display
        .as_ref()
        .map(|d| d.bounds().origin)
        .unwrap_or_default();
    let screens: Vec<_> = display
        .as_ref()
        .map(|d| {
            let b = d.visible_bounds();
            [
                f32::from(b.origin.x - origin.x),
                f32::from(b.origin.y - origin.y),
                b.size.width.into(),
                b.size.height.into(),
            ]
        })
        .into_iter()
        .collect();
    let [x, y, w, h] = crate::session_data::fit_rect(saved.rect, &screens);
    let position = point(px(x), px(y));
    #[cfg(not(target_os = "macos"))]
    let position = position + origin;
    let bounds = Bounds::new(position, size(px(w), px(h)));
    let mut options = crate::window_chrome::window_options(bounds, "Skylis");
    options.display_id = display.map(|d| d.id());
    options.window_min_size = Some(size(px(900f32.min(w)), px(600f32.min(h))));
    options.window_bounds = Some(if saved.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    });
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    #[gpui::test]
    fn restore_selects_saved_display_and_falls_back_when_disconnected(
        cx: &mut gpui::TestAppContext,
    ) {
        let secondary = gpui::DisplayId::new(42);
        cx.simulate_display_added(secondary, None);
        let saved = cx.update(|cx| {
            let display = cx
                .displays()
                .into_iter()
                .find(|d| d.id() == secondary)
                .unwrap();
            SavedWindow {
                display: Some(display.uuid().unwrap().to_string()),
                ..Default::default()
            }
        });
        cx.update(|cx| assert_eq!(window_options(&saved, cx).display_id, Some(secondary)));
        cx.simulate_display_removed(secondary);
        cx.update(|cx| {
            let options = window_options(&saved, cx);
            assert_eq!(options.display_id, cx.primary_display().map(|d| d.id()));
            assert!(options.window_bounds.unwrap().get_bounds().origin.x >= px(0.));
        });
    }

    #[gpui::test]
    fn explicit_process_cannot_overwrite_regular_session(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.json");
        let initial = Session {
            version: 1,
            windows: vec![SavedWindow::default(), SavedWindow::default()],
        };
        save(&file, &initial).unwrap();
        setup(cx, file.clone(), true);
        cx.update(|cx| cx.global_mut::<Store>().writable = false);
        let window = cx
            .add_window(|w, cx| crate::explorer::Explorer::restore(SavedWindow::default(), w, cx));
        window
            .update(cx, |v, w, cx| {
                closing(v.session_snapshot(w, cx), w.window_handle().window_id(), cx)
            })
            .unwrap();
        cx.update(|cx| cx.shutdown());
        assert_eq!(
            Session::decode(&std::fs::read_to_string(file).unwrap()).unwrap(),
            initial
        );
    }

    #[gpui::test]
    fn disabling_restoration_strips_paths_immediately_and_last_close_keeps_geometry(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.json");
        setup(cx, file.clone(), true);
        cx.update(|cx| {
            cx.global_mut::<Store>().last.windows = vec![SavedWindow {
                tabs: vec![Some(dir.path().to_path_buf())],
                ..Default::default()
            }];
            let preferences = cx.global::<crate::settings::SharedPreferences>().0.clone();
            preferences.update(cx, |p, cx| p.change(|s| s.restore_tabs = false, cx));
            assert_eq!(cx.global::<Store>().last.windows[0].tabs, vec![None]);
        });
        let window = cx
            .add_window(|w, cx| crate::explorer::Explorer::restore(SavedWindow::default(), w, cx));
        window
            .update(cx, |v, w, cx| {
                let snapshot = v.session_snapshot(w, cx);
                closing(snapshot.clone(), w.window_handle().window_id(), cx);
                assert_eq!(cx.global::<Store>().last.windows, vec![snapshot]);
            })
            .unwrap();
        cx.update(|cx| cx.shutdown());
        assert_eq!(
            Session::decode(&std::fs::read_to_string(file).unwrap())
                .unwrap()
                .windows[0]
                .tabs,
            vec![None]
        );
    }

    fn setup(cx: &mut gpui::TestAppContext, path: PathBuf, restore: bool) {
        cx.background_executor.allow_parking();
        // Shutdown's timeout is scheduler ticks in GPUI tests, not wall-clock time.
        // Give the real I/O thread time to acknowledge before that synthetic timeout.
        cx.background_executor
            .set_block_on_ticks(1_000_000..=1_000_000);
        cx.update(|cx| {
            let preferences = cx.new(|cx| {
                crate::settings::Preferences::new(
                    (
                        crate::settings_data::Settings {
                            restore_tabs: restore,
                            ..Default::default()
                        },
                        Some("memory-only session test".into()),
                    ),
                    cx,
                )
            });
            cx.set_global(crate::settings::SharedPreferences(preferences));
            init_at((Session::default(), None), path, true, cx);
        });
    }

    #[gpui::test]
    fn quit_saves_all_windows_and_selected_tab_without_starting_inactive_listing(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.json");
        setup(cx, file.clone(), true);
        let path = dir.path().join("missing-folder");
        let first = cx.add_window(|w, cx| {
            crate::explorer::Explorer::restore(
                SavedWindow {
                    tabs: vec![Some(path.clone()), None],
                    active: 1,
                    ..Default::default()
                },
                w,
                cx,
            )
        });
        let second = cx
            .add_window(|w, cx| crate::explorer::Explorer::restore(SavedWindow::default(), w, cx));
        first
            .update(cx, |v, w, cx| {
                let snapshot = v.session_snapshot(w, cx);
                assert_eq!(snapshot.tabs, vec![Some(path.clone()), None]);
                assert_eq!(snapshot.active, 1);
                crate::session::closing(snapshot, w.window_handle().window_id(), cx);
                assert_eq!(cx.global::<Store>().last.windows.len(), 1);
            })
            .unwrap();
        second
            .update(cx, |v, w, cx| {
                assert_eq!(v.session_snapshot(w, cx).tabs, vec![None])
            })
            .unwrap();
        cx.update(|cx| cx.shutdown());
        let saved = Session::decode(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(saved.windows.len(), 2);
        assert!(
            saved
                .windows
                .iter()
                .any(|w| w.active == 1 && w.tabs[0] == Some(path.clone()))
        );
    }

    #[gpui::test]
    fn quit_with_restore_disabled_saves_geometry_without_paths(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.json");
        setup(cx, file.clone(), false);
        cx.add_window(|w, cx| {
            crate::explorer::Explorer::restore(
                SavedWindow {
                    tabs: vec![Some(dir.path().to_path_buf()), None],
                    active: 1,
                    ..Default::default()
                },
                w,
                cx,
            )
        });
        cx.update(|cx| cx.shutdown());
        let saved = Session::decode(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(saved.windows[0].tabs, vec![None]);
        assert_eq!(saved.windows[0].active, 0);
        assert!(saved.windows[0].rect[2] > 0.);
    }

    #[test]
    fn explicit_path_handoff_and_errors_take_precedence_over_restore() {
        let mut request = crate::launch_request::LaunchRequest::default();
        assert!(!explicit_launch(&request));
        request.path = Some(std::env::current_dir().unwrap());
        assert!(explicit_launch(&request));
        request.path = None;
        request.handoff_token = Some("token".into());
        assert!(explicit_launch(&request));
    }
    #[test]
    fn atomic_save_replaces_only_session_and_keeps_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut value = Session::default();
        save(&path, &value).unwrap();
        value.windows.push(SavedWindow::default());
        save(&path, &value).unwrap();
        assert_eq!(
            Session::decode(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            value
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        value.version = 99;
        assert!(save(&path, &value).is_err());
        assert_eq!(
            Session::decode(&std::fs::read_to_string(path).unwrap())
                .unwrap()
                .version,
            1
        );
    }
}
