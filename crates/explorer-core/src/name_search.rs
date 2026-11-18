//! Bounded descendant-name search. All filesystem work belongs on a worker.
mod policy;

use crate::{CancelToken, EntryKind};
pub use policy::{Locality, classify};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Any,
    File,
    Folder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    kind: Kind,
    extension: String,
}

impl Filters {
    pub fn from_input(kind: Kind, extension: &str) -> Self {
        let extension = fold_extension_width(extension.trim());
        Self {
            kind,
            extension: if kind == Kind::Folder {
                String::new()
            } else {
                extension
                    .strip_prefix("*.")
                    .or_else(|| extension.strip_prefix('.'))
                    .unwrap_or(&extension)
                    .to_lowercase()
            },
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn count(&self) -> u8 {
        u8::from(self.kind != Kind::Any) + u8::from(!self.extension.is_empty())
    }

    pub fn with_kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        if kind == Kind::Folder {
            self.extension.clear();
        }
        self
    }

    pub fn is_too_long(&self) -> bool {
        self.extension.len() > 4096
    }

    pub fn is_empty(&self) -> bool {
        self.kind == Kind::Any && self.extension.is_empty()
    }

    pub fn matches(&self, entry: &crate::Entry) -> bool {
        match self.kind {
            Kind::File if entry.kind != EntryKind::File => return false,
            Kind::Folder if entry.kind != EntryKind::Dir => return false,
            _ => {}
        }
        if self.extension.is_empty() {
            return true;
        }
        entry.extension().is_some_and(|extension| {
            if extension.is_ascii() {
                extension.eq_ignore_ascii_case(&self.extension)
            } else {
                // Lowercase the suffix independently so final sigma is judged in
                // the same context as the extension typed by the user.
                fold_extension_width(extension).to_lowercase() == self.extension
            }
        })
    }
}

fn fold_extension_width(extension: &str) -> std::borrow::Cow<'_, str> {
    const FULLWIDTH_ASCII: std::ops::RangeInclusive<char> = '\u{ff01}'..='\u{ff5e}';
    if !extension.chars().any(|c| FULLWIDTH_ASCII.contains(&c)) {
        return extension.into();
    }
    extension
        .chars()
        .map(|c| {
            if FULLWIDTH_ASCII.contains(&c) {
                char::from_u32(c as u32 - 0xfee0).unwrap()
            } else {
                c
            }
        })
        .collect::<String>()
        .into()
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub name: String,
    pub parent: String,
    pub kind: EntryKind,
}
impl Hit {
    fn bytes(&self) -> usize {
        policy::path_bytes(&self.path) + self.name.len() + self.parent.len()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub visited: usize,
    pub matched: usize,
    pub errors: usize,
    pub links_or_cloud: usize,
    pub depth_or_volume: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Entries,
    Hits,
    Bytes,
    Depth,
    Deadline,
    Path,
    Frontier,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Complete,
    Limited(Limit),
    Cancelled,
}
#[derive(Debug)]
pub enum Event {
    Batch(Vec<Arc<Hit>>),
    Progress(Progress),
    Done(Progress, End),
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub hits: usize,
    pub depth: usize,
    pub bytes: usize,
    pub path_bytes: usize,
    pub frontier_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 100_000,
            hits: 5_000,
            depth: 64,
            bytes: 8 * 1024 * 1024,
            path_bytes: 32 * 1024,
            frontier_bytes: 512 * 1024,
        }
    }
}

