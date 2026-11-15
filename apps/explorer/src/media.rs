//! Bounded media loading. All filesystem/codec/Shell work stays on worker threads.
mod decode;
pub(crate) mod quicklook;
pub(crate) mod macos_actions;
#[cfg(windows)]
mod windows;

use explorer_core::{Entry, EntryKind};
use gpui::{Context, RenderImage, Task};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

pub const TEXT_LIMIT: usize = 128 * 1024;
const CACHE_BYTES: usize = 64 * 1024 * 1024;
const CACHE_ENTRIES: usize = 384;
const QUEUE_CAPACITY: usize = 96;

/// Give each folder and preview its own scope; cancel on navigation/preview close.
#[derive(Clone)]
pub struct MediaScope {
    id: u64,
    generation: Arc<AtomicU64>,
}

impl MediaScope {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for MediaScope {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub enum PreviewContent {
    Loading,
    Image(Arc<RenderImage>),
    Text { text: Arc<str>, truncated: bool },
    Metadata { reason: Arc<str> },
}

pub fn is_image_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "ico"
            | "avif"
            | "heic"
            | "heif"
            | "jxl"
            | "svg"
            | "psd"
            | "dng"
            | "cr2"
            | "cr3"
            | "nef"
            | "arw"
            | "orf"
            | "rw2"
    )
}

pub fn should_use_gallery<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> bool {
    let mut files = 0usize;
    let mut images = 0usize;
    for entry in entries {
        if entry.kind == EntryKind::File {
            files += 1;
            images += usize::from(is_image_path(&entry.path));
        }
    }
    gallery_ratio(images, files)
}

fn gallery_ratio(images: usize, files: usize) -> bool {
    files != 0 && (images as u128) * 5 >= (files as u128) * 4
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Kind {
    Thumbnail(u32),
    Preview,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: Option<u64>,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Stage {
    Fast,
    Shell,
}

#[derive(Clone)]
struct Job {
    key: Key,
    scope: MediaScope,
    generation: u64,
    stage: Stage,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct JobId {
    key: Key,
    scope: u64,
    generation: u64,
    stage: Stage,
}

impl Job {
    fn id(&self) -> JobId {
        JobId {
            key: self.key.clone(),
            scope: self.scope.id,
            generation: self.generation,
            stage: self.stage,
        }
    }
    fn current(&self) -> bool {
        self.scope.generation.load(Ordering::Relaxed) == self.generation
    }
}

struct Queue {
    jobs: VecDeque<Job>,
    active: HashSet<JobId>,
    closed: bool,
    capacity: usize,
}

impl Queue {
    fn new(capacity: usize) -> Self {
        Self {
            jobs: VecDeque::new(),
            active: HashSet::new(),
            closed: false,
            capacity,
        }
    }
    fn push(&mut self, job: Job) {
        if self.closed || self.capacity == 0 || !job.current() || self.active.contains(&job.id()) {
            return;
        }
        self.jobs
            .retain(|queued| queued.current() && queued.id() != job.id());
        if self.jobs.len() >= self.capacity {
            let oldest_thumbnail = self
                .jobs
                .iter()
                .position(|queued| matches!(queued.key.kind, Kind::Thumbnail(_)))
                .unwrap_or(0);
            self.jobs.remove(oldest_thumbnail);
        }
        self.jobs.push_back(job);
    }
    fn pop(&mut self) -> Option<Job> {
        self.jobs.retain(Job::current);
        let index = self
            .jobs
            .iter()
            .rposition(|job| job.key.kind == Kind::Preview)
            .or_else(|| self.jobs.len().checked_sub(1))?;
        let job = self.jobs.remove(index)?;
        self.active.insert(job.id());
        Some(job)
    }
    fn complete(&mut self, job: &Job) {
        self.active.remove(&job.id());
    }
}

struct WorkerQueue {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl WorkerQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Queue::new(QUEUE_CAPACITY)),
            wake: Condvar::new(),
        }
    }
    fn push(&self, job: Job) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(job);
        self.wake.notify_one();
    }
    fn next(&self) -> Option<Job> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if queue.closed {
                return None;
            }
            if let Some(job) = queue.pop() {
                return Some(job);
            }
            queue = self.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
    }
    fn complete(&self, job: &Job) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .complete(job);
    }
    fn close(&self) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.closed = true;
        queue.jobs.clear();
        self.wake.notify_all();
    }
}

struct Cached {
    value: PreviewContent,
    complete: bool,
    bytes: usize,
    used: u64,
}

