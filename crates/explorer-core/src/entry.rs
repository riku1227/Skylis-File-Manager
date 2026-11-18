use std::cmp::Ordering;
use std::fs::DirEntry;
use std::path::PathBuf;
use std::time::SystemTime;

/// Preprocess one normalized substring once for an entire folder, not for every name.
pub struct NameMatcher<'a>(memchr::memmem::Finder<'a>);
impl<'a> NameMatcher<'a> {
    pub fn new(normalized_query: &'a str) -> Self {
        Self(memchr::memmem::Finder::new(normalized_query.as_bytes()))
    }
    pub fn matches(&self, entry: &Entry) -> bool {
        self.0.find(entry.name_key.as_bytes()).is_some()
    }
    /// Original UTF-8 ranges for presentation; kept separate from the full filter.
    pub fn ranges(&self, entry: &Entry) -> Vec<std::ops::Range<usize>> {
        self.ranges_in(&entry.name, &entry.name_key)
    }
    pub fn ranges_for_name(&self, name: &str) -> Vec<std::ops::Range<usize>> {
        if self.0.needle().is_empty() {
            return Vec::new();
        }
        self.ranges_in(name, &name.to_lowercase())
    }
    fn ranges_in(&self, name: &str, key: &str) -> Vec<std::ops::Range<usize>> {
        if self.0.needle().is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        let mut hits = self.0.find_iter(key.as_bytes()).peekable();
        if hits.peek().is_none() {
            return result;
        }
        if name.is_ascii() && name.len() == key.len() {
            for start in hits {
                if !push_match(&mut result, start..start + self.0.needle().len()) {
                    break;
                }
            }
            return result;
        }
        // str::to_lowercase handles final sigma contextually. Its contextual
        // variant has the same UTF-8 width; every expansion still maps to one
        // original character. Fail closed if a future Unicode mapping differs.
        let width = |c: char| c.to_lowercase().map(char::len_utf8).sum::<usize>();
        if name.chars().map(width).sum::<usize>() != key.len() {
            return result;
        }
        let mut offset = 0;
        let mut chars = name.char_indices().map(|(start, c)| {
            let from = offset;
            offset += width(c);
            (from..offset, start..start + c.len_utf8())
        });
        let mut current = chars.next();
        for start in hits {
            let end = start + self.0.needle().len();
            while current.as_ref().is_some_and(|(k, _)| k.end <= start) {
                current = chars.next();
            }
            let Some((_, original)) = &current else {
                break;
            };
            let original_start = original.start;
            while current.as_ref().is_some_and(|(k, _)| k.end < end) {
                current = chars.next();
            }
            let Some((_, original)) = &current else {
                break;
            };
            if !push_match(&mut result, original_start..original.end) {
                break;
            }
        }
        result
    }
}

fn push_match(ranges: &mut Vec<std::ops::Range<usize>>, next: std::ops::Range<usize>) -> bool {
    if let Some(last) = ranges.last_mut()
        && last.end >= next.start
    {
        last.end = last.end.max(next.end);
        return true;
    }
    if ranges.len() >= 128 {
        return false;
    }
    ranges.push(next);
    true
}

