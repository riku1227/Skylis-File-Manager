//! Read-only process resident system memory, including shared resident pages.
//! This is neither GPU VRAM nor virtual address space/private committed bytes.
//! Sampling frequency and visibility are controlled by the diagnostics owner.

pub(super) fn resident_bytes() -> Option<u64> {
    platform::resident_bytes()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::{
        Foundation::HANDLE,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetCurrentProcess,
        },
    };

    pub(super) fn resident_bytes() -> Option<u64> {
        // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle for this
        // process. It does not allocate a handle and must not be closed.
        query(unsafe { GetCurrentProcess() })
    }

    fn query(process: HANDLE) -> Option<u64> {
        // SAFETY: this Win32 output structure contains only integer fields.
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: counters is writable for exactly the size supplied. The OS
        // validates the process handle and reports failure without a value.
        let success = unsafe { GetProcessMemoryInfo(process, &mut counters, counters.cb) };
        (success != 0).then_some(counters.WorkingSetSize as u64)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn failed_query_is_unknown_instead_of_zero() {
            assert_eq!(super::query(std::ptr::null_mut()), None);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub(super) fn resident_bytes() -> Option<u64> {
        // SAFETY: the libc definition matches the system Mach output structure;
        // every field is an integer or an integer-only time value.
        let mut info: libc::mach_task_basic_info = unsafe { std::mem::zeroed() };
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        // SAFETY: task_info writes at most count integer_t words to the live
        // output structure. mach_task_self is a borrowed task port.
        let status = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                (&raw mut info).cast(),
                &mut count,
            )
        };
        (status == libc::KERN_SUCCESS && count >= libc::MACH_TASK_BASIC_INFO_COUNT)
            .then_some(info.resident_size)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub(super) fn resident_bytes() -> Option<u64> {
        None
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    #[test]
    fn current_process_has_resident_memory() {
        assert!(super::resident_bytes().is_some_and(|bytes| bytes > 0));
    }
}
