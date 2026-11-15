use super::*;
use std::{
    fs::OpenOptions,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

pub(super) fn normalize(name: &str) -> Result<String, String> {
    let name = name.replace('\\', "/");
    let name = name.strip_suffix('/').unwrap_or(&name);
    if name.is_empty() || name.len() > 4096 || name.starts_with('/') {
        return Err("空・絶対パス・長すぎる圧縮項目名は使用できません".into());
    }
    for component in name.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.encode_utf16().count() > 255
            || component
                .chars()
                .any(|ch| ch < ' ' || "<>:\"|?*".contains(ch))
        {
            return Err(format!("安全に展開できない項目名です: {name}"));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .trim_end_matches(' ')
            .to_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if reserved {
            return Err(format!("予約された項目名です: {name}"));
        }
    }
    Ok(name.to_owned())
}

pub(super) fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
pub(super) fn is_name_surrogate(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

#[cfg(windows)]
pub(super) fn is_cloud_tag(tag: u32) -> bool {
    tag & !0x0000_f000 == 0x9000_001a || matches!(tag, 0x8000_0015 | 0x8000_001e | 0x8000_0021)
}

#[cfg(windows)]
fn handle_tag(file: &File) -> Result<u32, String> {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo, GetFileInformationByHandleEx,
    };
    if file.metadata().map_err(error)?.file_attributes() & 0x400 == 0 {
        return Ok(0);
    }
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(io::Error::last_os_error()));
    }
    Ok(if info.FileAttributes & 0x400 != 0 {
        info.ReparseTag
    } else {
        0
    })
}

#[cfg(windows)]
fn path_tag(path: &Path, metadata: &Metadata) -> Result<u32, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    if metadata.file_attributes() & 0x400 == 0 {
        return Ok(0);
    }
    // Query the reparse point itself, without hydrating cloud files or following links.
    let file = OpenOptions::new()
        .access_mode(0x80)
        .share_mode(7)
        .custom_flags(0x0020_0000 | 0x0200_0000)
        .open(path)
        .map_err(error)?;
    handle_tag(&file)
}

