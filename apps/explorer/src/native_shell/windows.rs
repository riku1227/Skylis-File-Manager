use super::{MenuOutcome, MenuResult, Target, windows_path_wide};
use anyhow::{Context as _, bail};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::OnceCell,
    ffi::c_void,
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::ClientToScreen,
        System::{
            Com::CoTaskMemFree,
            LibraryLoader::GetModuleHandleW,
            Ole::{OleInitialize, OleUninitialize},
        },
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{Interface, PCSTR, PCWSTR, w},
};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;
const CANCEL_TIMER: usize = 1;
const CLASS_NAME: PCWSTR = w!("Skylis.ShellContextMenuHost");
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) fn show(
    target: Target,
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> anyhow::Result<async_channel::Receiver<MenuResult>> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("Windowsウィンドウを取得できません: {error}"))?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        bail!("Windowsウィンドウを取得できません");
    };
    let parent = raw.hwnd.get();
    let scale = window.scale_factor();
    let mut point = POINT {
        x: (f32::from(position.x) * scale).round() as i32,
        y: (f32::from(position.y) * scale).round() as i32,
    };
    // The handle is borrowed on the UI thread and never dereferenced by Rust. The worker
    // checks IsWindow before use; closing the GPUI window cancels an open native menu.
    unsafe {
        ClientToScreen(HWND(parent as *mut c_void), &mut point).ok()?;
    }
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        bail!("Windowsのメニューはすでに開いています");
    }
    let modifiers = window.modifiers();
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("shell-menu".into())
        .spawn(move || {
            let _active = ActiveGuard;
            let result = run(target, parent, point, modifiers, &sender)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.try_send(result);
        });
    if let Err(error) = spawned {
        ACTIVE.store(false, Ordering::Release);
        return Err(error).context("Windowsメニューの処理を開始できません");
    }
    Ok(receiver)
}

struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

struct Apartment;
impl Apartment {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            // OLE initializes the STA and its clipboard services used by Copy/Paste handlers.
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

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

struct Host(HWND);
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.0), CANCEL_TIMER);
            let _ = DestroyWindow(self.0);
        }
    }
}

struct ItemId(*mut Common::ITEMIDLIST);
impl ItemId {
    fn parse(path: &Path) -> anyhow::Result<Self> {
        let wide = windows_path_wide(path)?;
        let mut value = std::ptr::null_mut();
        // Hold the returned allocation even if parsing fails, in case the Shell populated it.
        let result =
            unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut value, 0, None) };
        let item = Self(value);
        result.with_context(|| format!("項目を取得できません: {}", path.display()))?;
        anyhow::ensure!(!item.0.is_null(), "Shellが項目を返しませんでした");
        Ok(item)
    }
}
impl Drop for ItemId {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}

// Boxed on the STA thread; no COM interface or pointer crosses into GPUI's thread.
struct HostState {
    menu2: OnceCell<IContextMenu2>,
    menu3: OnceCell<IContextMenu3>,
    parent: HWND,
    result: async_channel::Sender<MenuResult>,
}

