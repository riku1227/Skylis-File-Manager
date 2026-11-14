//! OS shell images, fetched on a bounded worker, never during UI layout/paint.
use gpui::{Context, RenderImage, Task};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    // None is the shell's virtual recycle bin, not a fake directory path.
    path: Option<PathBuf>,
    is_dir: bool,
    size: u32,
}

pub struct ShellIcons {
    cache: HashMap<Key, Option<Arc<RenderImage>>>,
    order: VecDeque<Key>,
    pending: HashSet<Key>,
    requests: async_channel::Sender<Key>,
    _receiver: Task<()>,
}

impl ShellIcons {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::bounded::<Key>(128);
        let (results_tx, results_rx) = async_channel::bounded(128);
        // One worker bounds shell extension concurrency. A slow extension cannot block paint.
        let _ = std::thread::Builder::new()
            .name("shell-icons".into())
            .spawn(move || {
                #[cfg(windows)]
                let initialized = unsafe {
                    windows_sys::Win32::System::Com::CoInitializeEx(
                        std::ptr::null(),
                        windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
                    )
                } >= 0;
                while let Ok(key) = rx.recv_blocking() {
                    let pixels = extract(key.path.as_deref(), key.is_dir, key.size);
                    if results_tx.send_blocking((key, pixels)).is_err() {
                        break;
                    }
                }
                #[cfg(windows)]
                if initialized {
                    unsafe {
                        windows_sys::Win32::System::Com::CoUninitialize();
                    }
                }
            });
        let receiver = cx.spawn(async move |this, cx| {
            while let Ok((key, pixels)) = results_rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.pending.remove(&key);
                        let rendered = pixels
                            .and_then(|bytes| image::RgbaImage::from_raw(key.size, key.size, bytes))
                            .map(|buffer| {
                                Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
                            });
                        if this.cache.len() >= 512 {
                            if let Some(old) = this.order.pop_front() {
                                this.cache.remove(&old);
                            }
                        }
                        this.order.push_back(key.clone());
                        this.cache.insert(key, rendered);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            cache: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            requests: tx,
            _receiver: receiver,
        }
    }

    pub fn get(
        &mut self,
        path: &Path,
        is_dir: bool,
        size: u32,
        _cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let key = Key {
            path: Some(path.to_path_buf()),
            is_dir,
            size: size.clamp(16, 256),
        };
        self.get_key(key)
    }

    pub fn get_recycle_bin(
        &mut self,
        size: u32,
        _cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        self.get_key(Key {
            path: None,
            is_dir: true,
            size: size.clamp(16, 256),
        })
    }

    fn get_key(&mut self, key: Key) -> Option<Arc<RenderImage>> {
        if let Some(image) = self.cache.get(&key) {
            return image.clone();
        }
        if !self.pending.contains(&key) && self.requests.try_send(key.clone()).is_ok() {
            self.pending.insert(key);
        }
        None
    }
}

