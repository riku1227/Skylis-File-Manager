//! AppKit file actions. These expose Services/Share, not Finder Sync extensions.
use std::path::PathBuf;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn skylis_show_share(
        view: *mut std::ffi::c_void,
        paths: *const *const std::ffi::c_char,
        count: usize,
        x: f64,
        y: f64,
    ) -> i32;
    fn skylis_show_services(
        view: *mut std::ffi::c_void,
        paths: *const *const std::ffi::c_char,
        count: usize,
        x: f64,
        y: f64,
    ) -> i32;
    fn skylis_copy_files(paths: *const *const std::ffi::c_char, count: usize) -> i32;
}

#[cfg(target_os = "macos")]
fn encoded_paths(paths: &[PathBuf]) -> Result<Vec<std::ffi::CString>, String> {
    use std::os::unix::ffi::OsStrExt;
    if paths.is_empty() || paths.len() > 4096 {
        return Err("対象のファイル数が不正です".into());
    }
    paths
        .iter()
        .map(|path| {
            if !path.is_absolute() {
                return Err("ファイル操作には絶対パスが必要です".into());
            }
            std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| "パスにNUL文字があります".into())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn status(result: i32) -> Result<(), String> {
    match result {
        0 => Ok(()),
        -1 => Err("macOSのファイル操作はメインスレッドで実行してください".into()),
        -2 => Err("対象ファイルまたはウィンドウを取得できません".into()),
        -4 => Err("対応するサービスがありません。システム設定のサービスも確認してください".into()),
        _ => Err("macOSのファイル操作を開始できませんでした".into()),
    }
}

pub(crate) fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let strings = encoded_paths(paths)?;
        let pointers: Vec<_> = strings.iter().map(|path| path.as_ptr()).collect();
        // SAFETY: bounded terminated paths live through the main-thread call;
        // AppKit serializes NSURLs before this returns.
        status(unsafe { skylis_copy_files(pointers.as_ptr(), pointers.len()) })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        Err("この操作はmacOS用です".into())
    }
}

pub(crate) fn show_share(
    paths: &[PathBuf],
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> Result<(), String> {
    show(paths, window, position, false)
}

pub(crate) fn show_services(
    paths: &[PathBuf],
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> Result<(), String> {
    show(paths, window, position, true)
}

fn show(
    paths: &[PathBuf],
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
    services: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let strings = encoded_paths(paths)?;
        let pointers: Vec<_> = strings.iter().map(|path| path.as_ptr()).collect();
        let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("macOSのウィンドウを取得できません".into());
        };
        let x = f32::from(position.x) as f64;
        let y = f32::from(position.y) as f64;
        // SAFETY: NSView belongs to this live window. Native adapter copies URLs,
        // retains asynchronous picker state, and bounds synchronous Services state.
        status(unsafe {
            if services {
                skylis_show_services(
                    handle.ns_view.as_ptr(),
                    pointers.as_ptr(),
                    pointers.len(),
                    x,
                    y,
                )
            } else {
                skylis_show_share(
                    handle.ns_view.as_ptr(),
                    pointers.as_ptr(),
                    pointers.len(),
                    x,
                    y,
                )
            }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (paths, window, position, services);
        Err("この操作はmacOS用です".into())
    }
}