#[cfg(test)]
mod match_range_tests {
    use super::*;
    fn ranges(name: &str, query: &str) -> Vec<std::ops::Range<usize>> {
        let key = query.to_lowercase();
        NameMatcher::new(&key).ranges_for_name(name)
    }
    #[test]
    fn highlights_repeated_ascii_and_japanese_without_overlaps() {
        assert_eq!(ranges("TXT.txt", "txt"), vec![0..3, 4..7]);
        assert_eq!(ranges("aaa", "aa"), vec![0..2]);
        assert_eq!(ranges("MEGA-回復キー.txt", "回復"), vec![5..11]);
        assert!(ranges("report.txt", "").is_empty());
        assert!(ranges("x", "long").is_empty());
    }
    #[test]
    fn lowercasing_changes_width_and_uses_context_without_invalid_ranges() {
        assert_eq!(ranges("İstanbul", "i"), vec![0..2]);
        assert_eq!(ranges("İ", "\u{307}"), vec![0..2]);
        assert_eq!(ranges("ẞ.txt", "ß"), vec![0..3]);
        assert_eq!(ranges("ΟΔΟΣ", "ς"), vec![6..8]);
        assert!(ranges("ΟΔΟΣ", "σ").is_empty());
        assert_eq!(ranges("İİ", "i\u{307}"), vec![0..4]);
    }
    #[test]
    fn cached_matching_and_highlights_agree_for_unicode_combinations() {
        for name in [
            "report.TXT",
            "İ_ẞ_ΟΔΟΣ_日本語",
            "ΣΣςΣσ",
            "e\u{301}👩\u{200d}💻.txt",
        ] {
            let entry =
                Entry::virtual_entry(name.into(), PathBuf::new(), EntryKind::File, None, None);
            for query in [
                "txt",
                "i",
                "ß",
                "ς",
                "σ",
                "日本語",
                "\u{301}",
                "💻",
                "absent",
            ] {
                let key = query.to_lowercase();
                let matcher = NameMatcher::new(&key);
                let r = matcher.ranges(&entry);
                assert_eq!(!r.is_empty(), matcher.matches(&entry), "{name:?}/{query:?}");
                assert_eq!(r, matcher.ranges_for_name(name));
                assert!(r.iter().all(|r| r.start < r.end
                    && name.is_char_boundary(r.start)
                    && name.is_char_boundary(r.end)));
                assert!(r.windows(2).all(|r| r[0].end < r[1].start));
            }
        }
    }
    #[test]
    fn range_count_is_bounded_and_touching_matches_merge() {
        let r = ranges(&"a_".repeat(200), "a");
        assert_eq!(r.len(), 128);
        assert_eq!(ranges(&"a".repeat(300), "a"), vec![0..300]);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
}

/// 一覧の1行。UIの型は持たない。
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// 比較用に事前計算した小文字名（ソートのたびにlowercaseしない）
    name_key: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
}

impl Entry {
    /// Caller normalizes the query once, rather than allocating a lowercase name per row.
    pub fn matches_name_key(&self, normalized_query: &str) -> bool {
        self.name_key.contains(normalized_query)
    }
    /// Metadata for a read-only virtual location. `path` is its member identity,
    /// not a path that may be passed to filesystem or Shell operations.
    pub fn virtual_entry(
        name: String,
        path: PathBuf,
        kind: EntryKind,
        size: Option<u64>,
        modified: Option<SystemTime>,
    ) -> Self {
        Self::from_parts(name, path, kind, size, modified)
    }

    /// A recursive name-search result already has a filesystem path but may not have metadata.
    pub fn from_search_hit(hit: &crate::name_search::Hit) -> Self {
        Self::from_parts(hit.name.clone(), hit.path.clone(), hit.kind, None, None)
    }

    fn from_parts(
        name: String,
        path: PathBuf,
        kind: EntryKind,
        size: Option<u64>,
        modified: Option<SystemTime>,
    ) -> Self {
        Self {
            name_key: name.to_lowercase(),
            hidden: name.starts_with('.'),
            name,
            path,
            kind,
            size,
            modified,
        }
    }
    /// Windowsでは `DirEntry::metadata()` は FindNextFileW の結果から作られるため追加のsyscallが発生しない。
    pub(crate) fn from_dir_entry(de: &DirEntry) -> Option<Self> {
        let name = de.file_name().to_string_lossy().into_owned();
        let meta = de.metadata().ok();
        let ft = de.file_type().ok();
        let kind = match ft {
            Some(ft) if ft.is_symlink() => {
                // ジャンクション/シンボリックリンク: 先がディレクトリかはここでは追わない（遅いNAS対策）
                EntryKind::Symlink
            }
            Some(ft) if ft.is_dir() => EntryKind::Dir,
            _ => EntryKind::File,
        };
        let hidden = is_hidden(&name, meta.as_ref());
        Some(Self {
            name_key: name.to_lowercase(),
            name,
            path: de.path(),
            kind,
            size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            hidden,
        })
    }

