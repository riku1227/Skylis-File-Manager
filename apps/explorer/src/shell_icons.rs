//! OS shell images, fetched on a bounded worker, never during UI layout/paint.
use gpui::{Context, RenderImage, Task};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    // None is the shell's virtual recycle bin, not a fake directory path.
    path: Option<PathBuf>,
    is_dir: bool,
    size: u32,
    attributes_only: bool,
}

impl Key {
    fn file_type(path: &Path, is_dir: bool, size: u32) -> Self {
        // Never pass the real path for a type lookup. SHGFI_USEFILEATTRIBUTES uses
        // registered file associations, without opening the file or contacting a share.
        let mut name = PathBuf::from(if is_dir { "folder" } else { "file" });
        if !is_dir && let Some(extension) = path.extension() {
            name.set_extension(extension.to_ascii_lowercase());
        }
        Self {
            path: Some(name),
            is_dir,
            size: size.clamp(16, 256),
            attributes_only: true,
        }
    }
}

fn type_only(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..))
    )
}

pub struct ShellIcons {
    cache: IconCache,
    requests: Arc<WorkerRequests>,
    _receiver: Task<()>,
}

// Bound both pixel storage and negative entries. Cache hits retain the same RenderImage ID,
// so new icons never invalidate images that GPUI has already uploaded to the GPU.
struct CachedIcon {
    image: Option<Arc<RenderImage>>,
    touched: u64,
    bytes: usize,
}

struct IconCache {
    entries: HashMap<Key, CachedIcon>,
    recency: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl IconCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            recency: BTreeMap::new(),
            clock: 0,
            bytes: 0,
            max_entries,
            max_bytes,
        }
    }

    fn get(&mut self, key: &Key) -> Option<Option<Arc<RenderImage>>> {
        let entry = self.entries.get_mut(key)?;
        self.recency.remove(&entry.touched);
        self.clock += 1;
        entry.touched = self.clock;
        self.recency.insert(self.clock, key.clone());
        Some(entry.image.clone())
    }

    fn insert(&mut self, key: Key, image: Option<Arc<RenderImage>>) {
        if let Some(old) = self.entries.remove(&key) {
            self.recency.remove(&old.touched);
            self.bytes -= old.bytes;
        }
        let bytes = image
            .as_ref()
            .and_then(|image| image.as_bytes(0))
            .map_or(0, <[u8]>::len);
        if bytes > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while self.entries.len() >= self.max_entries || self.bytes + bytes > self.max_bytes {
            let Some((_, oldest)) = self.recency.pop_first() else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes;
            }
        }
        self.clock += 1;
        self.bytes += bytes;
        self.recency.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            CachedIcon {
                image,
                touched: self.clock,
                bytes,
            },
        );
    }
}

// Each render refreshes demand for its visible icons. A bounded, newest-first queue lets
// the current viewport jump ahead of old scroll positions without creating more threads.
struct RequestQueue {
    queued: VecDeque<Key>,
    in_flight: HashSet<Key>,
    capacity: usize,
    closed: bool,
}

impl RequestQueue {
    fn new(capacity: usize) -> Self {
        Self {
            queued: VecDeque::new(),
            in_flight: HashSet::new(),
            capacity,
            closed: false,
        }
    }

    fn request(&mut self, key: Key) {
        if self.closed || self.capacity == 0 || self.in_flight.contains(&key) {
            return;
        }
        if let Some(index) = self.queued.iter().position(|queued| queued == &key) {
            self.queued.remove(index);
        } else if self.queued.len() == self.capacity {
            self.queued.pop_front();
        }
        self.queued.push_back(key);
    }

    fn take(&mut self) -> Option<Key> {
        let key = self.queued.pop_back()?;
        self.in_flight.insert(key.clone());
        Some(key)
    }

    fn complete(&mut self, key: &Key) {
        self.in_flight.remove(key);
    }
}

struct WorkerRequests {
    queue: Mutex<RequestQueue>,
    wake: Condvar,
}

