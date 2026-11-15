//! Bounded, blocking archive operations. Call these APIs from a worker, never the UI thread.

mod paths;
mod seven_zip;
mod zip;

use crate::CancelToken;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, Metadata},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_NAMES_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_HEADER_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_MEMBER_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const COPY_BUFFER: usize = 256 * 1024;
const PASSWORD_UNSUPPORTED: &str =
    "パスワード付きの圧縮ファイルにはまだ対応していません。既定のアプリで開いてください";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Validated relative path, separated with '/'; directories have no trailing slash.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct ArchiveIndex {
    pub source: PathBuf,
    pub source_len: u64,
    pub modified: Option<SystemTime>,
    pub entries: Vec<ArchiveEntry>,
}

#[derive(Clone, Debug)]
pub struct ArchiveProgress {
    pub processed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub current_name: String,
    pub entries_done: usize,
}

pub fn detect_format(path: &Path) -> Option<ArchiveFormat> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("zip") {
        Some(ArchiveFormat::Zip)
    } else if extension.eq_ignore_ascii_case("7z") {
        Some(ArchiveFormat::SevenZip)
    } else {
        None
    }
}

pub fn inspect(path: &Path, cancel: &CancelToken) -> Result<ArchiveIndex, String> {
    check_cancel(cancel)?;
    let format = detect_format(path).ok_or("対応している圧縮形式はZIPと7zです")?;
    let file = paths::open_regular(path)?;
    let before = file.metadata().map_err(error)?;
    let reader = CancelReader {
        inner: file,
        cancel: cancel.clone(),
    };
    let entries = match format {
        ArchiveFormat::Zip => zip::inspect(reader, cancel)?,
        ArchiveFormat::SevenZip => seven_zip::inspect(reader, cancel)?,
    };
    validate_entries(&entries)?;
    check_cancel(cancel)?;
    let after = fs::symlink_metadata(path).map_err(error)?;
    paths::check_not_redirect(path, &after)?;
    if paths::is_link(&after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("読み込み中に圧縮ファイルが変更されました。更新してください".into());
    }
    Ok(ArchiveIndex {
        source: path.to_path_buf(),
        source_len: before.len(),
        modified: before.modified().ok(),
        entries,
    })
}

/// Worker-only cache fingerprint check, using the same redirect/cloud policy as inspection.
pub fn source_is_current(index: &ArchiveIndex) -> bool {
    paths::check_ancestors(&index.source).is_ok()
        && fs::symlink_metadata(&index.source)
            .is_ok_and(|metadata| metadata.is_file() && same_metadata(&metadata, index))
}

/// Direct children, including implicit parent directories omitted by some archive writers.
pub fn children(index: &ArchiveIndex, prefix: &str) -> Vec<ArchiveEntry> {
    let prefix = prefix.trim_end_matches('/');
    let start = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    let mut entries = BTreeMap::<String, ArchiveEntry>::new();
    for entry in &index.entries {
        let Some(rest) = entry.path.strip_prefix(&start) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let name = rest.split('/').next().unwrap_or(rest);
        let path = format!("{start}{name}");
        if !rest.contains('/') {
            entries.insert(path, entry.clone());
        } else {
            entries.entry(path.clone()).or_insert(ArchiveEntry {
                path,
                is_dir: true,
                size: 0,
                modified: None,
            });
        }
    }
    entries.into_values().collect()
}