pub(super) fn check_not_redirect(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if is_link(metadata) {
        return Err(format!(
            "リンクを通るパスには対応していません: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    if is_name_surrogate(path_tag(path, metadata)?) {
        return Err(format!(
            "リンクを通るパスには対応していません: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Reject redirects in every existing component, including junctions and mount-point reparse tags.
pub(super) fn check_ancestors(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("圧縮ファイルの操作には絶対パスが必要です".into());
    }
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(error)?;
        check_not_redirect(ancestor, &meta)?;
    }
    Ok(())
}

pub(super) fn check_directory(path: &Path) -> Result<(), String> {
    check_ancestors(path)?;
    if !fs::metadata(path).map_err(error)?.is_dir() {
        return Err("保存先はフォルダーを指定してください".into());
    }
    Ok(())
}

pub(super) fn open_regular(path: &Path) -> Result<File, String> {
    check_ancestors(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let tag = path_tag(path, &fs::symlink_metadata(path).map_err(error)?)?;
        // Cloud providers need normal reads to hydrate file data. Other reparse points
        // are opened as objects; name-surrogate tags were rejected above.
        if !is_cloud_tag(tag) {
            options.custom_flags(0x00200000);
        }
        options.share_mode(1); // FILE_SHARE_READ: source cannot change or be replaced while reading.
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(error)?;
    let meta = file.metadata().map_err(error)?;
    if !meta.is_file() || is_link(&meta) {
        return Err("通常のファイルではありません".into());
    }
    #[cfg(windows)]
    if is_name_surrogate(handle_tag(&file)?) {
        return Err("リンクを指すファイルは読み取れません".into());
    }
    Ok(file)
}

/// Root is a newly-created owned stage. Never join raw archive names here.
pub(super) fn prepare_output(
    root: &Path,
    entry: &ArchiveEntry,
    created: &mut HashSet<String>,
) -> Result<Option<File>, String> {
    let validated = normalize(&entry.path)?;
    check_directory(root)?;
    let mut path = root.to_path_buf();
    let parts: Vec<_> = validated.split('/').collect();
    let mut relative = String::new();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        if i > 0 {
            relative.push('/');
        }
        relative.push_str(part);
        if i + 1 < parts.len() || entry.is_dir {
            match fs::create_dir(&path) {
                Ok(()) => {
                    created.insert(relative.clone());
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    // NTFS short names may alias a previously created long directory.
                    // Accept only the exact relative spelling created by this operation.
                    if !created.contains(&relative) {
                        return Err(
                            "展開先で項目名が衝突しました（短いファイル名の別名を含む）".into()
                        );
                    }
                    let meta = fs::symlink_metadata(&path).map_err(error)?;
                    check_not_redirect(&path, &meta)?;
                    if !meta.is_dir() {
                        return Err("展開先のフォルダーが変更されました".into());
                    }
                }
                Err(e) => return Err(error(e)),
            }
        }
    }
    if entry.is_dir {
        return Ok(None);
    }
    check_directory(path.parent().ok_or("展開先が不正です")?)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(Some)
        .map_err(error)
}

pub(super) fn set_modified(file: &File, modified: Option<SystemTime>) -> Result<(), String> {
    if let Some(time) = modified {
        file.set_times(fs::FileTimes::new().set_modified(time))
            .map_err(error)?;
    }
    Ok(())
}

pub(super) struct Staging {
    pub path: PathBuf,
    directory: bool,
    owned: bool,
}
impl Staging {
    fn candidate(parent: &Path) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        parent.join(format!(
            ".skylis-archive-{}-{stamp:x}-{}",
            std::process::id(),
            NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
        ))
    }
    pub fn directory(parent: &Path) -> Result<Self, String> {
        check_directory(parent)?;
        for _ in 0..64 {
            let path = Self::candidate(parent);
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        directory: true,
                        owned: true,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(e)),
            }
        }
        Err("一時フォルダーを作成できません".into())
    }
    pub fn file(parent: &Path) -> Result<(Self, File), String> {
        check_directory(parent)?;
        for _ in 0..64 {
            let path = Self::candidate(parent);
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            directory: false,
                            owned: true,
                        },
                        file,
                    ));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(e)),
            }
        }
        Err("一時ファイルを作成できません".into())
    }
    pub fn publish_file(mut self, destination: &Path) -> Result<(), String> {
        check_directory(destination.parent().ok_or("保存先が不正です")?)?;
        rename_no_replace(&self.path, destination).map_err(error)?;
        self.owned = false;
        Ok(())
    }
    pub fn publish_unique_directory(
        mut self,
        parent: &Path,
        stem: &str,
    ) -> Result<PathBuf, String> {
        check_directory(parent)?;
        for index in 1..=10_000 {
            let destination = parent.join(if index == 1 {
                stem.to_owned()
            } else {
                format!("{stem} ({index})")
            });
            match rename_no_replace(&self.path, &destination) {
                Ok(()) => {
                    self.owned = false;
                    return Ok(destination);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(e)),
            }
        }
        Err("展開先の名前を確保できません".into())
    }
}

/// Preserve a downloaded archive's Windows security zone on every extracted file.
/// A bounded raw copy retains HostUrl/ReferrerUrl and unfamiliar future fields.
pub(super) fn read_zone(path: &Path) -> Result<Option<Vec<u8>>, String> {
    #[cfg(windows)]
    {
        let source = open_regular(path)?;
        if !volume_supports_named_streams(&source)? {
            return Ok(None);
        }
        let mut stream = path.as_os_str().to_os_string();
        stream.push(":Zone.Identifier");
        let file = match File::open(PathBuf::from(stream)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("ダウンロード元の安全情報を読み取れません: {e}")),
        };
        let mut data = Vec::new();
        file.take(65_537).read_to_end(&mut data).map_err(error)?;
        if data.len() > 65_536 {
            return Err("ダウンロード元の安全情報が上限（64KiB）を超えています".into());
        }
        Ok(Some(data))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(None)
    }
}