impl WorkerRequests {
    fn next(&self) -> Option<Key> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if queue.closed {
                return None;
            }
            if let Some(key) = queue.take() {
                return Some(key);
            }
            queue = self
                .wake
                .wait(queue)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

impl Drop for ShellIcons {
    fn drop(&mut self) {
        let mut queue = self
            .requests
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.closed = true;
        queue.queued.clear();
        self.requests.wake.notify_one();
    }
}

impl ShellIcons {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let requests = Arc::new(WorkerRequests {
            queue: Mutex::new(RequestQueue::new(128)),
            wake: Condvar::new(),
        });
        let worker_requests = requests.clone();
        let (results_tx, results_rx) = async_channel::bounded(16);
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
                while let Some(key) = worker_requests.next() {
                    let pixels = extract(
                        key.path.as_deref(),
                        key.is_dir,
                        key.size,
                        key.attributes_only,
                    );
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
            while let Ok(first) = results_rx.recv().await {
                // At most one short frame fraction of latency: coalesce fast completions,
                // rather than waking every FolderView once for every shell icon.
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let mut batch = vec![first];
                while batch.len() < 32 {
                    match results_rx.try_recv() {
                        Ok(result) => batch.push(result),
                        Err(_) => break,
                    }
                }
                if this
                    .update(cx, |this, cx| {
                        for (key, pixels) in batch {
                            this.requests
                                .queue
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .complete(&key);
                            let rendered = pixels
                                .and_then(|bytes| {
                                    image::RgbaImage::from_raw(key.size, key.size, bytes)
                                })
                                .map(|buffer| {
                                    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
                                });
                            this.cache.insert(key, rendered);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            cache: IconCache::new(2048, 16 * 1024 * 1024),
            requests,
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
        let fallback = Key::file_type(path, is_dir, size);
        // Network/WSL icons deliberately avoid per-file shell extensions: one slow
        // network lookup must not hold the worker that serves local visible icons.
        if type_only(path) {
            return self.get_key(fallback);
        }
        let key = Key {
            path: Some(path.to_path_buf()),
            is_dir,
            size: size.clamp(16, 256),
            attributes_only: false,
        };
        if let Some(image) = self.cache.get(&key) {
            return image.or_else(|| self.get_key(fallback));
        }
        // Publish both requests under one lock: the worker must see the cheap type
        // lookup before it can start an exact lookup that invokes a slow extension.
        // A ready exact icon remains stable; only the fallback is replaced.
        let fallback_image = self.cache.get(&fallback);
        {
            let mut queue = self
                .requests
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            queue.request(key);
            if fallback_image.is_none() {
                queue.request(fallback);
            }
        }
        self.requests.wake.notify_one();
        fallback_image.flatten()
    }

    pub fn get_type(&mut self, path: &Path, is_dir: bool, size: u32) -> Option<Arc<RenderImage>> {
        self.get_key(Key::file_type(path, is_dir, size))
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
            attributes_only: false,
        })
    }

    fn get_key(&mut self, key: Key) -> Option<Arc<RenderImage>> {
        if let Some(image) = self.cache.get(&key) {
            return image;
        }
        self.request(key);
        None
    }

    fn request(&self, key: Key) {
        self.requests
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .request(key);
        self.requests.wake.notify_one();
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn extract(_: Option<&Path>, _: bool, _: u32, _: bool) -> Option<Vec<u8>> {
    None
}

#[cfg(target_os = "macos")]
fn extract(path: Option<&Path>, is_dir: bool, size: u32, attributes_only: bool) -> Option<Vec<u8>> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn skylis_macos_icon(
            path: *const std::ffi::c_char,
            directory: i32,
            size: u32,
            type_only: i32,
            pixels: *mut u8,
        ) -> i32;
    }
    // The Windows virtual recycle bin has no equivalent in this adapter.
    let path = CString::new(path?.as_os_str().as_bytes()).ok()?;
    if !(16..=256).contains(&size) {
        return None;
    }
    let mut pixels = vec![0; size as usize * size as usize * 4];
    // SAFETY: NUL-terminated path and a size-bounded writable BGRA buffer. The
    // bridge drains an autorelease pool and retains no pointer after this call.
    let ok = unsafe {
        skylis_macos_icon(
            path.as_ptr(),
            is_dir.into(),
            size,
            attributes_only.into(),
            pixels.as_mut_ptr(),
        )
    };
    (ok == 1).then_some(pixels)
}

#[cfg(windows)]
fn extract(path: Option<&Path>, is_dir: bool, size: u32, attributes_only: bool) -> Option<Vec<u8>> {
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
                if attributes_only {
                    if is_dir {
                        FILE_ATTRIBUTE_DIRECTORY
                    } else {
                        FILE_ATTRIBUTE_NORMAL
                    }
                } else {
                    0
                },
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                flags
                    | if attributes_only {
                        SHGFI_USEFILEATTRIBUTES
                    } else {
                        0
                    },
            )
        };
        if ok == 0 && !recycle_bin && !attributes_only {
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
        if attributes_only && (ok == 0 || info.hIcon.is_null()) {
            // Association handlers are optional and can fail transiently. Keep the
            // type placeholder a real OS icon even when that handler is unavailable.
            if !info.hIcon.is_null() {
                DestroyIcon(info.hIcon);
            }
            let mut stock: SHSTOCKICONINFO = zeroed();
            stock.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
            let result = SHGetStockIconInfo(
                if is_dir { SIID_FOLDER } else { SIID_DOCNOASSOC },
                SHGSI_ICON
                    | if size <= 20 {
                        SHGSI_SMALLICON
                    } else {
                        SHGSI_LARGEICON
                    },
                &mut stock,
            );
            info.hIcon = stock.hIcon;
            ok = usize::from(result >= 0);
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

    fn key(name: &str) -> Key {
        Key {
            path: Some(name.into()),
            is_dir: false,
            size: 16,
            attributes_only: false,
        }
    }

    fn image() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(16, 16),
        )]))
    }

    #[test]
    fn visible_hits_keep_the_same_image_and_evict_offscreen_entries() {
        let mut cache = IconCache::new(2, 4096);
        let visible = image();
        cache.insert(key("visible"), Some(visible.clone()));
        cache.insert(key("offscreen"), Some(image()));
        assert!(Arc::ptr_eq(
            &visible,
            &cache.get(&key("visible")).unwrap().unwrap()
        ));
        cache.insert(key("new"), Some(image()));
        assert!(cache.get(&key("offscreen")).is_none());
        assert!(Arc::ptr_eq(
            &visible,
            &cache.get(&key("visible")).unwrap().unwrap()
        ));
    }

    #[test]
    fn cache_bounds_pixel_memory_and_caches_failures() {
        let mut cache = IconCache::new(8, 2048);
        cache.insert(key("a"), Some(image()));
        cache.insert(key("b"), Some(image()));
        cache.insert(key("c"), Some(image()));
        assert!(cache.get(&key("a")).is_none());
        assert_eq!(cache.bytes, 2048);
        cache.insert(key("missing"), None);
        assert!(matches!(cache.get(&key("missing")), Some(None)));
        assert_eq!(cache.bytes, 2048);
    }

    #[test]
    fn latest_demand_replaces_stale_work_without_duplicate_extraction() {
        let mut queue = RequestQueue::new(2);
        queue.request(key("old"));
        queue.request(key("visible"));
        queue.request(key("visible"));
        queue.request(key("new"));
        assert_eq!(queue.queued.len(), 2);
        assert_eq!(queue.take(), Some(key("new")));
        queue.request(key("new"));
        assert_eq!(queue.take(), Some(key("visible")));
        assert_eq!(queue.take(), None);
        queue.complete(&key("new"));
        queue.request(key("new"));
        assert_eq!(queue.take(), Some(key("new")));
    }

    #[test]
    fn refreshed_request_is_processed_before_obsolete_requests() {
        let mut queue = RequestQueue::new(4);
        queue.request(key("visible"));
        queue.request(key("obsolete"));
        queue.request(key("visible"));
        assert_eq!(queue.take(), Some(key("visible")));
    }

    #[test]
    fn unsupported_icons_cannot_grow_cache_without_bound() {
        let mut cache = IconCache::new(4, 4096);
        for index in 0..100 {
            cache.insert(key(&index.to_string()), None);
        }
        assert_eq!(cache.entries.len(), 4);
        assert_eq!(cache.recency.len(), 4);
        assert_eq!(cache.bytes, 0);
        assert!(matches!(cache.get(&key("99")), Some(None)));
        assert!(cache.get(&key("0")).is_none());
    }

    #[test]
    fn closing_worker_drops_pending_work_and_wakes_an_idle_thread() {
        let requests = Arc::new(WorkerRequests {
            queue: Mutex::new(RequestQueue::new(128)),
            wake: Condvar::new(),
        });
        let worker = requests.clone();
        let thread = std::thread::spawn(move || worker.next());
        {
            let mut queue = requests.queue.lock().unwrap();
            queue.closed = true;
            requests.wake.notify_one();
        }
        assert_eq!(thread.join().unwrap(), None);
        let mut queue = requests.queue.lock().unwrap();
        queue.request(key("late"));
        assert!(queue.queued.is_empty());
    }

    #[test]
    fn type_fallbacks_share_extensions_but_exact_icons_keep_file_identity() {
        let first = Key::file_type(Path::new("one/REPORT.PDF"), false, 16);
        let second = Key::file_type(Path::new("other/report.pdf"), false, 16);
        assert_eq!(first, second);
        assert!(first.attributes_only);
        assert_ne!(key("one/report.pdf"), key("other/report.pdf"));
        assert_ne!(first, Key::file_type(Path::new("report.txt"), false, 16));
        assert_ne!(first, Key::file_type(Path::new("report.pdf"), false, 32));
        assert_eq!(
            Key::file_type(Path::new("one.folder"), true, 16),
            Key::file_type(Path::new("two"), true, 16),
        );
    }

    #[cfg(windows)]
    #[test]
    fn remote_paths_use_type_icons_without_contacting_network_shell_handlers() {
        for path in [
            r"\\server\share\file.txt",
            r"\\wsl.localhost\Ubuntu\home",
            r"\\?\UNC\server\share\file",
        ] {
            assert!(type_only(Path::new(path)), "{path}");
        }
        for path in [r"C:\Users\file.txt", r"\\?\C:\Users\file.txt"] {
            assert!(!type_only(Path::new(path)), "{path}");
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_worker_icons_are_visible_bounded_and_support_type_fallbacks() {
        std::thread::spawn(|| {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("日本語 folder");
            std::fs::create_dir(&folder).unwrap();
            let file = folder.join("file name.txt");
            std::fs::write(&file, b"native icon fixture").unwrap();
            let extensionless = folder.join("no extension");
            std::fs::write(&extensionless, b"fixture").unwrap();
            for (path, directory) in [(&folder, true), (&file, false), (&extensionless, false)] {
                let pixels =
                    extract(Some(path), directory, 32, false).expect("existing native icon");
                assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
            }
            for (path, directory, type_only) in [
                ("/", true, false),
                ("/Applications", true, false),
                ("/nonexistent/日本語 folder/file name.txt", false, true),
                ("/nonexistent/日本語 folder/no extension", false, true),
                ("/nonexistent/folder", true, true),
                ("/nonexistent/file.unknown_skylis", false, true),
            ] {
                for size in [16, 32, 256] {
                    let pixels =
                        extract(Some(Path::new(path)), directory, size, type_only).expect(path);
                    assert_eq!(pixels.len(), size as usize * size as usize * 4);
                    assert!(pixels.chunks_exact(4).any(|p| p[3] > 0), "{path}");
                }
            }
            assert!(extract(None, true, 32, false).is_none());
            assert!(extract(Some(Path::new("/a\0b")), false, 32, false).is_none());
            assert!(extract(Some(Path::new("/")), true, 257, false).is_none());
        })
        .join()
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_folder_icon_has_visible_pixels() {
        let bytes =
            extract(Some(&std::env::temp_dir()), true, 32, false).expect("Windows folder icon");
        assert_eq!(bytes.len(), 32 * 32 * 4);
        assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
    #[cfg(windows)]
    #[test]
    fn windows_recycle_bin_uses_a_visible_shell_icon() {
        let bytes = extract(None, true, 32, false).expect("Windows recycle bin icon");
        assert_eq!(bytes.len(), 32 * 32 * 4);
        assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
    #[cfg(windows)]
    #[test]
    fn windows_type_icons_do_not_require_an_existing_file_or_folder() {
        // Match the worker's STA; registered file association handlers may require COM.
        let initialized = unsafe {
            windows_sys::Win32::System::Com::CoInitializeEx(
                std::ptr::null(),
                windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
            )
        } >= 0;
        for (name, is_dir) in [
            ("__skylis_nonexistent_type__.txt", false),
            ("__skylis_nonexistent_folder__", true),
        ] {
            let bytes = extract(Some(Path::new(name)), is_dir, 32, true)
                .unwrap_or_else(|| panic!("Windows type icon for {name}"));
            assert_eq!(bytes.len(), 32 * 32 * 4);
            assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0));
        }
        if initialized {
            unsafe {
                windows_sys::Win32::System::Com::CoUninitialize();
            }
        }
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
