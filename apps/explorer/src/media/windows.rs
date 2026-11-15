use ::windows::{
    Win32::{
        Foundation::SIZE,
        UI::Shell::{
            IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
            SIIGBF_THUMBNAILONLY,
        },
    },
    core::PCWSTR,
};
use std::{
    mem::{size_of, zeroed},
    path::Path,
};
use windows_sys::Win32::Graphics::Gdi::*;

/// Call only from a COM-initialized worker. Never asks Shell for an icon fallback.
pub(super) fn thumbnail(path: &Path, edge: u32) -> Option<image::RgbaImage> {
    let wide = crate::native_shell::windows_path_wide(path).ok()?;
    let edge = edge.clamp(64, 1024) as i32;
    // SAFETY: Shell inputs are terminated; bitmap/DC handles are owned locally
    // and released on every path. Image dimensions are checked before allocation.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let handle = factory
            .GetImage(
                SIZE { cx: edge, cy: edge },
                SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT,
            )
            .ok()?;
        struct Bitmap(HBITMAP);
        impl Drop for Bitmap {
            fn drop(&mut self) {
                unsafe {
                    DeleteObject(self.0);
                }
            }
        }
        struct Dc(HDC);
        impl Drop for Dc {
            fn drop(&mut self) {
                unsafe {
                    DeleteDC(self.0);
                }
            }
        }
        let bitmap = Bitmap(handle.0);
        let mut info: BITMAP = zeroed();
        if GetObjectW(
            bitmap.0,
            size_of::<BITMAP>() as i32,
            (&mut info as *mut BITMAP).cast(),
        ) == 0
        {
            return None;
        }
        let width = u32::try_from(info.bmWidth).ok()?;
        let height = info.bmHeight.unsigned_abs();
        if width == 0 || height == 0 || width > 2048 || height > 2048 {
            return None;
        }
        let dc = Dc(CreateCompatibleDC(std::ptr::null_mut()));
        if dc.0.is_null() {
            return None;
        }
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width as i32;
        bmi.bmiHeader.biHeight = -(height as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        if GetDIBits(
            dc.0,
            bitmap.0,
            0,
            height,
            pixels.as_mut_ptr().cast(),
            &mut bmi,
            DIB_RGB_COLORS,
        ) != height as i32
        {
            return None;
        }
        // Some providers emit BGRX; others premultiplied BGRA. Restore straight
        // RGBA here, then the shared upload conversion produces GPUI's BGRA.
        let opaque = pixels.chunks_exact(4).all(|pixel| pixel[3] == 0);
        for pixel in pixels.chunks_exact_mut(4) {
            if opaque {
                pixel[3] = 255;
            }
            let alpha = u16::from(pixel[3]);
            if alpha > 0 && alpha < 255 {
                for value in &mut pixel[..3] {
                    *value = ((u16::from(*value) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
            pixel.swap(0, 2);
        }
        image::RgbaImage::from_raw(width, height, pixels)
    }
}
