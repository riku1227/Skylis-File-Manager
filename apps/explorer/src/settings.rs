use crate::{
    settings_data::{Settings, ThemePreference},
    theme::ThemeMode,
};
use gpui::{Context, Entity, Global, Task};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub struct Preferences {
    pub value: Settings,
    pub error: Option<String>,
    writable: bool,
    writer: async_channel::Sender<Settings>,
    pending: async_channel::Receiver<Settings>,
    _result: Task<()>,
    pub folder_integration: Option<crate::folder_integration::Status>,
    pub integration_busy: bool,
    pub integration_error: Option<String>,
    integration_sender: async_channel::Sender<Option<bool>>,
    _integration_result: Task<()>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_save_replaces_prior_settings() {
        let directory = std::env::temp_dir().join(format!(
            "skylis-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("settings.conf");
        let mut settings = Settings::default();
        save(&path, &settings).unwrap();
        settings.compact = true;
        save(&path, &settings).unwrap();
        assert_eq!(
            Settings::decode(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            settings
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
#[derive(Clone)]
pub struct SharedPreferences(pub Entity<Preferences>);
impl Global for SharedPreferences {}

impl From<ThemePreference> for ThemeMode {
    fn from(value: ThemePreference) -> Self {
        match value {
            ThemePreference::Light => Self::Light,
            ThemePreference::Dark => Self::Dark,
            ThemePreference::System => Self::System,
        }
    }
}
impl From<ThemeMode> for ThemePreference {
    fn from(value: ThemeMode) -> Self {
        match value {
            ThemeMode::Light => Self::Light,
            ThemeMode::Dark => Self::Dark,
            ThemeMode::System => Self::System,
        }
    }
}

fn settings_path() -> PathBuf {
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(explorer_core::platform::home_dir);
    #[cfg(target_os = "macos")]
    let root = explorer_core::platform::home_dir().join("Library/Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| explorer_core::platform::home_dir().join(".config"));
    root.join("Skylis").join("settings.conf")
}

pub fn load() -> (Settings, Option<String>) {
    let result = (|| {
        let file = match std::fs::File::open(settings_path()) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(e.to_string()),
        };
        let mut text = String::new();
        file.take(16_385)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        Settings::decode(&text)
    })();
    match result {
        Ok(v) => (v, None),
        Err(e) => (
            Settings::default(),
            Some(format!(
                "設定を読み込めなかったため既定値を使用中（元のファイルは保持）: {e}"
            )),
        ),
    }
}

fn save(path: &Path, value: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(value.encode().as_bytes())?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: owned NUL-terminated paths live through the call; the destination is our settings file.
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(temporary, path)?;
    Ok(())
}

impl Preferences {
    pub fn new(initial: (Settings, Option<String>), cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::bounded::<Settings>(1);
        let pending = rx.clone();
        let (result_tx, result_rx) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || {
                let path = settings_path();
                while let Ok(mut value) = rx.recv_blocking() {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    while let Ok(latest) = rx.try_recv() {
                        value = latest;
                    }
                    let error = save(&path, &value)
                        .err()
                        .map(|e| format!("設定を保存できません: {e}"));
                    if result_tx.send_blocking(error).is_err() {
                        break;
                    }
                }
            });
        let error = initial.1.or_else(|| spawned.err().map(|e| e.to_string()));
        let writable = error.is_none();
        let result = cx.spawn(async move |this, cx| {
            while let Ok(error) = result_rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.error = error;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let (integration_sender, integration_requests) = async_channel::bounded::<Option<bool>>(1);
        let (integration_results, integration_receiver) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("folder-integration".into())
            .spawn(move || {
                while let Ok(request) = integration_requests.recv_blocking() {
                    let result = match request {
                        Some(enabled) => crate::folder_integration::set_enabled(enabled),
                        None => crate::folder_integration::query(),
                    };
                    let result = match result {
                        Ok(status) => (Some(status), None),
                        Err(error) => (crate::folder_integration::query().ok(), Some(error)),
                    };
                    if integration_results.send_blocking(result).is_err() {
                        break;
                    }
                }
            });
        let integration_error = spawned.err().map(|e| e.to_string());
        let integration_busy = cfg!(windows) && integration_error.is_none();
        if integration_busy {
            let _ = integration_sender.try_send(None);
        }
        let integration_result = cx.spawn(async move |this, cx| {
            while let Ok(result) = integration_receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.integration_busy = false;
                        this.folder_integration = result.0;
                        this.integration_error = result.1;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        // Keep a receiver only to replace the pending snapshot without growing the queue.
        Self {
            value: initial.0,
            error,
            writable,
            writer: tx,
            pending,
            _result: result,
            folder_integration: None,
            integration_busy,
            integration_error,
            integration_sender,
            _integration_result: integration_result,
        }
    }
    pub fn set_folder_integration(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.integration_busy {
            return;
        }
        match self.integration_sender.try_send(Some(enabled)) {
            Ok(()) => {
                self.integration_busy = true;
                self.integration_error = None;
            }
            Err(error) => self.integration_error = Some(error.to_string()),
        }
        cx.notify();
    }
    pub fn change(&mut self, edit: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        edit(&mut self.value);
        if self.writable {
            if self.writer.is_full() {
                let _ = self.pending.try_recv();
            }
            if let Err(e) = self.writer.try_send(self.value.clone()) {
                self.error = Some(format!("設定を保存できません: {e}"));
            }
        }
        cx.notify();
    }
}
