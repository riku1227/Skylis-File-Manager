//! Native macOS Quick Look panel. Invoke on the GPUI main thread only.
use std::path::Path;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn skylis_quicklook_show(view: *mut std::ffi::c_void, path: *const std::ffi::c_char) -> i32;
    fn skylis_quicklook_update(view: *mut std::ffi::c_void, path: *const std::ffi::c_char) -> i32;
    fn skylis_quicklook_close(view: *mut std::ffi::c_void) -> i32;
    fn skylis_quicklook_visible(view: *mut std::ffi::c_void) -> i32;
}

/// Refresh selection only while this window still owns a visible native panel.
/// `Ok(false)` means the user closed it or another controller took over. Unlike
/// `show`, this function never opens, raises, or takes control of a panel.
pub(crate) fn update(path: &Path, window: &gpui::Window) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        if !path.is_absolute() {
            return Err("Quick Lookには絶対パスが必要です".into());
        }
        let path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| "パスにNUL文字があります")?;
        let view = view_pointer(window)?;
        // SAFETY: bounded synchronous main-thread call with live borrowed handles.
        // Native code checks visibility and ownership before any panel mutation.
        match unsafe { skylis_quicklook_update(view, path.as_ptr()) } {
            0 => Ok(false),
            1 => Ok(true),
            -1 => Err("Quick Lookはメインスレッドから呼び出してください".into()),
            _ => Err("Quick Lookの表示を更新できませんでした".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, window);
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
fn view_pointer(window: &gpui::Window) -> Result<*mut std::ffi::c_void, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr()),
        _ => Err("macOSのウィンドウを取得できません".into()),
    }
}

pub(crate) fn show(path: &Path, window: &gpui::Window) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        if !path.is_absolute() {
            return Err("Quick Lookには絶対パスが必要です".into());
        }
        let path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| "パスにNUL文字があります")?;
        let view = view_pointer(window)?;
        // SAFETY: borrowed NSView and terminated path live through this main-thread
        // call. The adapter retains its NSURL/controller and catches ObjC exceptions.
        match unsafe { skylis_quicklook_show(view, path.as_ptr()) } {
            0 => Ok(()),
            -1 => Err("Quick Lookはメインスレッドから呼び出してください".into()),
            _ => Err("Quick Lookを開始できませんでした".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, window);
        Err("Quick LookはmacOSで利用できます".into())
    }
}

/// Closes only this window's panel; another Skylis window's preview is untouched.
pub(crate) fn close(window: &gpui::Window) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let view = view_pointer(window)?;
        if unsafe { skylis_quicklook_close(view) } == 0 {
            Ok(())
        } else {
            Err("Quick Lookを閉じられませんでした".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(())
    }
}

/// Query before toggling: Escape may have already closed the native panel.
pub(crate) fn is_visible(window: &gpui::Window) -> bool {
    #[cfg(target_os = "macos")]
    {
        view_pointer(window).is_ok_and(|view| unsafe { skylis_quicklook_visible(view) } == 1)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}
