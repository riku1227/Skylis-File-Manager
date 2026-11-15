//! Explicit Explorer context-command handoff. There is no resident window watcher.
//! Native source identity stays in a short-lived COM server; the UI holds only event handles.

pub const RETAINED_NOTICE: &str = "Skylisで開きました。元のExplorerを安全に特定できないか、複数のタブがあるため、そのまま残しています。";
const UNCERTAIN_NOTICE: &str = "Skylisで開きました。元のExplorerが閉じたか確認できませんでした。終了要求の処理が続いている可能性があります。";

pub struct PendingHandoff {
    started: bool,
    #[cfg(windows)]
    handle: Option<NativeHandle>,
}

#[cfg(windows)]
struct NativeHandle(*mut std::ffi::c_void);
// The native receiver owns event handles only, never apartment-bound COM interfaces.
#[cfg(windows)]
unsafe impl Send for NativeHandle {}
#[cfg(windows)]
impl Drop for NativeHandle {
    fn drop(&mut self) {
        unsafe {
            skylis_handoff_cancel(self.0);
        }
    }
}

impl PendingHandoff {
    pub fn from_token(token: &str) -> Option<Self> {
        if !crate::launch_request::valid_token(token) {
            return None;
        }
        #[cfg(windows)]
        {
            let token: Vec<u16> = token.encode_utf16().chain(Some(0)).collect();
            let handle = unsafe { skylis_handoff_open(token.as_ptr()) };
            (!handle.is_null()).then_some(Self {
                started: false,
                handle: Some(NativeHandle(handle)),
            })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// Called only after the requested folder's listing succeeds. Waiting is off the UI thread.
    /// Keep this guard in the folder until the reply; navigation/drop cancels even after ready.
    pub fn complete(&mut self) -> async_channel::Receiver<String> {
        let (sender, receiver) = async_channel::bounded(1);
        if std::mem::replace(&mut self.started, true) {
            return receiver;
        }
        #[cfg(windows)]
        if let Some(original) = self.handle.as_ref() {
            let duplicate = unsafe { skylis_handoff_duplicate(original.0) };
            if duplicate.is_null() {
                let _ = sender.try_send(RETAINED_NOTICE.into());
                return receiver;
            }
            let handle = NativeHandle(duplicate);
            let fallback = sender.clone();
            let spawned = std::thread::Builder::new()
                .name("explorer-handoff".into())
                .spawn(move || {
                    // complete_wait consumes and deletes the native allocation.
                    let result = unsafe { skylis_handoff_complete_wait(handle.0) };
                    std::mem::forget(handle);
                    let notice = match result {
                        1 => "Skylisへ切り替え、元のExplorerを閉じました。",
                        2 => RETAINED_NOTICE,
                        _ => UNCERTAIN_NOTICE,
                    };
                    let _ = sender.send_blocking(notice.into());
                });
            if spawned.is_err() {
                let _ = fallback.try_send(RETAINED_NOTICE.into());
            }
        }
        #[cfg(not(windows))]
        {
            let _ = sender.try_send(RETAINED_NOTICE.into());
        }
        receiver
    }
}

pub fn run_command_server() {
    #[cfg(windows)]
    unsafe {
        skylis_run_shell_command_server();
    }
}

#[cfg(windows)]
unsafe extern "C" {
    fn skylis_run_shell_command_server();
    fn skylis_handoff_open(token: *const u16) -> *mut std::ffi::c_void;
    fn skylis_handoff_duplicate(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn skylis_handoff_complete_wait(handle: *mut std::ffi::c_void) -> i32;
    fn skylis_handoff_cancel(handle: *mut std::ffi::c_void);
    #[cfg(test)]
    fn skylis_handoff_accept_close(flags: u32) -> i32;
    #[cfg(test)]
    fn skylis_handoff_test_recheck(first: u32, second: u32) -> u32;
    #[cfg(test)]
    fn skylis_handoff_test_fallback(
        directory: *const u16,
        parameters: *const u16,
        output: *mut u16,
        capacity: u32,
        check_directory: i32,
    ) -> u32;
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::{CreateEventW, SetEvent, WaitForSingleObject},
    };

    fn fallback(directory: &str, parameters: &str, check_directory: bool) -> Option<String> {
        let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let directory = wide(directory);
        let parameters = wide(parameters);
        let mut output = vec![0; 32761];
        let length = unsafe {
            skylis_handoff_test_fallback(
                directory.as_ptr(),
                parameters.as_ptr(),
                output.as_mut_ptr(),
                output.len() as u32,
                check_directory.into(),
            )
        };
        (length != 0).then(|| String::from_utf16(&output[..length as usize]).unwrap())
    }

    #[test]
    fn launch_fallback_preserves_unicode_and_prefers_explicit_parameters() {
        let path = r"C:\資料と画像\見積り 'final' & (2)";
        assert_eq!(
            fallback(
                r"C:\different",
                &format!("--switch-explorer \"{path}\""),
                false
            ),
            Some(path.into())
        );
        assert_eq!(
            fallback("", &format!("\"{path}\""), false),
            Some(path.into())
        );
        assert_eq!(fallback(path, "", false), Some(path.into()));
        // Invalid explicit parameters must not silently fall back to another working directory.
        assert!(fallback(path, "--unknown", false).is_none());
        assert!(fallback(path, "--switch-explorer \"C:\\valid\" --home", false).is_none());
    }

    #[test]
    fn launch_fallback_rejects_relative_device_malformed_and_oversized_paths() {
        for path in [
            "",
            r"relative\folder",
            r"C:relative",
            r"\rooted",
            r"\\server",
            r"\\.\C:\folder",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            r"C:\folder:stream",
            "C:\\line\nbreak",
            r"shell:Downloads",
            r"C:\wild*card",
        ] {
            assert!(fallback(path, "", false).is_none(), "{path}");
        }
        assert!(fallback("", "--switch-explorer \"C:\\unclosed", false).is_none());
        assert!(fallback("", "--switch-explorer \"C:\\folder\" garbage", false).is_none());
        assert!(fallback(&format!("C:\\{}", "x".repeat(32760)), "", false).is_none());
        assert!(fallback(r"C:\valid", &" ".repeat(32761), false).is_none());
        for path in [
            r"C:\.",
            r"\\server\share\.",
            r"\\?\C:\folder",
            r"\\?\UNC\server\share",
        ] {
            assert_eq!(fallback(path, "", false), Some(path.into()));
        }
    }

    #[test]
    fn launch_fallback_checks_directory_without_accepting_regular_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "skylis-handoff-fallback-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("file.txt");
        std::fs::write(&file, b"fixture").unwrap();
        assert!(fallback(&dir.to_string_lossy(), "", true).is_some());
        assert!(fallback(&file.to_string_lossy(), "", true).is_none());
        assert!(fallback(&dir.join("missing").to_string_lossy(), "", true).is_none());
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    struct TestEvents {
        token: String,
        handles: [HANDLE; 5],
    }
    impl TestEvents {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tick = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
                & 0xffffffffffff;
            let token = format!(
                "{:08x}-{:04x}-0000-0000-{:012x}",
                std::process::id(),
                id,
                tick
            );
            let handles = ["ready", "cancel", "closed", "retained", "uncertain"].map(|suffix| {
                let name: Vec<u16> = format!("Local\\Skylis.Handoff.{token}.{suffix}")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };
                assert!(!handle.is_null());
                handle
            });
            Self { token, handles }
        }
        fn signaled(&self, index: usize, timeout: u32) -> bool {
            unsafe { WaitForSingleObject(self.handles[index], timeout) == 0 }
        }
    }
    impl Drop for TestEvents {
        fn drop(&mut self) {
            for handle in self.handles {
                unsafe {
                    CloseHandle(handle);
                }
            }
        }
    }