#[cfg(not(windows))]
fn extract(_: Option<&Path>, _: bool, _: u32) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn extract(path: Option<&Path>, is_dir: bool, size: u32) -> Option<Vec<u8>> {
    use std::{
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    let recycle_bin = path.is_none();
    let path: Vec<u16> = path
        .unwrap_or_else(|| Path::new(""))
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: all handles are thread-local, owned and released below. Shell writes only
    // the supplied SHFILEINFOW. DIB buffers have size*size*4 bytes and size <= 256.
    unsafe {
        let mut info: SHFILEINFOW = zeroed();
        let flags = SHGFI_ICON
            | if size <= 20 {
                SHGFI_SMALLICON
            } else {
                SHGFI_LARGEICON
            };
        let mut ok = if recycle_bin {
            let mut stock: SHSTOCKICONINFO = zeroed();
            stock.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
            let mut bin: SHQUERYRBINFO = zeroed();
            bin.cbSize = size_of::<SHQUERYRBINFO>() as u32;
            let full = SHQueryRecycleBinW(std::ptr::null(), &mut bin) >= 0 && bin.i64NumItems > 0;
            let result = SHGetStockIconInfo(
                if full {
                    SIID_RECYCLERFULL
                } else {
                    SIID_RECYCLER
                },
                SHGSI_ICON
                    | if size <= 20 {
                        SHGSI_SMALLICON
                    } else {
                        SHGSI_LARGEICON
                    },
                &mut stock,
            );
            info.hIcon = stock.hIcon;
            usize::from(result >= 0)
        } else {
            SHGetFileInfoW(
                path.as_ptr(),
                0,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };
        if ok == 0 && !recycle_bin {
            ok = SHGetFileInfoW(
                path.as_ptr(),
                if is_dir {
                    FILE_ATTRIBUTE_DIRECTORY
                } else {
                    FILE_ATTRIBUTE_NORMAL
                },
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                flags | SHGFI_USEFILEATTRIBUTES,
            );
        }
        if ok == 0 || info.hIcon.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            DestroyIcon(info.hIcon);
            return None;
        }
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size as i32;
        bmi.bmiHeader.biHeight = -(size as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            DestroyIcon(info.hIcon);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        if old.is_null() || old as isize == -1 {
            DeleteObject(bitmap);
            DeleteDC(dc);
            DestroyIcon(info.hIcon);
            return None;
        }
        let count = (size * size * 4) as usize;
        std::ptr::write_bytes(bits, 0, count);
        let black_ok = DrawIconEx(
            dc,
            0,
            0,
            info.hIcon,
            size as i32,
            size as i32,
            0,
            null_mut(),
            DI_NORMAL,
        );
        GdiFlush();
        let black = std::slice::from_raw_parts(bits as *const u8, count).to_vec();
        std::ptr::write_bytes(bits, 255, count);
        let white_ok = DrawIconEx(
            dc,
            0,
            0,
            info.hIcon,
            size as i32,
            size as i32,
            0,
            null_mut(),
            DI_NORMAL,
        );
        GdiFlush();
        let white = std::slice::from_raw_parts(bits as *const u8, count).to_vec();
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DestroyIcon(info.hIcon);
        if black_ok == 0 || white_ok == 0 {
            return None;
        }
        Some(composite_to_bgra(&black, &white))
    }
}

/// Recover straight BGRA from black/white composites, including legacy mask icons.
/// This is alpha decoding, not a theme tint or recoloring of the OS image.
#[cfg(any(windows, test))]
fn composite_to_bgra(black: &[u8], white: &[u8]) -> Vec<u8> {
    black
        .chunks_exact(4)
        .zip(white.chunks_exact(4))
        .flat_map(|(b, w)| {
            let alpha = 255u16 - u16::from(w[0].saturating_sub(b[0]));
            if alpha == 0 {
                [0, 0, 0, 0]
            } else {
                [
                    ((u16::from(b[0]) * 255 + alpha / 2) / alpha).min(255) as u8,
                    ((u16::from(b[1]) * 255 + alpha / 2) / alpha).min(255) as u8,
                    ((u16::from(b[2]) * 255 + alpha / 2) / alpha).min(255) as u8,
                    alpha as u8,
                ]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_folder_icon_has_visible_pixels() {
        let bytes = extract(Some(&std::env::temp_dir()), true, 32).expect("Windows folder icon");
        assert_eq!(bytes.len(), 32 * 32 * 4);
        assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
    #[cfg(windows)]
    #[test]
    fn windows_recycle_bin_uses_a_visible_shell_icon() {
        let bytes = extract(None, true, 32).expect("Windows recycle bin icon");
        assert_eq!(bytes.len(), 32 * 32 * 4);
        assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
    #[test]
    fn preserves_opaque_color_and_transparent_background() {
        assert_eq!(
            composite_to_bgra(
                &[12, 34, 56, 0, 0, 0, 0, 0],
                &[12, 34, 56, 255, 255, 255, 255, 255]
            ),
            vec![12, 34, 56, 255, 0, 0, 0, 0]
        );
    }
    #[test]
    fn decodes_half_alpha_without_tint() {
        assert_eq!(
            composite_to_bgra(&[50, 25, 0, 0], &[177, 152, 127, 255]),
            vec![100, 50, 0, 128]
        );
    }
}