fn run(
    target: Target,
    parent: isize,
    point: POINT,
    modifiers: gpui::Modifiers,
    sender: &async_channel::Sender<MenuResult>,
) -> anyhow::Result<MenuOutcome> {
    let _apartment = Apartment::new().context("ShellのCOM初期化に失敗しました")?;
    let parent = HWND(parent as *mut c_void);
    if sender.is_closed() || !unsafe { IsWindow(Some(parent)).as_bool() } {
        return Ok(MenuOutcome::Cancelled);
    }
    register_class()?;
    let state = Box::new(HostState {
        menu2: OnceCell::new(),
        menu3: OnceCell::new(),
        parent,
        result: sender.clone(),
    });
    // `state` remains allocated until after Host::drop has destroyed its window.
    let host = Host(unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!(""),
            WS_POPUP,
            point.x,
            point.y,
            0,
            0,
            Some(parent),
            None,
            Some(HINSTANCE(GetModuleHandleW(None)?.0)),
            Some((&*state as *const HostState).cast()),
        )?
    });
    let context = context_menu(&target, host.0)?;
    if let Ok(menu) = context.cast() {
        let _ = state.menu3.set(menu);
    }
    if let Ok(menu) = context.cast() {
        let _ = state.menu2.set(menu);
    }
    let popup = Menu(unsafe { CreatePopupMenu()? });
    // CMF_CANRENAME is intentionally absent: rename is an in-place Shell-view operation,
    // which this menu host does not implement. Skylis supplies its own rename operation.
    unsafe {
        context
            .QueryContextMenu(
                popup.0,
                0,
                FIRST_COMMAND,
                LAST_COMMAND,
                if modifiers.shift {
                    CMF_EXTENDEDVERBS
                } else {
                    CMF_NORMAL
                },
            )
            .ok()
            .context("Windowsメニューを作成できません")?;
    }
    if sender.is_closed()
        || !unsafe { IsWindow(Some(parent)).as_bool() }
        || unsafe { GetForegroundWindow() } != parent
    {
        return Ok(MenuOutcome::Cancelled);
    }
    anyhow::ensure!(
        unsafe { GetMenuItemCount(Some(popup.0)) } > 0,
        "表示できるWindowsメニューがありません"
    );
    unsafe {
        // A visible zero-sized tool window gives this STA its own foreground/input queue.
        // No subclassing or cross-thread callbacks into GPUI are required.
        let _ = ShowWindow(host.0, SW_SHOWNOACTIVATE);
        if !SetForegroundWindow(host.0).as_bool() {
            return Ok(MenuOutcome::Cancelled);
        }
        anyhow::ensure!(
            SetTimer(Some(host.0), CANCEL_TIMER, 100, None) != 0,
            "メニュー監視を開始できません"
        );
        let selected = TrackPopupMenuEx(
            popup.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            point.x,
            point.y,
            host.0,
            None,
        )
        .0 as u32;
        // Required by TrackPopupMenu's notification-area/activation behavior. Do not steal
        // focus back if the user switched to a different application while the menu was open.
        let _ = PostMessageW(Some(host.0), WM_NULL, WPARAM(0), LPARAM(0));
        if GetForegroundWindow() == host.0 && IsWindow(Some(parent)).as_bool() {
            let _ = SetForegroundWindow(parent);
        }
        let Some(offset) = command_offset(selected) else {
            return Ok(MenuOutcome::Cancelled);
        };
        if sender.is_closed() || !IsWindow(Some(parent)).as_bool() {
            return Ok(MenuOutcome::Cancelled);
        }
        let working_directory = match &target {
            Target::Selection(paths) => paths[0].parent(),
            Target::Background(path) => Some(path.as_path()),
        }
        .map(windows_path_wide)
        .transpose()?;
        let invoke = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: SEE_MASK_UNICODE
                | SEE_MASK_NOASYNC
                | CMIC_MASK_PTINVOKE
                | if modifiers.shift {
                    CMIC_MASK_SHIFT_DOWN
                } else {
                    0
                }
                | if modifiers.control {
                    CMIC_MASK_CONTROL_DOWN
                } else {
                    0
                },
            hwnd: parent,
            lpVerb: PCSTR(offset as usize as *const u8),
            lpVerbW: PCWSTR(offset as usize as *const u16),
            lpDirectoryW: working_directory
                .as_ref()
                .map_or(PCWSTR::null(), |path| PCWSTR(path.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: point,
            ..Default::default()
        };
        context
            .InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast())
            .context("Windowsメニューの操作を実行できません")?;
    }
    Ok(MenuOutcome::Invoked)
}

pub(super) fn context_menu(target: &Target, owner: HWND) -> anyhow::Result<IContextMenu> {
    unsafe {
        match target {
            Target::Selection(paths) => {
                let ids = paths
                    .iter()
                    .map(|path| ItemId::parse(path))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let parent: IShellFolder = SHBindToParent(ids[0].0, None)?;
                let children: Vec<_> = ids
                    .iter()
                    .map(|id| ILFindLastID(id.0) as *const _)
                    .collect();
                Ok(parent.GetUIObjectOf(owner, &children, None)?)
            }
            Target::Background(path) => {
                let wide = windows_path_wide(path)?;
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
                let folder: IShellFolder = item.BindToHandler(None, &BHID_SFObject)?;
                Ok(folder.CreateViewObject(owner)?)
            }
        }
    }
}