pub struct MediaCache {
    entries: HashMap<Key, Cached>,
    bytes: usize,
    clock: u64,
    fast: Arc<WorkerQueue>,
    shell: Arc<WorkerQueue>,
    _receiver: Task<()>,
}

struct Completed {
    job: Job,
    value: Option<PreviewContent>,
    final_stage: bool,
}

impl MediaCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let fast = Arc::new(WorkerQueue::new());
        let shell = Arc::new(WorkerQueue::new());
        // Small result capacity also bounds decoded images waiting for the UI.
        let (tx, rx) = async_channel::bounded::<Completed>(4);
        let fast_queue = fast.clone();
        let shell_queue = shell.clone();
        let fast_tx = tx.clone();
        if std::thread::Builder::new()
            .name("media-decode".into())
            .spawn(move || {
                while let Some(job) = fast_queue.next() {
                    if !job.current() {
                        fast_queue.complete(&job);
                        continue;
                    }
                    let (value, needs_shell) = load_fast(&job);
                    if job.current() {
                        if fast_tx
                            .send_blocking(Completed {
                                job: job.clone(),
                                value: Some(value),
                                final_stage: !needs_shell,
                            })
                            .is_err()
                        {
                            break;
                        }
                        if needs_shell {
                            let mut next = job.clone();
                            next.stage = Stage::Shell;
                            shell_queue.push(next);
                        }
                    } else {
                        fast_queue.complete(&job);
                    }
                }
            })
            .is_err()
        {
            fast.close();
        }
        #[cfg(windows)]
        {
            let shell_queue = shell.clone();
            if std::thread::Builder::new()
                .name("media-shell".into())
                .spawn(move || {
                    use windows_sys::Win32::System::Com::{
                        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
                    };
                    let initialized = unsafe {
                        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32)
                    } >= 0;
                    while let Some(job) = shell_queue.next() {
                        if !job.current() {
                            shell_queue.complete(&job);
                            continue;
                        }
                        let edge = match job.key.kind {
                            Kind::Thumbnail(edge) => edge,
                            Kind::Preview => 1024,
                        };
                        let value = initialized
                            .then(|| windows::thumbnail(&job.key.path, edge))
                            .flatten()
                            .map(render_image);
                        if job.current() {
                            if tx
                                .send_blocking(Completed {
                                    job: job.clone(),
                                    value,
                                    final_stage: true,
                                })
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            shell_queue.complete(&job);
                        }
                    }
                    if initialized {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                })
                .is_err()
            {
                shell.close();
            }
        }
        #[cfg(not(windows))]
        drop(tx);
        let receiver = cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let mut batch = vec![first];
                while batch.len() < 8 {
                    match rx.try_recv() {
                        Ok(result) => batch.push(result),
                        Err(_) => break,
                    }
                }
                if this
                    .update(cx, |this, cx| {
                        let mut changed = false;
                        for completed in batch {
                            match completed.job.stage {
                                Stage::Fast => this.fast.complete(&completed.job),
                                Stage::Shell => this.shell.complete(&completed.job),
                            }
                            if completed.job.current() {
                                this.accept(completed);
                                changed = true;
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            fast,
            shell,
            _receiver: receiver,
        }
    }

    pub fn get_thumbnail(
        &mut self,
        entry: &Entry,
        edge: u32,
        scope: &MediaScope,
        _cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        if entry.kind != EntryKind::File || !is_image_path(&entry.path) {
            return None;
        }
        match self.get(entry, Kind::Thumbnail(edge.clamp(64, 512)), scope) {
            PreviewContent::Image(image) => Some(image),
            _ => None,
        }
    }

    pub fn get_preview(
        &mut self,
        entry: &Entry,
        scope: &MediaScope,
        _cx: &mut Context<Self>,
    ) -> PreviewContent {
        if entry.kind != EntryKind::File {
            return PreviewContent::Metadata {
                reason: "フォルダーやリンクは詳細情報を表示します".into(),
            };
        }
        self.get(entry, Kind::Preview, scope)
    }

    fn get(&mut self, entry: &Entry, kind: Kind, scope: &MediaScope) -> PreviewContent {
        let key = Key {
            path: entry.path.clone(),
            modified: entry.modified,
            len: entry.size,
            kind,
        };
        let mut job = Job {
            key: key.clone(),
            scope: scope.clone(),
            generation: scope.generation.load(Ordering::Relaxed),
            stage: Stage::Fast,
        };
        self.clock += 1;
        if let Some(cached) = self.entries.get_mut(&key) {
            cached.used = self.clock;
            if !cached.complete {
                job.stage = Stage::Shell;
                self.shell.push(job);
            }
            return cached.value.clone();
        }
        if self
            .fast
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed
        {
            return PreviewContent::Metadata {
                reason: "プレビューのワーカーを開始できませんでした".into(),
            };
        }
        self.fast.push(job);
        PreviewContent::Loading
    }

    fn accept(&mut self, completed: Completed) {
        let key = completed.job.key;
        // A late fast result must never replace an already refined image.
        if completed.job.stage == Stage::Fast
            && self.entries.get(&key).is_some_and(|cached| cached.complete)
        {
            return;
        }
        let mut value = completed
            .value
            .or_else(|| self.entries.get(&key).map(|cached| cached.value.clone()))
            .unwrap_or_else(|| PreviewContent::Metadata {
                reason: "この形式のプレビューを生成できませんでした".into(),
            });
        if let (PreviewContent::Image(new), Some(old)) = (&value, self.entries.get(&key)) {
            if let PreviewContent::Image(previous) = &old.value {
                if new.as_bytes(0).map_or(0, <[u8]>::len)
                    < previous.as_bytes(0).map_or(0, <[u8]>::len)
                {
                    value = old.value.clone();
                }
            }
        }
        let bytes = content_bytes(&value);
        if bytes > CACHE_BYTES {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, value)| value.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes;
            }
        }
        self.clock += 1;
        self.bytes += bytes;
        self.entries.insert(
            key,
            Cached {
                value,
                complete: completed.final_stage,
                bytes,
                used: self.clock,
            },
        );
    }
}