    #[test]
    fn dropping_before_listing_completion_cancels_without_ready() {
        let events = TestEvents::new();
        let guard = PendingHandoff::from_token(&events.token).unwrap();
        assert!(!events.signaled(0, 0));
        drop(guard);
        assert!(events.signaled(1, 0));
        assert!(!events.signaled(0, 0));
    }
    #[test]
    fn navigation_can_cancel_after_listing_ack_during_close_verification() {
        let events = TestEvents::new();
        let mut guard = PendingHandoff::from_token(&events.token).unwrap();
        let receiver = guard.complete();
        assert!(events.signaled(0, 2000));
        drop(guard);
        assert!(events.signaled(1, 0));
        assert_eq!(receiver.recv_blocking().unwrap(), RETAINED_NOTICE);
    }
    #[test]
    fn successful_handoff_requires_explicit_closed_reply() {
        let events = TestEvents::new();
        let mut guard = PendingHandoff::from_token(&events.token).unwrap();
        let receiver = guard.complete();
        assert!(events.signaled(0, 2000));
        unsafe {
            SetEvent(events.handles[2]);
        }
        assert!(
            receiver
                .recv_blocking()
                .unwrap()
                .contains("元のExplorerを閉じました")
        );
        assert!(!events.signaled(1, 0));
    }
    #[test]
    fn uncertain_close_does_not_claim_explorer_was_retained() {
        let events = TestEvents::new();
        let mut guard = PendingHandoff::from_token(&events.token).unwrap();
        let receiver = guard.complete();
        assert!(events.signaled(0, 2000));
        unsafe {
            SetEvent(events.handles[4]);
        }
        assert_eq!(receiver.recv_blocking().unwrap(), UNCERTAIN_NOTICE);
    }
    #[test]
    fn close_requires_every_independent_proof() {
        // Actual adapter predicate: listing ack, exact site, HWND/PID lifetime,
        // same folder, single tab, still active source view, live launched child.
        let required = (1u32 << 7) - 1;
        assert_eq!(unsafe { skylis_handoff_accept_close(required) }, 1);
        for bit in 0..7 {
            assert_eq!(
                unsafe { skylis_handoff_accept_close(required & !(1 << bit)) },
                0
            );
        }
        assert_eq!(unsafe { skylis_handoff_accept_close(0) }, 0);
    }
    #[test]
    fn changes_between_native_verification_passes_never_reach_close() {
        let valid = (1u32 << 7) - 1;
        assert_eq!(
            unsafe { skylis_handoff_test_recheck(valid, valid) },
            2 | (1 << 8)
        );
        for bit in 0..7 {
            // Source view/folder/PID/tab/child/ack proof disappeared after the first check.
            assert_eq!(
                unsafe { skylis_handoff_test_recheck(valid, valid & !(1 << bit)) },
                2
            );
        }
        // Initial rejection must not make a second COM inspection or issue close.
        assert_eq!(unsafe { skylis_handoff_test_recheck(0, valid) }, 1);
    }
}
