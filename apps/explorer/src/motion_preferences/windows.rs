use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETWHEELSCROLLLINES, SystemParametersInfoW,
};

pub(super) fn reduce_motion() -> Result<bool, String> {
    let mut animations_enabled = 0i32;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL (i32) to this live local.
    // This is a read-only query, and neither uiParam nor update flags are used.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&raw mut animations_enabled).cast(),
            0,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(animations_enabled == 0)
    }
}

pub(super) fn wheel_scroll_lines() -> Result<u32, String> {
    let mut lines = 0u32;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes a UINT (u32) to this live local.
    let success =
        unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&raw mut lines).cast(), 0) };
    if success == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(lines)
    }
}