/// Extract selected paths (empty = all) to a NEW folder. Existing files are never replaced.
pub fn extract(
    index: &ArchiveIndex,
    selected_prefixes: &[String],
    destination_parent: &Path,
    cancel: &CancelToken,
    mut progress: impl FnMut(ArchiveProgress),
) -> Result<PathBuf, String> {
    check_cancel(cancel)?;
    paths::check_directory(destination_parent)?;
    let current = inspect(&index.source, cancel)?;
    if current.source_len != index.source_len
        || current.modified != index.modified
        || current.entries != index.entries
    {
        return Err("圧縮ファイルが変更されました。更新してから展開してください".into());
    }
    let selected: Vec<String> = selected_prefixes
        .iter()
        .map(|s| paths::normalize(s))
        .collect::<Result<_, _>>()?;
    for prefix in &selected {
        if !current
            .entries
            .iter()
            .any(|e| selected_path(&e.path, std::slice::from_ref(prefix)))
        {
            return Err(format!("圧縮ファイル内の項目が見つかりません: {prefix}"));
        }
    }
    let total = current
        .entries
        .iter()
        .filter(|e| selected_path(&e.path, &selected))
        .map(|e| e.size)
        .sum();
    paths::check_free_space(destination_parent, total)?;
    let zone = paths::read_zone(&current.source)?;
    let stage = paths::Staging::directory(destination_parent)?;
    let mut work = Work::new(cancel, &mut progress, Some(total));
    let file = paths::open_regular(&current.source)?;
    if !same_metadata(&file.metadata().map_err(error)?, &current) {
        return Err("圧縮ファイルが変更されました。更新してください".into());
    }
    let reader = CancelReader {
        inner: file,
        cancel: cancel.clone(),
    };
    match detect_format(&current.source).ok_or("対応していない圧縮形式です")? {
        ArchiveFormat::Zip => zip::extract(reader, &current, &selected, &stage.path, &mut work)?,
        ArchiveFormat::SevenZip => {
            seven_zip::extract(reader, &current, &selected, &stage.path, &mut work)?
        }
    }
    check_cancel(cancel)?;
    if !same_metadata(&fs::metadata(&current.source).map_err(error)?, &current) {
        return Err("展開中に圧縮ファイルが変更されました。更新してください".into());
    }
    if paths::read_zone(&current.source)? != zone {
        return Err("展開中にダウンロード元の安全情報が変更されました".into());
    }
    paths::apply_zone(
        &stage.path,
        &current.entries,
        &selected,
        zone.as_deref(),
        cancel,
    )?;
    check_cancel(cancel)?;
    let name = current
        .source
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("展開したファイル");
    let name = paths::normalize(name)
        .ok()
        .filter(|n| !n.contains('/'))
        .unwrap_or_else(|| "展開したファイル".into());
    let destination = stage.publish_unique_directory(destination_parent, &name)?;
    work.emit(true);
    Ok(destination)
}

/// Compress selected physical paths. Destination must not exist; publishes only after finish.
pub fn create(
    paths: &[PathBuf],
    destination: &Path,
    format: ArchiveFormat,
    cancel: &CancelToken,
    mut progress: impl FnMut(ArchiveProgress),
) -> Result<PathBuf, String> {
    check_cancel(cancel)?;
    let parent = destination
        .parent()
        .ok_or("圧縮ファイルの保存先を指定してください")?;
    paths::check_directory(parent)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("同名の項目が既に存在します".into());
    }
    let sources = source_plan(paths, destination, cancel)?;
    let total = sources.iter().map(|s| s.entry.size).sum();
    let (stage, output) = paths::Staging::file(parent)?;
    let mut work = Work::new(cancel, &mut progress, Some(total));
    let output = match format {
        ArchiveFormat::Zip => zip::create(output, &sources, &mut work)?,
        ArchiveFormat::SevenZip => seven_zip::create(output, &sources, &mut work)?,
    };
    output.sync_all().map_err(error)?;
    drop(output);
    check_cancel(cancel)?;
    stage.publish_file(destination)?;
    work.emit(true);
    Ok(destination.to_path_buf())
}

fn selected_path(path: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|p| {
            path == p
                || path
                    .strip_prefix(p)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

fn validate_entries(entries: &[ArchiveEntry]) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err("圧縮ファイルの項目数が上限（100,000）を超えています".into());
    }
    let mut names = 0usize;
    let mut total = 0u64;
    let mut used = HashMap::<String, bool>::new();
    let mut spelling = HashMap::<String, String>::new();
    let mut spelling_bytes = 0usize;
    for entry in entries {
        if paths::normalize(&entry.path)? != entry.path {
            return Err("圧縮ファイルの項目名が不正です".into());
        }
        names = names
            .checked_add(entry.path.len())
            .ok_or("項目名が長すぎます")?;
        total = total
            .checked_add(entry.size)
            .ok_or("展開サイズが大きすぎます")?;
        if names > MAX_NAMES_BYTES || total > MAX_EXPANDED_BYTES || entry.size > MAX_MEMBER_BYTES {
            return Err(
                "圧縮ファイルの制限を超えています（名前32MiB、1項目16GiB、合計64GiB）".into(),
            );
        }
        if entry.is_dir && entry.size != 0 {
            return Err("データを持つフォルダー項目には対応していません".into());
        }
        let key = entry.path.to_uppercase();
        if used.insert(key, entry.is_dir).is_some() {
            return Err(format!("重複する項目名があります: {}", entry.path));
        }
        let mut component = entry.path.as_str();
        loop {
            let key = component.to_uppercase();
            if spelling
                .get(&key)
                .is_some_and(|previous| previous != component)
            {
                return Err(format!(
                    "大文字・小文字で衝突する項目名があります: {component}"
                ));
            }
            if !spelling.contains_key(&key) {
                spelling_bytes = spelling_bytes
                    .checked_add(key.len() + component.len())
                    .ok_or("項目名が長すぎます")?;
                if spelling_bytes > 2 * MAX_NAMES_BYTES {
                    return Err("圧縮ファイルのフォルダー構造が上限（32MiB）を超えています".into());
                }
                spelling.insert(key, component.to_owned());
            }
            let Some((parent, _)) = component.rsplit_once('/') else {
                break;
            };
            component = parent;
        }
    }
    for entry in entries {
        let mut parent = entry.path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if used.get(&prefix.to_uppercase()) == Some(&false) {
                return Err(format!(
                    "ファイルとフォルダーの名前が衝突しています: {prefix}"
                ));
            }
            parent = prefix;
        }
    }
    Ok(())
}