    pub fn is_dir_like(&self) -> bool {
        matches!(self.kind, EntryKind::Dir | EntryKind::Symlink)
    }

    pub fn extension(&self) -> Option<&str> {
        if self.kind != EntryKind::File {
            return None;
        }
        let dot = self.name.rfind('.')?;
        if dot == 0 {
            return None;
        }
        Some(&self.name[dot + 1..])
    }

    /// 種類列の表示文字列（PoCでは拡張子ベース。後でOSの種類名に置き換える）
    pub fn kind_label(&self) -> String {
        match self.kind {
            EntryKind::Dir => "フォルダー".into(),
            EntryKind::Symlink => "リンク".into(),
            EntryKind::File => match self.extension() {
                Some(ext) => format!("{} ファイル", ext.to_uppercase()),
                None => "ファイル".into(),
            },
        }
    }
}

#[cfg(windows)]
fn is_hidden(_name: &str, meta: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.is_some_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0)
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: Option<&std::fs::Metadata>) -> bool {
    name.starts_with('.')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortOrder {
    pub key: SortKey,
    pub ascending: bool,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            ascending: true,
        }
    }
}

impl SortOrder {
    /// 同じ列なら昇降を反転、別の列なら昇順で切り替え
    pub fn toggled(self, key: SortKey) -> Self {
        if self.key == key {
            Self {
                key,
                ascending: !self.ascending,
            }
        } else {
            Self {
                key,
                ascending: true,
            }
        }
    }

    /// フォルダーは常に先頭。その中で指定キー、同値は名前で安定化。
    pub fn compare(&self, a: &Entry, b: &Entry) -> Ordering {
        let dirs_first = b.is_dir_like().cmp(&a.is_dir_like());
        if dirs_first != Ordering::Equal {
            return dirs_first;
        }
        let ord = match self.key {
            SortKey::Name => Ordering::Equal,
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Kind => a
                .extension()
                .map(str::to_lowercase)
                .cmp(&b.extension().map(str::to_lowercase)),
        };
        let ord = ord.then_with(|| natural_cmp(&a.name_key, &b.name_key));
        if self.ascending { ord } else { ord.reverse() }
    }
}

/// "file2" < "file10" になる自然順比較（ASCII数字のみ対象）
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (da, db) = (trim_zeros(&a[..na]), trim_zeros(&b[..nb]));
                let ord = da
                    .len()
                    .cmp(&db.len())
                    .then_with(|| da.cmp(db))
                    .then_with(|| na.cmp(&nb));
                if ord != Ordering::Equal {
                    return ord;
                }
                a = &a[na..];
                b = &b[nb..];
            }
            (Some(_), Some(_)) => {
                // UTF-8のまま比較（マルチバイトでもバイト順＝コードポイント順）
                let la = utf8_len(a[0]);
                let lb = utf8_len(b[0]);
                let ord = a[..la.min(a.len())].cmp(&b[..lb.min(b.len())]);
                if ord != Ordering::Equal {
                    return ord;
                }
                a = &a[la.min(a.len())..];
                b = &b[lb.min(b.len())..];
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let n = d.iter().take_while(|c| **c == b'0').count();
    &d[n..]
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_name_matching_is_substring_and_unicode_case_aware() {
        let entry = Entry::virtual_entry(
            "日本語_REPORT.TXT".into(),
            PathBuf::from("file"),
            EntryKind::File,
            None,
            None,
        );
        assert!(entry.matches_name_key("report"));
        assert!(entry.matches_name_key("日本語"));
        assert!(!entry.matches_name_key("other"));
        assert!(NameMatcher::new("report").matches(&entry));
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "file2", "file1", "File02", "a", "日本"];
        v.sort_by(|a, b| natural_cmp(&a.to_lowercase(), &b.to_lowercase()));
        assert_eq!(v, vec!["a", "file1", "file2", "File02", "file10", "日本"]);
    }
}
