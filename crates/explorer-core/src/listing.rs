use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::entry::Entry;

/// 列挙を途中で止めるためのフラグ。フォルダー移動時に古い列挙を打ち切る。
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub enum ListingEvent {
    /// 部分結果。UI側でマージする。
    Batch(Vec<Entry>),
    /// 列挙完了
    Done(ListingStats),
    /// フォルダー自体を開けなかった（権限なし・存在しない等）
    Failed(io::ErrorKind, String),
}

#[derive(Debug, Clone, Copy)]
pub struct ListingStats {
    pub total: usize,
    pub first_batch: Option<Duration>,
    pub elapsed: Duration,
    pub cancelled: bool,
}

/// 最初のバッチは早く出し（初回表示を優先）、以降は間隔でまとめてUI更新回数を抑える。
const FIRST_BATCH_ITEMS: usize = 200;
const BATCH_INTERVAL: Duration = Duration::from_millis(40);

/// ブロッキングでディレクトリを列挙し、バッチ単位で `emit` に渡す。
/// `emit` が false を返したら受け手がいなくなったとみなして中断する。
/// 専用スレッドから呼ぶ想定（遅いNASでUIスレッド・GPUIの実行プールを塞がない）。
pub fn list_dir(path: &Path, cancel: &CancelToken, mut emit: impl FnMut(ListingEvent) -> bool) {
    let start = Instant::now();
    let read = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => {
            emit(ListingEvent::Failed(e.kind(), e.to_string()));
            return;
        }
    };

    let mut buf: Vec<Entry> = Vec::with_capacity(FIRST_BATCH_ITEMS);
    let mut total = 0usize;
    let mut first_batch = None;
    let mut last_emit = start;
    let mut cancelled = false;

    for de in read {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let Ok(de) = de else { continue };
        if let Some(entry) = Entry::from_dir_entry(&de) {
            buf.push(entry);
        }

        let due = if first_batch.is_none() {
            buf.len() >= FIRST_BATCH_ITEMS
        } else {
            last_emit.elapsed() >= BATCH_INTERVAL
        };
        if due && !buf.is_empty() {
            total += buf.len();
            first_batch.get_or_insert_with(|| start.elapsed());
            last_emit = Instant::now();
            if !emit(ListingEvent::Batch(std::mem::take(&mut buf))) {
                return;
            }
        }
    }

    if !buf.is_empty() && !cancelled {
        total += buf.len();
        first_batch.get_or_insert_with(|| start.elapsed());
        if !emit(ListingEvent::Batch(buf)) {
            return;
        }
    }
    emit(ListingEvent::Done(ListingStats {
        total,
        first_batch,
        elapsed: start.elapsed(),
        cancelled,
    }));
}