/// Direct children are visited to discover directories, but never emitted.
/// `deadline` includes queue time. Cancelling cannot interrupt a native call.
pub fn search(
    root: &Path,
    query: &str,
    filters: &Filters,
    hidden: bool,
    cancel: &CancelToken,
    deadline: Instant,
    limits: Limits,
    mut emit: impl FnMut(Event) -> bool,
) -> io::Result<()> {
    let stopped = || cancel.is_cancelled() || Instant::now() >= deadline;
    if stopped() {
        return Ok(());
    }
    let key = query.to_lowercase();
    if (key.is_empty() && filters.is_empty())
        || key.len() > 4096
        || filters.is_too_long()
        || policy::path_bytes(root) > limits.path_bytes
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "検索語または検索元が上限を超えています",
        ));
    }
    let root_id = policy::directory(root)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "リンク、未取得クラウド、または未対応の検索元です",
        )
    })?;
    if stopped() {
        return Ok(());
    }
    struct Frame {
        iter: fs::ReadDir,
        id: policy::Identity,
        bytes: usize,
    }
    let mut stack = vec![Frame {
        iter: fs::read_dir(root)?,
        id: root_id,
        bytes: policy::path_bytes(root),
    }];
    let mut frontier = policy::path_bytes(root);
    let mut p = Progress::default();
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    let mut total_bytes = 0;
    let mut last_batch = Instant::now();
    let mut last_progress = Instant::now();
    let mut limit = None;
    let matcher = crate::entry::NameMatcher::new(&key);
    while !stack.is_empty() {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            limit = Some(Limit::Deadline);
            break;
        }
        let depth = stack.len();
        let Some(next) = stack.last_mut().unwrap().iter.next() else {
            frontier = frontier.saturating_sub(stack.pop().unwrap().bytes);
            continue;
        };
        if stopped() {
            if cancel.is_cancelled() {
                return Ok(());
            }
            limit = Some(Limit::Deadline);
            break;
        }
        if p.visited >= limits.entries {
            limit = Some(Limit::Entries);
            break;
        }
        p.visited += 1;
        let de = match next {
            Ok(e) => e,
            Err(_) => {
                p.errors += 1;
                continue;
            }
        };
        let path = de.path();
        let path_bytes = policy::path_bytes(&path);
        if path_bytes > limits.path_bytes {
            p.depth_or_volume += 1;
            limit.get_or_insert(Limit::Path);
            continue;
        }
        let Some(entry) = crate::Entry::from_dir_entry(&de) else {
            p.errors += 1;
            continue;
        };
        if !hidden && entry.hidden {
            continue;
        }
        if depth >= 2 && filters.matches(&entry) && matcher.matches(&entry) {
            let hit = Arc::new(Hit {
                parent: path
                    .parent()
                    .and_then(|p| p.strip_prefix(root).ok())
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .into_owned(),
                path: path.clone(),
                name: entry.name,
                kind: entry.kind,
            });
            let bytes = hit.bytes();
            if p.matched >= limits.hits {
                limit = Some(Limit::Hits);
                break;
            }
            if total_bytes + bytes > limits.bytes {
                limit = Some(Limit::Bytes);
                break;
            }
            // Flush before adding so a batch cannot exceed its byte bound.
            if !batch.is_empty() && batch_bytes + bytes > 256 * 1024 {
                if !emit(Event::Batch(std::mem::take(&mut batch))) {
                    return Ok(());
                }
                batch_bytes = 0;
            }
            total_bytes += bytes;
            batch_bytes += bytes;
            p.matched += 1;
            batch.push(hit);
            if p.matched >= limits.hits {
                limit = Some(Limit::Hits);
                break;
            }
        }
        if entry.kind == EntryKind::Dir {
            if depth >= limits.depth || frontier + path_bytes > limits.frontier_bytes {
                p.depth_or_volume += 1;
                limit.get_or_insert(if depth >= limits.depth {
                    Limit::Depth
                } else {
                    Limit::Frontier
                });
            } else {
                if stopped() {
                    continue;
                }
                match policy::directory(&path) {
                    Ok(Some(id))
                        if id.volume == root_id.volume && !stack.iter().any(|f| f.id == id) =>
                    {
                        if !stopped() {
                            match fs::read_dir(&path) {
                                Ok(iter) => {
                                    frontier += path_bytes;
                                    stack.push(Frame {
                                        iter,
                                        id,
                                        bytes: path_bytes,
                                    });
                                }
                                Err(_) => p.errors += 1,
                            }
                        }
                    }
                    Ok(Some(_)) => p.depth_or_volume += 1,
                    Ok(None) => p.links_or_cloud += 1,
                    Err(_) => p.errors += 1,
                }
            }
        } else if entry.kind == EntryKind::Symlink {
            p.links_or_cloud += 1;
        }
        if batch.len() >= 64
            || (!batch.is_empty() && last_batch.elapsed() >= Duration::from_millis(50))
        {
            if !emit(Event::Batch(std::mem::take(&mut batch))) {
                return Ok(());
            }
            batch_bytes = 0;
            last_batch = Instant::now();
        }
        if last_progress.elapsed() >= Duration::from_millis(100) {
            if !emit(Event::Progress(p)) {
                return Ok(());
            }
            last_progress = Instant::now();
        }
    }
    if cancel.is_cancelled() {
        return Ok(());
    }
    if !batch.is_empty() && !emit(Event::Batch(batch)) {
        return Ok(());
    }
    emit(Event::Done(
        p,
        limit.map(End::Limited).unwrap_or(End::Complete),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("sub/deeper")).unwrap();
        fs::write(d.path().join("REPORT.txt"), "").unwrap();
        fs::write(d.path().join("sub/日本語_REPORT.txt"), "").unwrap();
        fs::write(d.path().join("sub/deeper/report.txt"), "").unwrap();
        d
    }
    fn run(root: &Path, limits: Limits, hidden: bool) -> (Vec<Arc<Hit>>, Progress, End) {
        let mut hits = vec![];
        let mut end = None;
        search(
            root,
            "report",
            &Filters::default(),
            hidden,
            &CancelToken::new(),
            Instant::now() + Duration::from_secs(30),
            limits,
            |e| {
                match e {
                    Event::Batch(b) => hits.extend(b),
                    Event::Done(p, s) => end = Some((p, s)),
                    _ => {}
                }
                true
            },
        )
        .unwrap();
        let (p, s) = end.unwrap();
        (hits, p, s)
    }
    #[test]
    fn extension_matches_the_final_suffix_and_kind_without_resolving_links() {
        let matches = |name: &str, kind, filter: &Filters| {
            filter.matches(&crate::Entry::virtual_entry(
                name.into(),
                PathBuf::new(),
                kind,
                None,
                None,
            ))
        };
        let pdf = Filters::from_input(Kind::Any, " .pDf ");
        for equivalent in ["pdf", ".PDF", "*.pdf", "．ｐｄｆ", "＊．ＰＤＦ"] {
            assert_eq!(Filters::from_input(Kind::Any, equivalent), pdf);
        }
        assert!(matches("book.PDF", EntryKind::File, &pdf));
        assert!(matches("book.ＰＤＦ", EntryKind::File, &pdf));
        assert!(!matches("book.PDF.bak", EntryKind::File, &pdf));
        assert!(!matches(".pdf", EntryKind::File, &pdf));
        assert!(!matches("book.pdf", EntryKind::Dir, &pdf));
        assert!(!matches("book.pdf", EntryKind::Symlink, &pdf));
        assert!(matches(
            "book.Σ",
            EntryKind::File,
            &Filters::from_input(Kind::File, ".σ")
        ));
        assert!(matches(
            "book.pdf",
            EntryKind::Dir,
            &pdf.with_kind(Kind::Folder)
        ));
    }

    #[test]
    fn filters_apply_before_the_hit_cap_and_can_search_without_a_name() {
        let d = fixture();
        fs::create_dir(d.path().join("sub/report.pdf")).unwrap();
        fs::write(d.path().join("sub/deeper/REPORT.PDF"), "").unwrap();
        fs::write(d.path().join("sub/deeper/other.PDF"), "").unwrap();
        let filters = Filters::from_input(Kind::Any, " .Pdf ");
        for (name, expected) in [("report", 1), ("", 2)] {
            let mut hits = Vec::new();
            let mut end = None;
            search(
                d.path(),
                name,
                &filters,
                false,
                &CancelToken::new(),
                Instant::now() + Duration::from_secs(30),
                Limits {
                    hits: 3,
                    ..Limits::default()
                },
                |event| {
                    match event {
                        Event::Batch(batch) => hits.extend(batch),
                        Event::Done(_, result) => end = Some(result),
                        _ => {}
                    }
                    true
                },
            )
            .unwrap();
            assert_eq!(hits.len(), expected);
            assert_eq!(end, Some(End::Complete));
            assert!(
                hits.iter()
                    .all(|hit| hit.kind == EntryKind::File && hit.name.ends_with(".PDF"))
            );
        }
    }

    #[test]
    fn descendants_only_case_insensitive_and_relative_parent() {
        let d = fixture();
        let (h, p, s) = run(d.path(), Limits::default(), false);
        assert_eq!(h.len(), 2);
        assert_eq!(p.matched, 2);
        assert_eq!(s, End::Complete);
        assert!(h.iter().any(|h| h.parent == "sub"));
        assert!(h.iter().all(|h| h.path.parent() != Some(d.path())));
    }
    #[test]
    fn caps_report_incomplete_and_terminal_follows_batch() {
        let d = fixture();
        let (h, _, s) = run(
            d.path(),
            Limits {
                hits: 1,
                ..Limits::default()
            },
            false,
        );
        assert_eq!(h.len(), 1);
        assert_eq!(s, End::Limited(Limit::Hits));
        assert_eq!(
            run(
                d.path(),
                Limits {
                    hits: 2,
                    ..Limits::default()
                },
                false
            )
            .2,
            End::Limited(Limit::Hits)
        );
        let (_, p, s) = run(
            d.path(),
            Limits {
                entries: 1,
                ..Limits::default()
            },
            false,
        );
        assert_eq!(p.visited, 1);
        assert_eq!(s, End::Limited(Limit::Entries));
    }
    #[test]
    fn cancel_and_expired_deadline_do_no_work() {
        let d = fixture();
        let c = CancelToken::new();
        c.cancel();
        search(
            d.path(),
            "x",
            &Filters::default(),
            false,
            &c,
            Instant::now() + Duration::from_secs(1),
            Limits::default(),
            |_| panic!("cancelled"),
        )
        .unwrap();
        search(
            d.path(),
            "x",
            &Filters::default(),
            false,
            &CancelToken::new(),
            Instant::now(),
            Limits::default(),
            |_| panic!("expired"),
        )
        .unwrap();
    }
    #[test]
    fn hidden_subtrees_and_path_byte_caps() {
        let d = fixture();
        fs::create_dir(d.path().join(".hidden")).unwrap();
        fs::write(d.path().join(".hidden/report.txt"), "").unwrap();
        // Windows hidden policy uses attributes, not a Unix dot prefix.
        #[cfg(unix)]
        assert_eq!(run(d.path(), Limits::default(), false).0.len(), 2);
        assert_eq!(run(d.path(), Limits::default(), true).0.len(), 3);
        assert!(
            run(
                d.path(),
                Limits {
                    path_bytes: policy::path_bytes(d.path()),
                    ..Limits::default()
                },
                true
            )
            .0
            .is_empty()
        );
    }
    #[test]
    fn byte_frontier_and_depth_limits_are_explicit() {
        let d = fixture();
        assert_eq!(
            run(
                d.path(),
                Limits {
                    bytes: 1,
                    ..Limits::default()
                },
                true
            )
            .2,
            End::Limited(Limit::Bytes)
        );
        assert_eq!(
            run(
                d.path(),
                Limits {
                    depth: 1,
                    ..Limits::default()
                },
                true
            )
            .2,
            End::Limited(Limit::Depth)
        );
        assert_eq!(
            run(
                d.path(),
                Limits {
                    frontier_bytes: policy::path_bytes(d.path()),
                    ..Limits::default()
                },
                true
            )
            .2,
            End::Limited(Limit::Frontier)
        );
    }
    #[cfg(windows)]
    #[test]
    fn windows_long_root_and_descendant_paths_are_searchable() {
        let d = tempfile::tempdir().unwrap();
        let root = d
            .path()
            .join("a".repeat(110))
            .join("b".repeat(110))
            .join("c".repeat(110));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/report.txt"), "").unwrap();
        assert_eq!(run(&root, Limits::default(), false).0.len(), 1);
    }
    #[test]
    fn downstream_cancellation_stops_delivery_before_terminal() {
        let d = fixture();
        let c = CancelToken::new();
        let mut batches = 0;
        search(
            d.path(),
            "report",
            &Filters::default(),
            true,
            &c,
            Instant::now() + Duration::from_secs(30),
            Limits::default(),
            |e| {
                if matches!(e, Event::Batch(_)) {
                    batches += 1;
                    c.cancel();
                    false
                } else {
                    panic!("terminal after cancellation")
                }
            },
        )
        .unwrap();
        assert_eq!(batches, 1);
    }
    #[cfg(unix)]
    #[test]
    fn ancestor_symlink_and_link_root_are_not_followed() {
        let d = fixture();
        std::os::unix::fs::symlink(d.path(), d.path().join("sub/loop")).unwrap();
        let (_, p, s) = run(d.path(), Limits::default(), true);
        assert_eq!(s, End::Complete);
        assert_eq!(p.links_or_cloud, 1);
        assert!(
            search(
                &d.path().join("sub/loop"),
                "report",
                &Filters::default(),
                true,
                &CancelToken::new(),
                Instant::now() + Duration::from_secs(30),
                Limits::default(),
                |_| true
            )
            .is_err()
        );
    }
}