impl Drop for MediaCache {
    fn drop(&mut self) {
        self.fast.close();
        self.shell.close();
    }
}

fn content_bytes(value: &PreviewContent) -> usize {
    match value {
        PreviewContent::Image(image) => image.as_bytes(0).map_or(0, <[u8]>::len),
        PreviewContent::Text { text, .. } => text.len(),
        PreviewContent::Metadata { reason } => reason.len(),
        PreviewContent::Loading => 0,
    }
}

fn render_image(mut image: image::RgbaImage) -> PreviewContent {
    // GPUI RenderImage consumes straight BGRA, despite image::RgbaImage's name.
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    PreviewContent::Image(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

fn load_fast(job: &Job) -> (PreviewContent, bool) {
    let fallback = |reason: String| {
        (
            PreviewContent::Metadata {
                reason: reason.into(),
            },
            false,
        )
    };
    let metadata = match std::fs::symlink_metadata(&job.key.path) {
        Ok(metadata) => metadata,
        Err(e) => return fallback(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return fallback("通常のファイルのみプレビューできます".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Avoid hydrating cloud placeholders or traversing reparse points implicitly.
        if metadata.file_attributes() & 0x400 != 0 {
            return fallback("リンク・クラウド上のファイルは既定のアプリで開いてください".into());
        }
    }
    if !job.current() {
        return fallback("キャンセルしました".into());
    }
    if is_image_path(&job.key.path) {
        let edge = match job.key.kind {
            Kind::Thumbnail(edge) => {
                if cfg!(windows) {
                    edge.min(160)
                } else {
                    edge
                }
            }
            Kind::Preview => 1600,
        };
        match decode::image(&job.key.path, edge) {
            Ok(image) => (
                render_image(image),
                cfg!(windows) && matches!(job.key.kind, Kind::Thumbnail(_)),
            ),
            Err(reason) => (
                PreviewContent::Metadata {
                    reason: reason.into(),
                },
                cfg!(windows),
            ),
        }
    } else if job.key.kind == Kind::Preview && is_text_path(&job.key.path) {
        match std::fs::File::open(&job.key.path).and_then(decode::text) {
            Ok(Some((text, truncated))) => (
                PreviewContent::Text {
                    text: text.into(),
                    truncated,
                },
                false,
            ),
            Ok(None) => {
                fallback("この文字コード・バイナリ形式は内蔵プレビューに対応していません".into())
            }
            Err(error) => fallback(error.to_string()),
        }
    } else {
        fallback("この形式は既定のアプリで開いてください".into())
    }
}

fn is_text_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "json"
            | "html"
            | "htm"
            | "css"
            | "csv"
            | "tsv"
            | "toml"
            | "yaml"
            | "yml"
            | "xml"
            | "log"
            | "ini"
            | "c"
            | "cpp"
            | "h"
            | "py"
            | "sh"
            | "bat"
            | "ps1"
            | "srt"
    ) || matches!(
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "readme" | "license" | "dockerfile" | "makefile"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    fn job(name: &str, scope: &MediaScope, kind: Kind) -> Job {
        Job {
            key: Key {
                path: name.into(),
                modified: None,
                len: None,
                kind,
            },
            scope: scope.clone(),
            generation: scope.generation.load(Ordering::Relaxed),
            stage: Stage::Fast,
        }
    }
    #[test]
    fn gallery_threshold_requires_files_and_includes_exactly_eighty_percent() {
        assert!(!gallery_ratio(0, 0));
        assert!(!gallery_ratio(3, 4));
        assert!(gallery_ratio(4, 5));
        assert!(gallery_ratio(1, 1));
        assert!(!gallery_ratio(0, 10));
        assert!(is_image_path(Path::new("写真.JPEG")));
        assert!(!is_image_path(Path::new("picture.jpg.exe")));
    }
    #[test]
    fn new_demand_replaces_old_queue_and_preview_goes_first() {
        let scope = MediaScope::new();
        let mut queue = Queue::new(2);
        queue.push(job("a", &scope, Kind::Thumbnail(128)));
        queue.push(job("b", &scope, Kind::Preview));
        queue.push(job("c", &scope, Kind::Thumbnail(128)));
        assert_eq!(queue.jobs.len(), 2);
        assert_eq!(queue.pop().unwrap().key.path, PathBuf::from("b"));
        assert_eq!(queue.pop().unwrap().key.path, PathBuf::from("c"));
    }
    #[test]
    fn cancelled_scopes_discard_queue_and_allow_same_file_in_new_generation() {
        let scope = MediaScope::new();
        let mut queue = Queue::new(2);
        queue.push(job("a", &scope, Kind::Preview));
        let old = queue.pop().unwrap();
        scope.cancel();
        queue.push(job("a", &scope, Kind::Preview));
        let new = queue.pop().unwrap();
        assert!(!old.current());
        assert!(new.current());
        queue.complete(&old);
        queue.complete(&new);
        queue.push(job("b", &scope, Kind::Preview));
        scope.cancel();
        assert!(queue.pop().is_none());
    }

    #[gpui::test]
    fn cache_budget_is_bounded_and_late_fast_results_keep_refined_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        // Pure cache test: do not start OS workers under GPUI's deterministic
        // scheduler, which rejects nondeterministic worker wakeups in full suites.
        let cache = cx.new(|cx| MediaCache {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            fast: Arc::new(WorkerQueue::new()),
            shell: Arc::new(WorkerQueue::new()),
            _receiver: cx.spawn(async |_, _| {}),
        });
        let scope = MediaScope::new();
        cache.update(cx, |cache, _| {
            let value = render_image(image::RgbaImage::new(512, 512));
            for i in 0..100 {
                cache.accept(Completed {
                    job: job(&i.to_string(), &scope, Kind::Thumbnail(512)),
                    value: Some(value.clone()),
                    final_stage: true,
                });
            }
            assert!(cache.bytes <= CACHE_BYTES);
            assert!(cache.entries.len() <= CACHE_ENTRIES);
            let mut refined = job("latest", &scope, Kind::Thumbnail(512));
            refined.stage = Stage::Shell;
            cache.accept(Completed {
                job: refined.clone(),
                value: Some(value.clone()),
                final_stage: true,
            });
            let PreviewContent::Image(expected) = value else {
                unreachable!()
            };
            let mut fast = refined.clone();
            fast.stage = Stage::Fast;
            cache.accept(Completed {
                job: fast,
                value: Some(render_image(image::RgbaImage::new(160, 160))),
                final_stage: false,
            });
            let PreviewContent::Image(actual) = &cache.entries[&refined.key].value else {
                panic!("image missing")
            };
            assert!(Arc::ptr_eq(&expected, actual));
            cache.accept(Completed {
                job: refined.clone(),
                value: None,
                final_stage: true,
            });
            let PreviewContent::Image(actual) = &cache.entries[&refined.key].value else {
                panic!("failed refinement discarded fallback")
            };
            assert!(Arc::ptr_eq(&expected, actual));
        });
    }

    #[test]
    fn bgra_upload_preserves_red_and_alpha() {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 128]));
        let PreviewContent::Image(image) = render_image(image) else {
            unreachable!()
        };
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 128]);
    }
}
