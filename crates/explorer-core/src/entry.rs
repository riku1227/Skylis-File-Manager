use std::cmp::Ordering;
use std::fs::DirEntry;
use std::path::PathBuf;
use std::time::SystemTime;

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
    fn natural_order() {
        let mut v = vec!["file10", "file2", "file1", "File02", "a", "日本"];
        v.sort_by(|a, b| natural_cmp(&a.to_lowercase(), &b.to_lowercase()));
        assert_eq!(v, vec!["a", "file1", "file2", "File02", "file10", "日本"]);
    }
}