pub(super) fn apply_zone(
    root: &Path,
    entries: &[ArchiveEntry],
    selected: &[String],
    zone: Option<&[u8]>,
    cancel: &CancelToken,
) -> Result<(), String> {
    let Some(zone) = zone else {
        return Ok(());
    };
    #[cfg(windows)]
    for entry in entries
        .iter()
        .filter(|e| !e.is_dir && selected_path(&e.path, selected))
    {
        check_cancel(cancel)?;
        let path = root.join(normalize(&entry.path)?);
        let file = open_regular(&path)?;
        if !volume_supports_named_streams(&file)? {
            return Err("展開先のファイルシステムはダウンロード元の安全情報を保持できないため、展開を中止しました".into());
        }
        drop(file);
        let mut stream = path.as_os_str().to_os_string();
        stream.push(":Zone.Identifier");
        let stream = PathBuf::from(stream);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stream)
            .map_err(|e| {
                format!("ダウンロード元の安全情報を保存できないため、展開を中止しました: {e}")
            })?;
        output.write_all(zone).map_err(error)?;
        output.sync_all().map_err(error)?;
        drop(output);
        // Do not publish if a filesystem silently discarded ADS or truncated the mark.
        let mut actual = Vec::new();
        File::open(&stream)
            .map_err(error)?
            .take(65_537)
            .read_to_end(&mut actual)
            .map_err(error)?;
        if actual != zone {
            return Err("ダウンロード元の安全情報を保持できないため、展開を中止しました".into());
        }
    }
    #[cfg(not(windows))]
    let _ = (root, entries, selected, zone, cancel);
    Ok(())
}

pub(super) fn check_free_space(directory: &Path, required: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let path = native_path(directory).map_err(error)?;
        let mut available = 0;
        // This is an early advisory check; every actual write must still handle disk-full.
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0
            && available < required
        {
            return Err("展開先の空き容量が不足しています".into());
        }
    }
    #[cfg(not(windows))]
    let _ = (directory, required);
    Ok(())
}
impl Drop for Staging {
    fn drop(&mut self) {
        if self.owned
            && let Ok(meta) = fs::symlink_metadata(&self.path)
        {
            // Never traverse a redirect that replaced the stage.
            if check_not_redirect(&self.path, &meta).is_ok() {
                if self.directory {
                    let _ = fs::remove_dir_all(&self.path);
                } else {
                    let _ = fs::remove_file(&self.path);
                }
            }
        }
    }
}

fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let source = native_path(source)?;
        let destination = native_path(destination)?;
        // No MOVEFILE_REPLACE_EXISTING: atomic publication never replaces any existing item.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "このOSの安全な保存処理には対応していません",
        ))
    }
}

#[cfg(windows)]
pub(super) fn named_streams_flag(flags: u32) -> bool {
    flags & 0x0004_0000 != 0
}

#[cfg(windows)]
fn volume_supports_named_streams(file: &File) -> Result<bool, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
    let mut flags = 0;
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(format!(
            "ファイルシステムの安全情報の対応状況を確認できません: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(named_streams_flag(flags))
}

/// Raw Win32 calls do not receive std::fs's automatic extended-length conversion.
#[cfg(windows)]
pub(super) fn native_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    let path = std::path::absolute(path)?;
    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    if raw.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "パスにNULが含まれています",
        ));
    }
    let mut result: Vec<u16> = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) | Prefix::Verbatim(_) => raw,
            Prefix::Disk(_) => "\\\\?\\".encode_utf16().chain(raw).collect(),
            Prefix::UNC(_, _) => "\\\\?\\UNC\\"
                .encode_utf16()
                .chain(raw.into_iter().skip(2))
                .collect(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "デバイスパスには対応していません",
                ));
            }
        },
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "絶対パスが必要です",
            ));
        }
    };
    result.push(0);
    Ok(result)
}
