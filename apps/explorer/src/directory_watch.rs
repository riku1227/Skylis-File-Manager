//! Non-recursive native directory notifications. Keep a watch across same-directory
//! reloads: the first Changed reconciles changes made while native setup was in flight.
//! No directory scans or watcher construction/destruction happen on the UI thread.

use async_channel::{Receiver, Sender, TrySendError};
use explorer_core::CancelToken;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(200);
const MAX_DELAY: Duration = Duration::from_secs(1);
const CANCEL_CHECK: Duration = Duration::from_millis(250);
const MAX_WATCHERS: usize = 32;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Eq, PartialEq)]
pub enum WatchEvent {
    Changed,
    /// Native monitoring stopped or could not start. The view remains usable with F5.
    Unavailable(String),
}

/// Dropping this handle only signals cancellation; native cleanup stays on its worker.
/// Use a navigation-lifetime CancelToken, not the token replaced by each listing reload.
pub struct WatchHandle {
    shared: Arc<Shared>,
}

impl WatchHandle {
    pub fn cancel(&self) {
        self.shared.state.lock().unwrap().stopped = true;
        self.shared.wake.notify_one();
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The event channel and native callback mailbox each coalesce pending changes. Native
/// setup errors arrive asynchronously as Unavailable; no polling watcher is substituted.
pub fn watch(
    path: PathBuf,
    cancel: CancelToken,
) -> Result<(WatchHandle, Receiver<WatchEvent>), String> {
    validate_path(&path)?;
    if cancel.is_cancelled() {
        return Err("フォルダー監視は取り消されました".into());
    }
    let slot = WatchSlot::acquire()?;
    let shared = Arc::new(Shared::default());
    let (sender, receiver) = async_channel::bounded(1);
    let worker_shared = shared.clone();
    std::thread::Builder::new()
        .name("directory-watch".into())
        .spawn(move || {
            let _slot = slot;
            run(path, cancel, worker_shared, sender);
        })
        .map_err(|error| format!("自動更新を開始できません。F5で更新してください: {error}"))?;
    Ok((WatchHandle { shared }, receiver))
}

struct WatchSlot;
impl WatchSlot {
    fn acquire() -> Result<Self, String> {
        let mut active = ACTIVE.load(Ordering::Acquire);
        loop {
            if active >= MAX_WATCHERS {
                return Err("自動更新の上限に達しました。このタブはF5で更新してください".into());
            }
            match ACTIVE.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(current) => active = current,
            }
        }
    }
}
impl Drop for WatchSlot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct Pending {
    first: Instant,
    last: Instant,
}

impl Pending {
    fn deadline(self) -> Instant {
        (self.last + DEBOUNCE).min(self.first + MAX_DELAY)
    }
}

#[derive(Default)]
struct Mailbox {
    stopped: bool,
    pending: Option<Pending>,
    error: Option<String>,
}

impl Mailbox {
    fn changed(&mut self, now: Instant) {
        if self.stopped {
            return;
        }
        match &mut self.pending {
            Some(pending) => pending.last = now,
            None => {
                self.pending = Some(Pending {
                    first: now,
                    last: now,
                })
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<Mailbox>,
    wake: Condvar,
}

impl Shared {
    fn event(&self, result: notify::Result<notify::Event>) {
        let mut state = self.state.lock().unwrap();
        if state.stopped {
            return;
        }
        match result {
            // Reading a preview or enumerating the view must not trigger a reload loop.
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
            Ok(_) => state.changed(Instant::now()),
            Err(error) => {
                state.error.get_or_insert_with(|| {
                    format!("自動更新が停止しました。F5で更新してください: {error}")
                });
            }
        }
        self.wake.notify_one();
    }
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("フォルダー監視には有効な絶対パスが必要です".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        ) {
            return Err("ネットワーク・WSLのフォルダーはF5で更新してください".into());
        }
    }
    Ok(())
}

fn check_local_drive(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            if let Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) = prefix.kind() {
                let root = [drive as u16, b':' as u16, b'\\' as u16, 0];
                // DRIVE_REMOTE = 4. This query and any native setup stay on the worker.
                if unsafe { GetDriveTypeW(root.as_ptr()) } == 4 {
                    return Err("ネットワークドライブはF5で更新してください".into());
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

fn cancelled(shared: &Shared, cancel: &CancelToken, sender: &Sender<WatchEvent>) -> bool {
    cancel.is_cancelled() || sender.is_closed() || shared.state.lock().unwrap().stopped
}

fn run(path: PathBuf, cancel: CancelToken, shared: Arc<Shared>, sender: Sender<WatchEvent>) {
    if cancelled(&shared, &cancel, &sender) {
        return;
    }
    let callback = shared.clone();
    let setup = || -> Result<RecommendedWatcher, String> {
        check_local_drive(&path)?;
        let mut watcher = RecommendedWatcher::new(
            move |event| callback.event(event),
            notify::Config::default().with_follow_symlinks(false),
        )
        .map_err(|error| format!("自動更新を開始できません。F5で更新してください: {error}"))?;
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|error| format!("このフォルダーはF5で更新してください: {error}"))?;
        Ok(watcher)
    };
    let _watcher = match setup() {
        Ok(watcher) => watcher,
        Err(error) => {
            send_error(error, &shared, &cancel, &sender);
            return;
        }
    };
    if cancelled(&shared, &cancel, &sender) {
        return;
    }
    // Registration is asynchronous with respect to the first listing. Reconcile once
    // after it succeeds so an edit in that gap cannot leave a permanently stale view.
    let _ = sender.try_send(WatchEvent::Changed);
    event_loop(&shared, &cancel, &sender);
}

fn event_loop(shared: &Shared, cancel: &CancelToken, sender: &Sender<WatchEvent>) {
    let mut state = shared.state.lock().unwrap();
    loop {
        if state.stopped || cancel.is_cancelled() || sender.is_closed() {
            return;
        }
        if let Some(error) = state.error.take() {
            drop(state);
            send_error(error, shared, cancel, sender);
            return;
        }
        let now = Instant::now();
        let wait = if let Some(pending) = state.pending {
            let until = pending.deadline().saturating_duration_since(now);
            if until.is_zero() {
                state.pending = None;
                // Full means a refresh is already queued. Never queue work per file.
                let _ = sender.try_send(WatchEvent::Changed);
                continue;
            }
            until.min(CANCEL_CHECK)
        } else {
            CANCEL_CHECK
        };
        state = shared.wake.wait_timeout(state, wait).unwrap().0;
    }
}

fn send_error(error: String, shared: &Shared, cancel: &CancelToken, sender: &Sender<WatchEvent>) {
    let mut event = WatchEvent::Unavailable(error);
    loop {
        if cancelled(shared, cancel, sender) {
            return;
        }
        match sender.try_send(event) {
            Ok(()) | Err(TrySendError::Closed(_)) => return,
            Err(TrySendError::Full(pending)) => event = pending,
        }
        // Preserve an error behind a queued Changed event without an unbounded queue or
        // blocking send that would prevent cancellation after the tab closes.
        let state = shared.state.lock().unwrap();
        if state.stopped {
            return;
        }
        drop(shared.wake.wait_timeout(state, CANCEL_CHECK).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounce_waits_for_quiet_but_does_not_starve_during_continuous_writes() {
        let start = Instant::now();
        let mut state = Mailbox::default();
        state.changed(start);
        assert_eq!(state.pending.unwrap().deadline(), start + DEBOUNCE);
        state.changed(start + Duration::from_millis(150));
        assert_eq!(
            state.pending.unwrap().deadline(),
            start + Duration::from_millis(350)
        );
        for milliseconds in (300..=900).step_by(150) {
            state.changed(start + Duration::from_millis(milliseconds));
        }
        assert_eq!(state.pending.unwrap().deadline(), start + MAX_DELAY);
    }

    #[test]
    fn access_events_do_not_refresh_and_cancel_discards_pending_changes() {
        let shared = Arc::new(Shared::default());
        shared.event(Ok(notify::Event::new(EventKind::Access(
            notify::event::AccessKind::Any,
        ))));
        assert!(shared.state.lock().unwrap().pending.is_none());
        shared.event(Ok(notify::Event::new(EventKind::Create(
            notify::event::CreateKind::File,
        ))));
        assert!(shared.state.lock().unwrap().pending.is_some());
        let handle = WatchHandle {
            shared: shared.clone(),
        };
        drop(handle);
        let (sender, receiver) = async_channel::bounded(1);
        event_loop(&shared, &CancelToken::new(), &sender);
        assert!(receiver.is_empty());
    }

    #[test]
    fn full_channel_keeps_one_refresh_and_cancellation_wakes_an_idle_worker() {
        let shared = Arc::new(Shared::default());
        let handle = WatchHandle {
            shared: shared.clone(),
        };
        let (sender, receiver) = async_channel::bounded(1);
        sender.try_send(WatchEvent::Changed).unwrap();
        shared
            .state
            .lock()
            .unwrap()
            .changed(Instant::now() - DEBOUNCE);
        let worker_shared = shared.clone();
        let worker =
            std::thread::spawn(move || event_loop(&worker_shared, &CancelToken::new(), &sender));
        let deadline = Instant::now() + Duration::from_secs(2);
        while shared.state.lock().unwrap().pending.is_some() {
            assert!(Instant::now() < deadline, "pending change did not coalesce");
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(handle);
        worker.join().unwrap();
        assert_eq!(receiver.len(), 1);
        assert_eq!(receiver.try_recv().unwrap(), WatchEvent::Changed);
        assert!(receiver.is_closed());
    }

    #[test]
    fn native_error_survives_a_full_channel_and_external_cancellation_stops_waiting() {
        let shared = Arc::new(Shared::default());
        let cancel = CancelToken::new();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = async_channel::bounded(1);
        sender.try_send(WatchEvent::Changed).unwrap();
        let worker_shared = shared.clone();
        let worker = std::thread::spawn(move || {
            send_error(
                "native watch stopped".into(),
                &worker_shared,
                &worker_cancel,
                &sender,
            );
        });
        assert_eq!(receiver.recv_blocking().unwrap(), WatchEvent::Changed);
        assert_eq!(
            receiver.recv_blocking().unwrap(),
            WatchEvent::Unavailable("native watch stopped".into())
        );
        worker.join().unwrap();

        let (sender, receiver) = async_channel::bounded(1);
        sender.try_send(WatchEvent::Changed).unwrap();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            send_error("error".into(), &shared, &worker_cancel, &sender)
        });
        cancel.cancel();
        worker.join().unwrap();
        assert_eq!(receiver.len(), 1);
        assert!(receiver.is_closed());
    }

    #[test]
    fn invalid_paths_and_network_paths_are_not_watched() {
        assert!(validate_path(Path::new("relative")).is_err());
        #[cfg(windows)]
        for path in [
            r"\\server\share\folder",
            r"\\wsl.localhost\Ubuntu\home",
            r"\\?\UNC\server\share",
        ] {
            assert!(validate_path(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn native_watch_reports_changes_and_handle_drop_closes_the_stream() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                // This directory was exclusively created by this test, below the system
                // temporary directory; never remove a caller-supplied or existing path.
                assert!(self.0.is_absolute() && self.0.starts_with(std::env::temp_dir()));
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("skylis-watch-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let fixture = Fixture(path);
        let (handle, receiver) = watch(fixture.0.clone(), CancelToken::new()).unwrap();
        let receive_before = |timeout: Duration| {
            let deadline = Instant::now() + timeout;
            loop {
                match receiver.try_recv() {
                    Ok(event) => break event,
                    Err(async_channel::TryRecvError::Closed) => {
                        panic!("watch stream closed unexpectedly")
                    }
                    Err(async_channel::TryRecvError::Empty) => assert!(
                        Instant::now() < deadline,
                        "native change notification timed out"
                    ),
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        assert_eq!(receive_before(Duration::from_secs(5)), WatchEvent::Changed);
        std::fs::write(fixture.0.join("created.txt"), "changed outside the app").unwrap();
        assert_eq!(receive_before(Duration::from_secs(5)), WatchEvent::Changed);
        drop(handle);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !receiver.is_closed() {
            assert!(
                Instant::now() < deadline,
                "watch cancellation did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
