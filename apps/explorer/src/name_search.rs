//! Two permanent worker lanes. No filesystem call, worker join or blocking send on GPUI.
use explorer_core::{
    CancelToken,
    name_search::{self as core, Event, Locality},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub owner: u64,
    pub epoch: u64,
    pub generation: u64,
}
pub enum Message {
    Classified(Locality),
    Search(Event),
    Failed(String),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Classify,
    Local,
    Explicit,
}
struct Request {
    key: Key,
    root: PathBuf,
    query: String,
    filters: core::Filters,
    hidden: bool,
    mode: Mode,
    cancel: CancelToken,
    deadline: Instant,
    reply: async_channel::Sender<Message>,
}
#[derive(Default)]
struct Queue {
    pending: VecDeque<Request>,
    active: HashSet<u64>,
    live: HashMap<u64, Key>,
    shutdown: bool,
}
struct Service {
    queue: Mutex<Queue>,
    wake: Condvar,
    #[cfg(test)]
    work: Option<Arc<dyn Fn(&mut Request) -> bool + Send + Sync>>,
}
pub struct Ticket {
    pub receiver: async_channel::Receiver<Message>,
    pub deadline: Instant,
    key: Key,
    cancel: CancelToken,
    service: Arc<Service>,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.receiver.close();
        drain_budget()
            .lock()
            .unwrap()
            .ready
            .retain(|k| *k != self.key);
        let mut q = self.service.queue.lock().unwrap();
        if q.live.get(&self.key.owner) == Some(&self.key) {
            q.live.remove(&self.key.owner);
        }
        q.pending.retain(|r| r.key != self.key);
        self.service.wake.notify_all();
    }
}
impl Service {
    fn start() -> Result<Arc<Self>, String> {
        let service = Arc::new(Self {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            #[cfg(test)]
            work: None,
        });
        for local in [true, false] {
            let s = service.clone();
            let spawned = std::thread::Builder::new()
                .name(
                    if local {
                        "search-local"
                    } else {
                        "search-explicit"
                    }
                    .into(),
                )
                .spawn(move || s.run(local));
            if let Err(e) = spawned {
                service.queue.lock().unwrap().shutdown = true;
                service.wake.notify_all();
                return Err(e.to_string());
            }
        }
        Ok(service)
    }
    fn run(self: Arc<Self>, local: bool) {
        loop {
            let mut request = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if q.shutdown {
                        return;
                    }
                    q.pending.retain(|r| {
                        !r.cancel.is_cancelled()
                            && !r.reply.is_closed()
                            && Instant::now() < r.deadline
                    });
                    if let Some(i) = q.pending.iter().position(|r| {
                        (r.mode == Mode::Local) == local && !q.active.contains(&r.key.owner)
                    }) {
                        let r = q.pending.remove(i).unwrap();
                        q.active.insert(r.key.owner);
                        break r;
                    }
                    q = self.wake.wait(q).unwrap();
                }
            };
            let mut transfer = false;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(test)]
                if let Some(work) = &self.work {
                    transfer = work(&mut request);
                    return;
                }
                if !request.cancel.is_cancelled()
                    && !request.reply.is_closed()
                    && Instant::now() < request.deadline
                {
                    if request.mode == Mode::Classify {
                        let locality = core::classify(&request.root);
                        if !request.cancel.is_cancelled() && Instant::now() < request.deadline {
                            let _ = request.reply.try_send(Message::Classified(locality));
                            if locality == Locality::Local {
                                request.mode = Mode::Local;
                                transfer = true;
                            }
                        }
                    } else {
                        let result = core::search(
                            &request.root,
                            &request.query,
                            &request.filters,
                            request.hidden,
                            &request.cancel,
                            request.deadline,
                            core::Limits::default(),
                            |event| {
                                if request.cancel.is_cancelled()
                                    || Instant::now() >= request.deadline
                                {
                                    return false;
                                }
                                if matches!(event, Event::Progress(_)) {
                                    let _ = request.reply.try_send(Message::Search(event));
                                    true
                                } else {
                                    deliver(&request, Message::Search(event))
                                }
                            },
                        );
                        if let Err(e) = result {
                            let _ = deliver(&request, Message::Failed(e.to_string()));
                        }
                    }
                }
            }));
            if outcome.is_err() {
                transfer = false;
                let _ = deliver(
                    &request,
                    Message::Failed("検索処理に失敗しました。再試行してください".into()),
                );
            }
            let mut q = self.queue.lock().unwrap();
            q.active.remove(&request.key.owner);
            if transfer && !request.cancel.is_cancelled() && !request.reply.is_closed() {
                q.pending.push_back(request);
            }
            self.wake.notify_all();
        }
    }
}
// Deadline-aware backpressure. Sleeping is confined to the permanent worker.
fn deliver(request: &Request, mut message: Message) -> bool {
    loop {
        if request.cancel.is_cancelled() || Instant::now() >= request.deadline {
            return false;
        }
        match request.reply.try_send(message) {
            Ok(()) => return true,
            Err(async_channel::TrySendError::Closed(_)) => return false,
            Err(async_channel::TrySendError::Full(m)) => {
                message = m;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
pub fn request(
    key: Key,
    root: PathBuf,
    query: String,
    filters: core::Filters,
    hidden: bool,
    mode: Mode,
) -> Result<Ticket, String> {
    static SERVICE: OnceLock<Result<Arc<Service>, String>> = OnceLock::new();
    let service = SERVICE
        .get_or_init(Service::start)
        .as_ref()
        .map_err(Clone::clone)?
        .clone();
    submit(service, key, root, query, filters, hidden, mode)
}
fn submit(
    service: Arc<Service>,
    key: Key,
    root: PathBuf,
    query: String,
    filters: core::Filters,
    hidden: bool,
    mode: Mode,
) -> Result<Ticket, String> {
    let mut q = service.queue.lock().unwrap();
    if !q.live.contains_key(&key.owner) && q.live.len() >= 8 {
        return Err("検索を同時に保持できるタブ数の上限です。ほかの検索を停止してください".into());
    }
    q.pending
        .retain(|r| r.key.owner != key.owner && !r.cancel.is_cancelled() && !r.reply.is_closed());
    if q.pending.len() >= 8 {
        return Err("検索待ちが混み合っています。再試行してください".into());
    }
    let (reply, receiver) = async_channel::bounded(2);
    let cancel = CancelToken::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    q.live.insert(key.owner, key);
    q.pending.push_back(Request {
        key,
        root,
        query,
        filters,
        hidden,
        mode,
        cancel: cancel.clone(),
        deadline,
        reply,
    });
    service.wake.notify_all();
    drop(q);
    Ok(Ticket {
        receiver,
        deadline,
        key,
        cancel,
        service,
    })
}

/// Cooperative 2ms admission budget per 16ms across windows, with ready-key fairness.
/// A chunk contains at most 64 Arc hits; a single chunk can overshoot the budget.
#[derive(Default)]
pub struct DrainBudget {
    start: Option<Instant>,
    used: Duration,
    ready: VecDeque<Key>,
}
impl DrainBudget {
    fn available(&mut self, key: Key) -> bool {
        if !self.ready.contains(&key) {
            self.ready.push_back(key);
        }
        if self
            .start
            .is_none_or(|s| s.elapsed() >= Duration::from_millis(16))
        {
            self.start = Some(Instant::now());
            self.used = Duration::ZERO;
        }
        if self.used >= Duration::from_millis(2) || self.ready.front() != Some(&key) {
            return false;
        }
        self.ready.pop_front();
        self.ready.push_back(key);
        true
    }
    fn charge(&mut self, key: Key, time: Duration, more: bool) {
        self.used += time;
        if !more {
            self.ready.retain(|k| *k != key);
        }
    }
}
fn drain_budget() -> &'static Mutex<DrainBudget> {
    static BUDGET: OnceLock<Mutex<DrainBudget>> = OnceLock::new();
    BUDGET.get_or_init(|| Mutex::new(DrainBudget::default()))
}
pub fn drain_available(key: Key) -> bool {
    drain_budget().lock().unwrap().available(key)
}
pub fn drain_charge(key: Key, time: Duration, more: bool) {
    drain_budget().lock().unwrap().charge(key, time, more);
}

#[cfg(test)]
pub(crate) fn test_ticket(key: Key) -> (Ticket, async_channel::Sender<Message>) {
    let service = Arc::new(Service {
        queue: Mutex::new(Queue::default()),
        wake: Condvar::new(),
        work: None,
    });
    let (reply, receiver) = async_channel::bounded(2);
    (
        Ticket {
            receiver,
            deadline: Instant::now() + Duration::from_secs(30),
            key,
            cancel: CancelToken::new(),
            service,
        },
        reply,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_service_preserves_batches_and_terminal_order() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        for i in 0..130 {
            std::fs::write(root.path().join(format!("sub/report_{i}.txt")), "").unwrap();
        }
        let ticket = request(
            Key {
                owner: u64::MAX,
                epoch: 0,
                generation: 0,
            },
            root.path().into(),
            "report".into(),
            core::Filters::default(),
            true,
            Mode::Local,
        )
        .unwrap();
        let mut count = 0;
        loop {
            match ticket.receiver.recv_blocking().unwrap() {
                Message::Search(Event::Batch(hits)) => {
                    assert!(hits.len() <= 64);
                    count += hits.len();
                }
                Message::Search(Event::Done(p, end)) => {
                    assert_eq!(count, 130);
                    assert_eq!(p.matched, 130);
                    assert_eq!(end, core::End::Complete);
                    break;
                }
                Message::Search(Event::Progress(_)) => {}
                _ => panic!("unexpected event"),
            }
        }
        #[cfg(windows)]
        assert_eq!(core::classify(root.path()), Locality::Local);
    }
    #[test]
    fn blocked_explicit_lane_does_not_block_local_and_same_owner_never_overlaps() {
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let service = Arc::new(Service {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            work: Some(Arc::new(move |r| {
                entered.send(r.key).unwrap();
                if r.key.owner == 1 && r.key.generation == 0 {
                    release_rx.lock().unwrap().recv().unwrap();
                }
                if r.key.owner == 3 {
                    panic!("simulated filesystem panic");
                }
                false
            })),
        });
        let mut threads = vec![];
        for local in [true, false] {
            let s = service.clone();
            threads.push(std::thread::spawn(move || s.run(local)));
        }
        let key = |owner, generation| Key {
            owner,
            epoch: 1,
            generation,
        };
        let submit_test = |k, mode| {
            submit(
                service.clone(),
                k,
                PathBuf::new(),
                "x".into(),
                core::Filters::default(),
                false,
                mode,
            )
            .unwrap()
        };
        let old = submit_test(key(1, 0), Mode::Explicit);
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key(1, 0)
        );
        drop(old);
        let next = submit_test(key(1, 1), Mode::Local);
        let local = submit_test(key(2, 0), Mode::Local);
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key(2, 0)
        );
        assert!(entered_rx.try_recv().is_err());
        release.send(()).unwrap();
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key(1, 1)
        );
        let panic_ticket = submit_test(key(3, 0), Mode::Local);
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key(3, 0)
        );
        // FIFO message wakes only after panic recovery, not merely after entering.
        assert!(matches!(
            panic_ticket.receiver.recv_blocking().unwrap(),
            Message::Failed(_)
        ));
        let after = submit_test(key(4, 0), Mode::Local);
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key(4, 0)
        );
        drop((next, local, panic_ticket, after));
        service.queue.lock().unwrap().shutdown = true;
        service.wake.notify_all();
        for t in threads {
            t.join().unwrap();
        }
    }
    #[test]
    fn full_channel_honours_deadline_without_receiver_destruction() {
        let (tx, rx) = async_channel::bounded(1);
        tx.try_send(Message::Classified(Locality::Local)).unwrap();
        let request = Request {
            key: Key {
                owner: 1,
                epoch: 0,
                generation: 0,
            },
            root: PathBuf::new(),
            query: "x".into(),
            filters: core::Filters::default(),
            hidden: false,
            mode: Mode::Local,
            cancel: CancelToken::new(),
            deadline: Instant::now(),
            reply: tx,
        };
        assert!(!deliver(&request, Message::Classified(Locality::Local)));
        assert_eq!(rx.len(), 1);
    }
    #[test]
    fn drain_quota_preserves_ready_order_across_intervals() {
        let mut budget = DrainBudget::default();
        let a = Key {
            owner: 1,
            epoch: 0,
            generation: 0,
        };
        let b = Key { owner: 2, ..a };
        assert!(budget.available(a));
        assert!(!budget.available(b));
        budget.charge(a, Duration::from_millis(2), true);
        assert!(!budget.available(a));
        budget.start = Some(Instant::now() - Duration::from_millis(17));
        assert!(budget.available(a));
        assert!(budget.available(b));
        assert!(!budget.available(b));
        budget.charge(a, Duration::ZERO, false);
        assert!(budget.available(b));
    }
    #[test]
    fn dropping_ticket_unblocks_full_delivery_and_releases_slot() {
        let service = Arc::new(Service {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            #[cfg(test)]
            work: None,
        });
        let key = Key {
            owner: 1,
            epoch: 0,
            generation: 0,
        };
        let cancel = CancelToken::new();
        let (tx, rx) = async_channel::bounded(2);
        service.queue.lock().unwrap().live.insert(1, key);
        tx.send_blocking(Message::Classified(Locality::Local))
            .unwrap();
        tx.send_blocking(Message::Classified(Locality::Local))
            .unwrap();
        let t = Ticket {
            receiver: rx,
            deadline: Instant::now(),
            key,
            cancel: cancel.clone(),
            service: service.clone(),
        };
        let thread = std::thread::spawn(move || {
            tx.send_blocking(Message::Classified(Locality::Local))
                .is_err()
        });
        drop(t);
        assert!(thread.join().unwrap());
        assert!(cancel.is_cancelled());
        assert!(service.queue.lock().unwrap().live.is_empty());
    }
}
