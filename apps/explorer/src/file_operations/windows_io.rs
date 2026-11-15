//! Windows file clipboard and native drag bridge. GPUI 0.1.15 reads CF_HDROP but does
//! not write it or start external drags on Windows. Shell IDataObject holds only memory;
//! preparing a drag does not stat files or load icon/thumbnail handlers on the UI thread.
use super::{TransferKind, validate_arguments};
use anyhow::Context as _;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    ffi::{OsString, c_void},
    mem::ManuallyDrop,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};
use windows::Win32::{
    Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
    System::{
        Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL},
        DataExchange::*,
        Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Ole::{
            CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, OleInitialize,
            OleUninitialize,
        },
    },
    UI::{
        Shell::{
            CFSTR_PREFERREDDROPEFFECT, DROPFILES, DragQueryFileW, HDROP, SHCreateDataObject,
            SHDoDragDrop,
        },
        WindowsAndMessaging::IsWindow,
    },
};

const MAX_PATHS: usize = 16_384;
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Created during a mouse gesture, then run on the foreground executor after the
/// current GPUI entity/window update has returned. Never call run while holding an
/// App/Window/Entity borrow: the native loop can deliver drops back into Skylis.
pub struct NativeFileDrag {
    owner: isize,
    data: Vec<u8>,
}
impl NativeFileDrag {
    pub fn new(paths: Vec<PathBuf>, window: &gpui::Window) -> anyhow::Result<Self> {
        Ok(Self {
            owner: owner_handle(window)?,
            data: encode_paths(&paths)?,
        })
    }

    /// Returns the chosen action, or None when Escape / dropping outside a target cancels.
    /// The receiving application performs the actual transfer. Do not copy/move again here.
    pub fn run(self) -> anyhow::Result<Option<TransferKind>> {
        let owner = HWND(self.owner as *mut c_void);
        anyhow::ensure!(
            unsafe { IsWindow(Some(owner)).as_bool() },
            "ドラッグ元のウィンドウが閉じられました"
        );
        let _apartment = Apartment::new()?;
        let data = data_object(&self.data)?;
        let result =
            unsafe { SHDoDragDrop(Some(owner), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE) }?;
        Ok(if result == DROPEFFECT_NONE {
            None
        } else if result.0 & DROPEFFECT_MOVE.0 != 0 {
            Some(TransferKind::Move)
        } else {
            Some(TransferKind::Copy)
        })
    }
}

#[derive(Debug)]
pub struct ClipboardFiles {
    pub paths: Vec<PathBuf>,
    pub kind: TransferKind,
    /// Use to clear a successful Cut/Paste only if the clipboard has not changed since read.
    pub sequence: u32,
}

pub fn write_file_clipboard(
    paths: &[PathBuf],
    kind: TransferKind,
    window: &gpui::Window,
) -> anyhow::Result<()> {
    let bytes = encode_paths(paths)?;
    let files = GlobalMemory::new(&bytes)?;
    let effect = GlobalMemory::new(
        &(match kind {
            TransferKind::Copy => DROPEFFECT_COPY.0,
            TransferKind::Move => DROPEFFECT_MOVE.0,
        })
        .to_le_bytes(),
    )?;
    let preferred = preferred_format()?;
    let owner = HWND(owner_handle(window)? as *mut c_void);
    let _clipboard = Clipboard::open(Some(owner))?;
    unsafe {
        EmptyClipboard()?;
        SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(files.0.0)))?;
        files.release();
        SetClipboardData(preferred, Some(HANDLE(effect.0.0)))?;
        effect.release();
    }
    Ok(())
}

pub fn read_file_clipboard() -> anyhow::Result<Option<ClipboardFiles>> {
    let _clipboard = Clipboard::open(None)?;
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32) }.is_err() {
        return Ok(None);
    }
    let handle = unsafe { GetClipboardData(CF_HDROP.0 as u32)? };
    let paths = paths_from_drop(HDROP(handle.0))?;
    let preferred = preferred_format()?;
    let mut kind = TransferKind::Copy;
    if unsafe { IsClipboardFormatAvailable(preferred) }.is_ok() {
        if let Ok(handle) = unsafe { GetClipboardData(preferred) } {
            let global = HGLOBAL(handle.0);
            if unsafe { GlobalSize(global) } >= std::mem::size_of::<u32>() {
                let pointer = unsafe { GlobalLock(global) };
                if !pointer.is_null() {
                    let value = unsafe { std::ptr::read_unaligned(pointer.cast::<u32>()) };
                    unsafe {
                        let _ = GlobalUnlock(global);
                    }
                    if value == DROPEFFECT_MOVE.0 {
                        kind = TransferKind::Move;
                    }
                }
            }
        }
    }
    Ok(Some(ClipboardFiles {
        paths,
        kind,
        sequence: unsafe { GetClipboardSequenceNumber() },
    }))
}

pub fn clear_file_clipboard(sequence: u32) -> anyhow::Result<bool> {
    // Clearing needs no clipboard owner window. The originating tab/window may already
    // be closed when a long-running move finishes; only the locked sequence matters.
    let _clipboard = Clipboard::open(None)?;
    if unsafe { GetClipboardSequenceNumber() } != sequence {
        return Ok(false);
    }
    unsafe {
        EmptyClipboard()?;
    }
    Ok(true)
}

fn owner_handle(window: &gpui::Window) -> anyhow::Result<isize> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("Windowsウィンドウを取得できません: {error}"))?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        anyhow::bail!("Windowsウィンドウが必要です");
    };
    Ok(raw.hwnd.get())
}

