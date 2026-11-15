//! Native properties use a worker-owned dialog host, so the GPUI window remains usable.
use std::path::PathBuf;

pub fn active() -> bool {
    #[cfg(windows)]
    {
        platform::ACTIVE.load(std::sync::atomic::Ordering::Acquire)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn show(
    paths: Vec<PathBuf>,
    window: &gpui::Window,
) -> Result<async_channel::Receiver<Result<(), String>>, String> {
    super::validate_selection(&paths).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        platform::show(paths, window).map_err(|e| format!("{e:#}"))
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, window);
        Err("このOSのプロパティ表示にはまだ対応していません".into())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use anyhow::Context as _;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        ffi::c_void,
        sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        },
    };
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            System::{
                LibraryLoader::GetModuleHandleW,
                Ole::{OleInitialize, OleUninitialize},
            },
            UI::{Shell::*, WindowsAndMessaging::*},
        },
        core::{PCWSTR, s, w},
    };
    pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
    const CLASS: PCWSTR = w!("Skylis.PropertiesHost");
    const CANCEL_TIMER: usize = 1;
    type Reply = async_channel::Sender<Result<(), String>>;

    pub(super) fn show(
        paths: Vec<PathBuf>,
        window: &gpui::Window,
    ) -> anyhow::Result<async_channel::Receiver<Result<(), String>>> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|e| anyhow::anyhow!("Windowsウィンドウを取得できません: {e}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("Windowsウィンドウを取得できません");
        };
        let parent = raw.hwnd.get();
        anyhow::ensure!(
            ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "プロパティはすでに開いています"
        );
        let (sender, receiver) = async_channel::bounded(1);
        if let Err(error) = std::thread::Builder::new()
            .name("shell-properties".into())
            .spawn(move || {
                let _active = Active;
                let result = run(paths, parent, &sender).map_err(|e| format!("{e:#}"));
                let _ = sender.try_send(result);
            })
        {
            ACTIVE.store(false, Ordering::Release);
            return Err(error).context("プロパティの表示を開始できません");
        }
        Ok(receiver)
    }
    struct Active;
    impl Drop for Active {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Release);
        }
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                OleUninitialize();
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
    struct Popup(HMENU);
    impl Drop for Popup {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyMenu(self.0);
            }
        }
    }
    struct State {
        sender: Reply,
    }

    fn register() -> anyhow::Result<()> {
        static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
        REGISTERED
            .get_or_init(|| unsafe {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: HINSTANCE(GetModuleHandleW(None).map_err(|e| e.to_string())?.0),
                    lpszClassName: CLASS,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    Err(windows::core::Error::from_win32().to_string())
                } else {
                    Ok(())
                }
            })
            .as_ref()
            .map_err(|e| anyhow::anyhow!("プロパティウィンドウを登録できません: {e}"))
            .copied()
    }
    fn owned_dialogs(host: HWND) -> Vec<HWND> {
        struct Scan {
            host: HWND,
            dialogs: Vec<HWND>,
        }
        unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
            let scan = unsafe { &mut *(data.0 as *mut Scan) };
            let mut owner = unsafe { GetWindow(hwnd, GW_OWNER) }.ok();
            for _ in 0..16 {
                let Some(current) = owner else { break };
                if current == scan.host {
                    scan.dialogs.push(hwnd);
                    break;
                }
                owner = unsafe { GetWindow(current, GW_OWNER) }.ok();
            }
            true.into()
        }
        let mut scan = Scan {
            host,
            dialogs: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM((&mut scan as *mut Scan) as isize));
        }
        scan.dialogs
    }
    fn cancel_if_needed(host: HWND, state: &State) {
        if state.sender.is_closed() {
            for dialog in owned_dialogs(host) {
                unsafe {
                    let _ = PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
    fn has_visible_dialog(host: HWND) -> bool {
        owned_dialogs(host)
            .into_iter()
            .any(|dialog| unsafe { IsWindowVisible(dialog).as_bool() })
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            if message == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            if message == WM_TIMER && wparam.0 == CANCEL_TIMER {
                if let Some(state) =
                    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State).as_ref()
                {
                    cancel_if_needed(hwnd, state);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
    fn run(paths: Vec<PathBuf>, parent: isize, sender: &Reply) -> anyhow::Result<()> {
        unsafe {
            OleInitialize(None)?;
        }
        let _apartment = Apartment;
        let parent = HWND(parent as *mut c_void);
        if sender.is_closed() {
            return Ok(());
        }
        register()?;
        // Host lives on this STA and owns the property sheets. Passing the GPUI HWND
        // directly would allow a modal Shell sheet to disable the entire app window.
        let state = Box::new(State {
            sender: sender.clone(),
        });
        let mut bounds = RECT::default();
        let _ = unsafe { GetWindowRect(parent, &mut bounds) };
        let host = Host(unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS,
                w!(""),
                WS_POPUP,
                bounds.left + (bounds.right - bounds.left) / 2,
                bounds.top + (bounds.bottom - bounds.top) / 2,
                0,
                0,
                None,
                None,
                Some(HINSTANCE(GetModuleHandleW(None)?.0)),
                Some((&*state as *const State).cast()),
            )?
        });
        let timer = unsafe { SetTimer(Some(host.0), CANCEL_TIMER, 100, None) };
        anyhow::ensure!(timer != 0, "プロパティのキャンセル監視を開始できません");
        let target = crate::native_shell::Target::Selection(paths);
        let context = crate::native_shell::windows::context_menu(&target, host.0)?;
        let menu = Popup(unsafe { CreatePopupMenu()? });
        unsafe {
            context
                .QueryContextMenu(menu.0, 0, 1, 0x7fff, CMF_NORMAL)
                .ok()?;
        }
        if sender.is_closed() {
            return Ok(());
        }
        let invoke = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            // CMIC_MASK_NOASYNC is the SDK alias of SEE_MASK_NOASYNC.
            fMask: SEE_MASK_UNICODE | SEE_MASK_NOASYNC,
            hwnd: host.0,
            lpVerb: s!("properties"),
            lpVerbW: w!("properties"),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe {
            context
                .InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast())
                .context("プロパティを表示できません")?;
        }
        // Some handlers create modeless sheets even without ASYNCOK. Keep their owner
        // and COM apartment alive until its visible sheets close. An invisible IME
        // helper also belongs to this host and survives the sheet; it must not keep
        // ACTIVE set forever or prevent the last application window from closing.
        wait_for_property_sheets(host.0, &state);
        Ok(())
    }

    fn wait_for_property_sheets(host: HWND, state: &State) {
        while has_visible_dialog(host) {
            cancel_if_needed(host, state);
            unsafe {
                MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn hidden(owner: Option<HWND>) -> Host {
            Host(unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Skylis properties ownership test"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    owner,
                    None,
                    None,
                    None,
                )
                .unwrap()
            })
        }
        #[test]
        fn worker_cleanup_returns_with_only_hidden_owned_helpers() {
            let (cancel, cancellation) = async_channel::bounded(1);
            let (finished, completion) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let host = hidden(None);
                let _ime_helper = hidden(Some(host.0));
                let state = State { sender: cancel };
                wait_for_property_sheets(host.0, &state);
                finished.send(()).unwrap();
            });
            // Keep the receiver alive: normal sheet closure must finish without
            // needing tab cancellation to destroy an unrelated hidden IME helper.
            let result = completion.recv_timeout(std::time::Duration::from_secs(2));
            drop(cancellation);
            assert!(
                result.is_ok(),
                "hidden helper retained the properties worker"
            );
            worker.join().unwrap();
        }
        #[test]
        fn hidden_owned_helpers_do_not_keep_a_closed_property_sheet_alive() {
            let host = hidden(None);
            let helper = hidden(Some(host.0));
            assert!(owned_dialogs(host.0).contains(&helper.0));
            assert!(!has_visible_dialog(host.0));
            unsafe {
                let _ = ShowWindow(helper.0, SW_SHOWNOACTIVATE);
            }
            assert!(has_visible_dialog(host.0));
            unsafe {
                let _ = ShowWindow(helper.0, SW_HIDE);
            }
            assert!(!has_visible_dialog(host.0));
        }
        #[test]
        fn independent_property_host_survives_the_original_window() {
            let original = hidden(None);
            let host = hidden(None);
            let dialog = hidden(Some(host.0));
            let (sender, _receiver) = async_channel::bounded(1);
            let state = State { sender };
            drop(original);
            cancel_if_needed(host.0, &state);
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                assert!(IsWindow(Some(host.0)).as_bool());
                assert!(IsWindow(Some(dialog.0)).as_bool());
            }
        }
        #[test]
        fn cancellation_only_closes_dialogs_owned_by_its_worker_host() {
            // Fresh hidden test windows only; no registered Shell handler or user window.
            let parent = hidden(None);
            let host = hidden(Some(parent.0));
            let unrelated = hidden(None);
            let dialog = hidden(Some(host.0));
            let nested = hidden(Some(dialog.0));
            let owned = owned_dialogs(host.0);
            assert!(owned.contains(&dialog.0));
            assert!(owned.contains(&nested.0));
            assert!(!owned.contains(&parent.0));
            assert!(!owned.contains(&unrelated.0));
            let (sender, receiver) = async_channel::bounded(1);
            let state = State { sender };
            cancel_if_needed(host.0, &state);
            assert!(unsafe { IsWindow(Some(dialog.0)).as_bool() });
            drop(receiver);
            cancel_if_needed(host.0, &state);
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                assert!(!IsWindow(Some(dialog.0)).as_bool());
                assert!(!IsWindow(Some(nested.0)).as_bool());
                assert!(IsWindow(Some(parent.0)).as_bool());
                assert!(IsWindow(Some(unrelated.0)).as_bool());
                assert!(IsWindow(Some(host.0)).as_bool());
            }
        }
    }
}
