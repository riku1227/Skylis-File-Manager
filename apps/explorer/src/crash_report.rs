//! Writes best-effort local diagnostics when a Rust thread panics.
use std::{
    backtrace::Backtrace,
    cmp::Reverse,
    collections::BinaryHeap,
    fs::{self, OpenOptions},
    io::{self, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{DateTime, Utc};

const KEEP_REPORTS: usize = 20;

pub(crate) fn install() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = save(info);
        previous(info);
    }));
}

fn logs_dir() -> PathBuf {
    #[cfg(windows)]
    {
        match std::env::var_os("LOCALAPPDATA").filter(|path| !path.is_empty()) {
            Some(root) => PathBuf::from(root).join("Skylis").join("logs"),
            None => crate::settings::settings_path().with_file_name("logs"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        explorer_core::platform::home_dir().join("Library/Logs/Skylis")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| explorer_core::platform::home_dir().join(".local/state"))
            .join("Skylis")
            .join("logs")
    }
}

fn report_name(now: DateTime<Utc>, pid: u32) -> String {
    format!("crash-{}-{pid}.log", now.format("%Y%m%d-%H%M%S"))
}

fn is_report_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix("crash-")
        .and_then(|s| s.strip_suffix(".log"))
    else {
        return false;
    };
    let mut parts = stem.split('-');
    let (Some(date), Some(time), Some(pid), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    date.len() == 8
        && time.len() == 6
        && date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit())
        && pid.bytes().all(|b| b.is_ascii_digit())
        && pid.parse::<u32>().is_ok()
}

fn prune(dir: &Path) -> io::Result<()> {
    // Retain only the newest candidates in memory, even in a crowded log directory.
    let mut newest = BinaryHeap::with_capacity(KEEP_REPORTS + 1);
    for entry in fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_report_name)
            || !entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok();
        newest.push(Reverse((modified, name)));
        if newest.len() > KEEP_REPORTS
            && let Some(Reverse((_, oldest))) = newest.pop()
        {
            let _ = fs::remove_file(dir.join(oldest));
        }
    }
    Ok(())
}

fn bounded(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes)]
}

fn write_report(
    mut out: impl Write,
    thread: &str,
    message: &str,
    location: Option<(&str, u32, u32)>,
    module_base: Option<usize>,
    backtrace: impl std::fmt::Display,
) -> io::Result<()> {
    writeln!(out, "Skylis {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(
        out,
        "Target: {} ({})",
        env!("SKYLIS_BUILD_TARGET"),
        std::env::consts::OS
    )?;
    writeln!(out, "Thread: {}", bounded(thread, 256))?;
    writeln!(out, "Panic: {}", bounded(message, 4096))?;
    if let Some((file, line, column)) = location {
        writeln!(out, "Location: {}:{line}:{column}", bounded(file, 1024))?;
    } else {
        writeln!(out, "Location: <unknown>")?;
    }
    if let Some(base) = module_base {
        writeln!(out, "Main module base: {base:#x}")?;
    }
    writeln!(out, "Backtrace:\n{backtrace:#}\n")
}

fn module_base() -> Option<usize> {
    #[cfg(windows)]
    {
        // SAFETY: a null module name requests the current executable's module.
        let handle = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null())
        };
        (!handle.is_null()).then_some(handle as usize)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn save(info: &PanicHookInfo<'_>) -> io::Result<()> {
    static REPORT_LOCK: Mutex<()> = Mutex::new(());
    let _guard = match REPORT_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let dir = logs_dir();
    fs::create_dir_all(&dir)?;
    // Several panics in a single process and second share a report file.
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(report_name(Utc::now(), std::process::id())))?;
    let thread = std::thread::current();
    let result = write_report(
        file,
        thread.name().unwrap_or("<unnamed>"),
        info.payload_as_str().unwrap_or("<non-string panic>"),
        info.location()
            .map(|loc| (loc.file(), loc.line(), loc.column())),
        module_base(),
        Backtrace::force_capture(),
    );
    let _ = prune(&dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_name_contains_utc_time_and_pid() {
        let time = "2026-01-02T03:04:05Z".parse::<DateTime<Utc>>().unwrap();
        let name = report_name(time, 42);
        assert_eq!(name, "crash-20260102-030405-42.log");
        assert!(is_report_name(&name));
    }

    #[test]
    fn pruning_keeps_twenty_reports_and_preserves_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let start = "2026-01-02T03:04:00Z".parse::<DateTime<Utc>>().unwrap();
        for offset in 0..25 {
            let name = report_name(start + chrono::Duration::seconds(offset), 42);
            let file = fs::File::create(tmp.path().join(name)).unwrap();
            file.set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(60 + offset as u64),
            )
            .unwrap();
        }
        let unrelated = tmp.path().join("crash-20260102-test.log");
        fs::write(&unrelated, "keep").unwrap();
        prune(tmp.path()).unwrap();
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), KEEP_REPORTS + 1);
        assert!(unrelated.exists());
        for offset in 0..25 {
            let name = report_name(start + chrono::Duration::seconds(offset), 42);
            assert_eq!(tmp.path().join(name).exists(), offset >= 5);
        }
    }

    #[test]
    fn report_formats_diagnostics_and_bounds_utf8_input() {
        let mut report = Vec::new();
        let message = "雪".repeat(1400);
        write_report(
            &mut report,
            "worker",
            &message,
            Some(("src/main.rs", 12, 9)),
            Some(0x1234),
            "backtrace frame",
        )
        .unwrap();
        let report = String::from_utf8(report).unwrap();
        assert!(report.starts_with(concat!("Skylis ", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains(concat!("Target: ", env!("SKYLIS_BUILD_TARGET"))));
        assert!(report.contains("Thread: worker\n"));
        assert!(report.contains(&format!("Panic: {}\n", "雪".repeat(1365))));
        assert!(report.contains("Location: src/main.rs:12:9\n"));
        assert!(report.contains("Main module base: 0x1234\n"));
        assert!(report.contains("Backtrace:\nbacktrace frame\n"));
    }
}