struct SourceEntry {
    path: PathBuf,
    entry: ArchiveEntry,
}

fn source_plan(
    paths: &[PathBuf],
    destination: &Path,
    cancel: &CancelToken,
) -> Result<Vec<SourceEntry>, String> {
    if paths.is_empty() {
        return Err("圧縮する項目を選択してください".into());
    }
    if paths.len() > MAX_ENTRIES {
        return Err("圧縮する項目数が上限（100,000）を超えています".into());
    }
    let mut unique = HashSet::<PathBuf>::new();
    let mut path_bytes = 0usize;
    for path in paths {
        check_cancel(cancel)?;
        paths::check_ancestors(path)?;
        let path = path.canonicalize().map_err(error)?;
        if path.file_name().is_none() {
            return Err("ドライブ全体は圧縮できません".into());
        }
        if !unique.contains(&path) {
            path_bytes = path_bytes
                .checked_add(path.as_os_str().len())
                .ok_or("圧縮するパスが長すぎます")?;
            if path_bytes > 2 * MAX_NAMES_BYTES {
                return Err("圧縮するパスの合計長が上限を超えています".into());
            }
            unique.insert(path);
        }
    }
    // Ancestor lookup avoids comparing every selected item against every other item.
    let roots: Vec<_> = unique
        .iter()
        .filter(|path| {
            !path
                .ancestors()
                .skip(1)
                .any(|parent| unique.contains(parent))
        })
        .cloned()
        .collect();
    drop(unique);
    let destination = destination
        .parent()
        .ok_or("保存先が不正です")?
        .canonicalize()
        .map_err(error)?
        .join(destination.file_name().ok_or("保存先が不正です")?);
    let mut stack: Vec<_> = roots
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("UTF-8で表せない名前には対応していません")?
                .to_owned();
            Ok((p, name))
        })
        .collect::<Result<_, String>>()?;
    let mut result = Vec::new();
    let mut names: usize = stack.iter().map(|(_, name)| name.len()).sum();
    path_bytes = stack.iter().map(|(path, _)| path.as_os_str().len()).sum();
    while let Some((path, name)) = stack.pop() {
        check_cancel(cancel)?;
        if path == destination {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        paths::check_not_redirect(&path, &metadata)?;
        if paths::is_link(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
            return Err(format!(
                "リンク・特殊ファイルは圧縮できません: {}",
                path.display()
            ));
        }
        let name = paths::normalize(&name)?;
        if names > MAX_NAMES_BYTES || result.len() >= MAX_ENTRIES || stack.len() > MAX_ENTRIES {
            return Err("圧縮する項目が上限を超えています".into());
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path).map_err(error)? {
                check_cancel(cancel)?;
                let child = child.map_err(error)?;
                let child_name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| "UTF-8で表せない名前には対応していません")?;
                let child_name = format!("{name}/{child_name}");
                names = names
                    .checked_add(child_name.len())
                    .ok_or("項目名が長すぎます")?;
                if names > MAX_NAMES_BYTES {
                    return Err("圧縮する項目名が上限（32MiB）を超えています".into());
                }
                let child_path = child.path();
                path_bytes = path_bytes
                    .checked_add(child_path.as_os_str().len())
                    .ok_or("圧縮するパスが長すぎます")?;
                if path_bytes > 2 * MAX_NAMES_BYTES {
                    return Err("圧縮するパスの合計長が上限を超えています".into());
                }
                stack.push((child_path, child_name));
                if stack.len() + result.len() > MAX_ENTRIES {
                    return Err("圧縮する項目数が上限（100,000）を超えています".into());
                }
            }
        }
        result.push(SourceEntry {
            path,
            entry: ArchiveEntry {
                path: name,
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                modified: metadata.modified().ok(),
            },
        });
    }
    validate_entries(&result.iter().map(|s| s.entry.clone()).collect::<Vec<_>>())?;
    result.sort_by(|a, b| a.entry.path.cmp(&b.entry.path));
    Ok(result)
}

