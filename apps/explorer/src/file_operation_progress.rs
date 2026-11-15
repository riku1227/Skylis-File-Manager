//! A latest-value mailbox. Shell callbacks never wait for GPUI or an unbounded queue.
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationProgress {
    pub label: &'static str,
    pub current_item: Option<String>,
    /// Relative Shell work estimates, not bytes or a count of files.
    pub work_done: u32,
    pub work_total: u32,
}
const INTERVAL: Duration = Duration::from_millis(100);
#[derive(Default)]
struct State {
    snapshot: Option<OperationProgress>,
    revision: u64,
    published: u64,
    last_publish: Option<Instant>,
    subscribers: Vec<async_channel::Sender<()>>,
}
impl State {
    fn update(&mut self, value: Option<OperationProgress>) -> bool {
        if self.snapshot == value {
            return false;
        }
        self.snapshot = value;
        self.revision = self.revision.wrapping_add(1);
        true
    }
    fn delay(&self, now: Instant) -> Option<Duration> {
        if self.revision == self.published {
            return None;
        }
        Some(self.last_publish.map_or(Duration::ZERO, |last| {
            INTERVAL.saturating_sub(now.saturating_duration_since(last))
        }))
    }
    fn publish(&mut self, now: Instant) {
        if self.delay(now) != Some(Duration::ZERO) {
            return;
        }
        self.subscribers.retain(|s| !s.is_closed());
        for sender in &self.subscribers {
            let _ = sender.try_send(());
        }
        self.published = self.revision;
        self.last_publish = Some(now);
    }
}
struct Hub {
    state: Arc<Mutex<State>>,
    wake: async_channel::Sender<()>,
}
impl Hub {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let worker = state.clone();
        let (wake, requests) = async_channel::bounded(1);
        // The thread sleeps on the channel when idle; only changed progress schedules work.
        // If resources prevent starting it, the native Windows progress dialog still works.
        let _ = std::thread::Builder::new()
            .name("file-progress".into())
            .spawn(move || {
                while requests.recv_blocking().is_ok() {
                    loop {
                        let delay = worker
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .delay(Instant::now());
                        match delay {
                            None => break,
                            Some(delay) if delay.is_zero() => {
                                worker
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .publish(Instant::now());
                                break;
                            }
                            Some(delay) => std::thread::sleep(delay),
                        }
                    }
                }
            });
        Self { state, wake }
    }
    fn change(&self, edit: impl FnOnce(&mut Option<OperationProgress>)) {
        let changed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut value = state.snapshot.clone();
            edit(&mut value);
            state.update(value)
        };
        if changed {
            let _ = self.wake.try_send(());
        }
    }
}
fn hub() -> &'static Hub {
    static HUB: OnceLock<Hub> = OnceLock::new();
    HUB.get_or_init(Hub::new)
}

pub fn subscribe_progress() -> async_channel::Receiver<()> {
    let (sender, receiver) = async_channel::bounded(1);
    let mut state = hub().state.lock().unwrap_or_else(|e| e.into_inner());
    state.subscribers.retain(|s| !s.is_closed());
    let _ = sender.try_send(());
    state.subscribers.push(sender);
    receiver
}
pub fn progress_snapshot() -> Option<OperationProgress> {
    hub()
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snapshot
        .clone()
}

pub(crate) struct Guard;
pub(crate) fn begin(label: &'static str) -> Guard {
    hub().change(|value| {
        *value = Some(OperationProgress {
            label,
            current_item: None,
            work_done: 0,
            work_total: 0,
        })
    });
    Guard
}
impl Drop for Guard {
    fn drop(&mut self) {
        hub().change(|value| *value = None);
    }
}
#[cfg(windows)]
fn item(label: &'static str, name: Option<String>) {
    hub().change(|value| {
        if let Some(value) = value {
            value.label = label;
            value.current_item = name;
        }
    });
}
#[cfg(windows)]
fn work(total: u32, done: u32) {
    hub().change(|value| {
        if let Some(value) = value {
            value.work_total = total;
            value.work_done = done.min(total);
        }
    });
}

#[cfg(windows)]
mod sink;
#[cfg(windows)]
pub(crate) use sink::Subscription;

#[cfg(test)]
mod tests {
    use super::*;
    fn value(n: u32) -> Option<OperationProgress> {
        Some(OperationProgress {
            label: "コピー",
            current_item: Some(format!("file-{n}")),
            work_done: n,
            work_total: 1000,
        })
    }
    #[test]
    fn callback_burst_delivers_latest_snapshot_with_one_bounded_signal() {
        let mut state = State::default();
        let (sender, receiver) = async_channel::bounded(1);
        state.subscribers.push(sender);
        let now = Instant::now();
        state.update(value(1));
        state.publish(now);
        assert!(receiver.try_recv().is_ok());
        for n in 2..=99 {
            state.update(value(n));
            state.publish(now + Duration::from_millis(n as u64));
        }
        assert!(receiver.try_recv().is_err());
        assert_eq!(state.snapshot, value(99));
        state.publish(now + INTERVAL);
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
        // A slow subscriber cannot accumulate a per-file backlog.
        for n in 1..=100 {
            state.update(value(n));
            state.publish(now + INTERVAL * (n + 1));
        }
        assert_eq!(receiver.len(), 1);
        assert_eq!(state.snapshot, value(100));
    }
    #[test]
    fn completion_is_delivered_after_throttle_and_closed_subscribers_are_pruned() {
        let mut state = State::default();
        let (sender, receiver) = async_channel::bounded(1);
        let (closed, drop_me) = async_channel::bounded(1);
        state.subscribers.extend([sender, closed]);
        drop(drop_me);
        let now = Instant::now();
        state.update(value(1));
        state.publish(now);
        assert_eq!(state.subscribers.len(), 1);
        receiver.try_recv().unwrap();
        state.update(None);
        state.publish(now + Duration::from_millis(5));
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            state.delay(now + Duration::from_millis(5)),
            Some(Duration::from_millis(95))
        );
        state.publish(now + INTERVAL);
        receiver.try_recv().unwrap();
        assert!(state.snapshot.is_none());
        assert_eq!(state.delay(now + INTERVAL), None);
        state.update(None);
        state.publish(now + INTERVAL * 2);
        assert!(receiver.try_recv().is_err());
    }
}