fn command_offset(command: u32) -> Option<u32> {
    (FIRST_COMMAND..=LAST_COMMAND)
        .contains(&command)
        .then(|| command - FIRST_COMMAND)
}

fn register_class() -> anyhow::Result<()> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            let instance = GetModuleHandleW(None).map_err(|error| error.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: HINSTANCE(instance.0),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                Err(windows::core::Error::from_win32().to_string())
            } else {
                Ok(())
            }
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("Shellメニューウィンドウを登録できません: {error}"))?;
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Only CreateWindowExW supplies WM_NCCREATE, with a CREATESTRUCTW pointer. All subsequent
    // messages borrow HostState until DestroyWindow returns; the procedure never owns it.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const HostState;
        if let Some(state) = state.as_ref() {
            if message == WM_TIMER && wparam.0 == CANCEL_TIMER {
                if state.result.is_closed() || !IsWindow(Some(state.parent)).as_bool() {
                    let _ = EndMenu();
                }
                return LRESULT(0);
            }
            if matches!(
                message,
                WM_INITMENUPOPUP | WM_UNINITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
            ) {
                if let Some(menu) = state.menu3.get() {
                    let mut result = LRESULT(0);
                    if menu
                        .HandleMenuMsg2(message, wparam, lparam, Some(&mut result))
                        .is_ok()
                    {
                        return result;
                    }
                }
                if let Some(menu) = state.menu2.get() {
                    if menu.HandleMenuMsg(message, wparam, lparam).is_ok() {
                        return LRESULT(if matches!(message, WM_DRAWITEM | WM_MEASUREITEM) {
                            1
                        } else {
                            0
                        });
                    }
                }
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_out_of_range_ids_cannot_invoke_commands() {
        assert_eq!(command_offset(0), None);
        assert_eq!(command_offset(FIRST_COMMAND), Some(0));
        assert_eq!(command_offset(LAST_COMMAND), Some(LAST_COMMAND - 1));
        assert_eq!(command_offset(LAST_COMMAND + 1), None);
    }

    /// Read-only integration check. Override the input to reproduce a real CLI-opened
    /// path; neither a context menu nor an action is invoked.
    #[test]
    fn native_parsing_accepts_forward_slash_paths() {
        let _apartment = Apartment::new().unwrap();
        let original = std::env::var_os("SKYLIS_SHELL_SMOKE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
        assert!(original.exists());
        let forward = std::path::PathBuf::from(original.to_str().unwrap().replace('\\', "/"));
        let _pidl = ItemId::parse(&forward).unwrap();
        let wide = windows_path_wide(&forward).unwrap();
        let _: IShellItem =
            unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.unwrap();
    }

    /// Opt-in because this loads the computer's installed third-party Shell extensions.
    /// It populates and destroys menus without showing them or invoking any command.
    #[test]
    #[ignore = "loads installed Windows Shell extensions; run explicitly on a desktop"]
    fn native_menu_population_smoke() {
        let _apartment = Apartment::new().unwrap();
        let folder = std::env::temp_dir().join(format!("skylis-shell-menu-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let first = folder.join("first.txt");
        let second = folder.join("second.txt");
        std::fs::write(&first, "menu smoke test").unwrap();
        std::fs::write(&second, "menu smoke test").unwrap();
        let result = (|| -> anyhow::Result<()> {
            for target in [
                Target::Selection(vec![first.clone(), second.clone()]),
                Target::Background(folder.clone()),
            ] {
                let context = context_menu(&target, HWND::default())?;
                let menu = Menu(unsafe { CreatePopupMenu()? });
                unsafe {
                    context
                        .QueryContextMenu(menu.0, 0, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL)
                        .ok()?;
                    anyhow::ensure!(GetMenuItemCount(Some(menu.0)) > 0, "empty Shell menu");
                }
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        let _ = std::fs::remove_dir(folder);
        result.unwrap();
    }
}