fn same_metadata(meta: &Metadata, index: &ArchiveIndex) -> bool {
    meta.len() == index.source_len && meta.modified().ok() == index.modified
}
fn check_cancel(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("圧縮ファイルの操作をキャンセルしました".into())
    } else {
        Ok(())
    }
}
fn error(err: impl std::fmt::Display) -> String {
    format!("圧縮ファイルを処理できません: {err}")
}
fn io_error(err: impl Into<String>) -> io::Error {
    io::Error::other(err.into())
}

struct CancelReader<R> {
    inner: R,
    cancel: CancelToken,
}
impl<R: Read> Read for CancelReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        check_cancel(&self.cancel).map_err(io_error)?;
        self.inner.read(buf)
    }
}
impl<R: Seek> Seek for CancelReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        check_cancel(&self.cancel).map_err(io_error)?;
        self.inner.seek(pos)
    }
}

struct Work<'a> {
    cancel: &'a CancelToken,
    callback: &'a mut dyn FnMut(ArchiveProgress),
    state: ArchiveProgress,
    decoded: u64,
    last_emit: Instant,
    directories: HashSet<String>,
}
impl<'a> Work<'a> {
    fn new(
        cancel: &'a CancelToken,
        callback: &'a mut dyn FnMut(ArchiveProgress),
        total: Option<u64>,
    ) -> Self {
        Self {
            cancel,
            callback,
            state: ArchiveProgress {
                processed_bytes: 0,
                total_bytes: total,
                current_name: String::new(),
                entries_done: 0,
            },
            decoded: 0,
            last_emit: Instant::now(),
            directories: HashSet::new(),
        }
    }
    fn emit(&mut self, force: bool) {
        if force || self.last_emit.elapsed() >= Duration::from_millis(50) {
            (self.callback)(self.state.clone());
            self.last_emit = Instant::now();
        }
    }
    fn bytes(&mut self, count: usize, selected: bool) -> io::Result<()> {
        check_cancel(self.cancel).map_err(io_error)?;
        self.decoded = self
            .decoded
            .checked_add(count as u64)
            .ok_or_else(|| io_error("展開サイズが大きすぎます"))?;
        if self.decoded > MAX_EXPANDED_BYTES {
            return Err(io_error("展開サイズの上限（64GiB）を超えています"));
        }
        if selected {
            self.state.processed_bytes += count as u64;
        }
        self.emit(false);
        Ok(())
    }
    fn complete_entry(&mut self) {
        self.state.entries_done += 1;
        self.emit(false);
    }
    fn copy(
        &mut self,
        reader: &mut dyn Read,
        writer: &mut dyn Write,
        expected: u64,
        selected: bool,
    ) -> Result<(), String> {
        let mut buffer = vec![0u8; COPY_BUFFER];
        let mut read = 0u64;
        loop {
            check_cancel(self.cancel)?;
            let count = reader.read(&mut buffer).map_err(error)?;
            if count == 0 {
                break;
            }
            read = read
                .checked_add(count as u64)
                .ok_or("展開サイズが大きすぎます")?;
            if read > expected || read > MAX_MEMBER_BYTES {
                return Err("宣言されたサイズを超える圧縮データです".into());
            }
            self.bytes(count, selected).map_err(error)?;
            writer.write_all(&buffer[..count]).map_err(error)?;
        }
        if read != expected {
            return Err("圧縮データが途中で終わっています".into());
        }
        Ok(())
    }
}

struct SourceReader<'a, 'b> {
    file: File,
    work: &'a mut Work<'b>,
    read: u64,
    expected: u64,
}
impl Read for SourceReader<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check_cancel(self.work.cancel).map_err(io_error)?;
        let size = buffer.len().min(COPY_BUFFER);
        let count = self.file.read(&mut buffer[..size])?;
        self.read += count as u64;
        if self.read > self.expected || (count == 0 && self.read != self.expected) {
            return Err(io_error("圧縮中に元ファイルのサイズが変更されました"));
        }
        self.work.bytes(count, true)?;
        Ok(count)
    }
}
fn open_source<'a, 'b>(
    source: &SourceEntry,
    work: &'a mut Work<'b>,
) -> Result<SourceReader<'a, 'b>, String> {
    paths::check_ancestors(&source.path)?;
    let file = paths::open_regular(&source.path)?;
    let meta = file.metadata().map_err(error)?;
    if meta.len() != source.entry.size || meta.modified().ok() != source.entry.modified {
        return Err("圧縮前に元ファイルが変更されました".into());
    }
    Ok(SourceReader {
        file,
        work,
        read: 0,
        expected: source.entry.size,
    })
}

#[cfg(test)]
mod tests;