struct Apartment;
impl Apartment {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            OleInitialize(None)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

struct Clipboard;
impl Clipboard {
    fn open(owner: Option<HWND>) -> anyhow::Result<Self> {
        unsafe { OpenClipboard(owner) }
            .context("クリップボードを開けません。他のアプリの処理後にもう一度試してください")?;
        Ok(Self)
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct GlobalMemory(HGLOBAL);
impl GlobalMemory {
    fn new(bytes: &[u8]) -> windows::core::Result<Self> {
        let memory = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len())? });
        let pointer = unsafe { GlobalLock(memory.0) };
        if pointer.is_null() {
            return Err(windows::core::Error::from_win32());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            let _ = GlobalUnlock(memory.0);
        }
        Ok(memory)
    }
    fn release(self) {
        std::mem::forget(self);
    }
}
impl Drop for GlobalMemory {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}

fn preferred_format() -> anyhow::Result<u32> {
    let format = unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) };
    anyhow::ensure!(format != 0, "ファイルクリップボード形式を登録できません");
    Ok(format)
}

fn encode_paths(paths: &[PathBuf]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(!paths.is_empty(), "ファイルが選択されていません");
    validate_arguments(paths, &paths[0])?;
    anyhow::ensure!(
        paths.len() <= MAX_PATHS,
        "一度に操作できるファイル数を超えています"
    );
    // DROPFILES is five 32-bit fields: byte offset, point.x/y, fNC, fWide.
    // Serialize explicitly to avoid copying struct padding/uninitialized memory.
    let mut bytes = Vec::with_capacity(256);
    for field in [std::mem::size_of::<DROPFILES>() as u32, 0, 0, 0, 1] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    for path in paths {
        for unit in crate::native_shell::windows_path_wide(path)? {
            anyhow::ensure!(
                bytes.len() <= MAX_BYTES - 4,
                "ファイルパスの合計が大きすぎます"
            );
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    Ok(bytes)
}

fn paths_from_drop(drop: HDROP) -> anyhow::Result<Vec<PathBuf>> {
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) } as usize;
    anyhow::ensure!(
        count > 0 && count <= MAX_PATHS,
        "クリップボードのファイル数を処理できません"
    );
    let mut paths = Vec::with_capacity(count);
    let mut bytes = 0;
    for index in 0..count {
        let len = unsafe { DragQueryFileW(drop, index as u32, None) } as usize;
        anyhow::ensure!(
            len > 0 && len <= 32_767,
            "クリップボードのファイルパスが無効です"
        );
        bytes += (len + 1) * 2;
        anyhow::ensure!(
            bytes <= MAX_BYTES,
            "クリップボードのファイルパスが大きすぎます"
        );
        let mut buffer = vec![0_u16; len + 1];
        let copied = unsafe { DragQueryFileW(drop, index as u32, Some(&mut buffer)) } as usize;
        anyhow::ensure!(
            copied == len,
            "クリップボードのファイルパスを読み取れません"
        );
        paths.push(PathBuf::from(OsString::from_wide(&buffer[..len])));
    }
    validate_arguments(&paths, &paths[0])?;
    Ok(paths)
}

fn data_object(bytes: &[u8]) -> anyhow::Result<IDataObject> {
    let memory = GlobalMemory::new(bytes)?;
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    };
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: memory.0 },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    unsafe {
        let object: IDataObject = SHCreateDataObject(None, None, None)?;
        object.SetData(&format, &medium, true)?;
        memory.release();
        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_file_payload_roundtrips_unicode_and_spaces_without_filesystem_access() {
        let paths = vec![
            PathBuf::from("C:\\不存在\\日本語 and space.txt"),
            PathBuf::from("\\\\server\\share\\quoted'file.txt"),
        ];
        let bytes = encode_paths(&paths).unwrap();
        let memory = GlobalMemory::new(&bytes).unwrap();
        assert_eq!(paths_from_drop(HDROP(memory.0.0)).unwrap(), paths);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn windows_file_payload_uses_native_separators_for_cli_paths() {
        let bytes = encode_paths(&[PathBuf::from("C:/不存在/日本語.txt")]).unwrap();
        let memory = GlobalMemory::new(&bytes).unwrap();
        let paths = paths_from_drop(HDROP(memory.0.0)).unwrap();
        assert_eq!(
            paths[0].as_os_str(),
            std::ffi::OsStr::new(r"C:\不存在\日本語.txt")
        );
    }

    #[test]
    fn windows_shell_data_object_offers_real_file_drop_format() {
        let _apartment = Apartment::new().unwrap();
        let paths = vec![PathBuf::from("C:\\nonexistent\\file.txt")];
        let object = data_object(&encode_paths(&paths).unwrap()).unwrap();
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        unsafe {
            // Explorer prefers Shell ID lists to CF_HDROP. An empty advertised ID list
            // would hide our real paths, so the generic data object must not offer one.
            let shell_ids = FORMATETC {
                cfFormat: RegisterClipboardFormatW(windows::Win32::UI::Shell::CFSTR_SHELLIDLIST)
                    as u16,
                ..format
            };
            assert_ne!(object.QueryGetData(&shell_ids), windows::core::HRESULT(0));
            let mut medium = object.GetData(&format).unwrap();
            let actual = paths_from_drop(HDROP(medium.u.hGlobal.0));
            windows::Win32::System::Ole::ReleaseStgMedium(&mut medium);
            assert_eq!(actual.unwrap(), paths);
        }
    }
}
