//! Small versioned preferences; no UI types or unbounded configuration parsing.
use std::path::{Path, PathBuf};

pub const MAX_PINS: usize = 32;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileTransferEngine {
    #[default]
    OsIntegrated,
    OsDialog,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityDisplay {
    #[default]
    StatusBar,
    Window,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub show_hidden: bool,
    pub restore_tabs: bool,
    pub theme: ThemePreference,
    pub compact: bool,
    pub group_downloads: bool,
    pub auto_gallery: bool,
    pub automatic_sizes: bool,
    pub high_refresh_rate: bool,
    pub smooth_scroll: bool,
    pub performance_overlay: bool,
    pub file_transfer_engine: FileTransferEngine,
    pub activity_display: ActivityDisplay,
    pub pinned_folders: Vec<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            restore_tabs: true,
            theme: ThemePreference::System,
            compact: false,
            group_downloads: true,
            auto_gallery: true,
            automatic_sizes: false,
            high_refresh_rate: true,
            smooth_scroll: true,
            performance_overlay: false,
            file_transfer_engine: FileTransferEngine::default(),
            activity_display: ActivityDisplay::default(),
            pinned_folders: Vec::new(),
        }
    }
}
impl Settings {
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > 16_384 {
            return Err("設定ファイルが大きすぎます".into());
        }
        let mut settings = Self::default();
        let mut version = false;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (key, value) = line.split_once('=').ok_or("設定ファイルの形式が不正です")?;
            let boolean = || match value {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err("設定値が不正です"),
            };
            match key {
                "show_hidden" => settings.show_hidden = boolean()?,
                "restore_tabs" => settings.restore_tabs = boolean()?,
                "version" if value == "1" => version = true,
                "version" => return Err("未対応の設定バージョンです".into()),
                "theme" => {
                    settings.theme = match value {
                        "light" => ThemePreference::Light,
                        "dark" => ThemePreference::Dark,
                        "system" => ThemePreference::System,
                        _ => return Err("テーマ設定が不正です".into()),
                    }
                }
                "compact" => settings.compact = boolean()?,
                "group_downloads" => settings.group_downloads = boolean()?,
                "auto_gallery" => settings.auto_gallery = boolean()?,
                "automatic_sizes" => settings.automatic_sizes = boolean()?,
                "high_refresh_rate" => settings.high_refresh_rate = boolean()?,
                "smooth_scroll" => settings.smooth_scroll = boolean()?,
                "performance_overlay" => settings.performance_overlay = boolean()?,
                "file_transfer_engine" => {
                    settings.file_transfer_engine = match value {
                        "os_integrated" => FileTransferEngine::OsIntegrated,
                        "os_dialog" => FileTransferEngine::OsDialog,
                        _ => return Err("ファイル転送エンジンの設定値が不正です".into()),
                    };
                }
                "activity_display" => {
                    settings.activity_display = match value {
                        "status_bar" => ActivityDisplay::StatusBar,
                        "window" => ActivityDisplay::Window,
                        _ => return Err("アクティビティ表示の設定値が不正です".into()),
                    };
                }
                "pin" => {
                    let path = decode_pin(value)?;
                    if !settings.pinned_folders.iter().any(|p| same_pin(p, &path)) {
                        if settings.pinned_folders.len() == MAX_PINS {
                            return Err("ピン留めは32件までです".into());
                        }
                        settings.pinned_folders.push(path);
                    }
                }
                _ => (),
            }
        }
        if !version {
            return Err("設定バージョンがありません".into());
        }
        Ok(settings)
    }
    pub fn encode(&self) -> String {
        let theme = match self.theme {
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
            ThemePreference::System => "system",
        };
        let mut text = format!(
            "version=1\ntheme={theme}\ncompact={}\ngroup_downloads={}\nauto_gallery={}\nautomatic_sizes={}\nhigh_refresh_rate={}\nsmooth_scroll={}\nperformance_overlay={}\n",
            self.compact,
            self.group_downloads,
            self.auto_gallery,
            self.automatic_sizes,
            self.high_refresh_rate,
            self.smooth_scroll,
            self.performance_overlay
        );
        text.push_str(&format!("restore_tabs={}\n", self.restore_tabs));
        text.push_str(&format!("show_hidden={}\n", self.show_hidden));
        text.push_str(match self.file_transfer_engine {
            FileTransferEngine::OsIntegrated => "file_transfer_engine=os_integrated\n",
            FileTransferEngine::OsDialog => "file_transfer_engine=os_dialog\n",
        });
        text.push_str(match self.activity_display {
            ActivityDisplay::StatusBar => "activity_display=status_bar\n",
            ActivityDisplay::Window => "activity_display=window\n",
        });
        for path in &self.pinned_folders {
            text.push_str("pin=");
            for byte in path.to_string_lossy().as_bytes() {
                use std::fmt::Write;
                let _ = write!(text, "{byte:02x}");
            }
            text.push('\n');
        }
        text
    }
    pub fn toggle_pin(&mut self, path: &Path) -> Result<(), String> {
        if let Some(index) = self.pinned_folders.iter().position(|p| same_pin(p, path)) {
            self.pinned_folders.remove(index);
            return Ok(());
        }
        if !path.is_absolute()
            || path
                .to_str()
                .is_none_or(|p| p.contains('\0') || p.len() > 4096)
        {
            return Err("このパスはピン留めできません".into());
        }
        if self.pinned_folders.len() >= MAX_PINS {
            return Err("ピン留めは32件までです".into());
        }
        self.pinned_folders.push(path.to_path_buf());
        if self.encode().len() > 16_384 {
            self.pinned_folders.pop();
            return Err("ピン留めするパスが長すぎます".into());
        }
        Ok(())
    }
}

pub fn same_pin(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn decode_pin(value: &str) -> Result<PathBuf, String> {
    if value.len() % 2 != 0 || value.len() > 8192 || !value.is_ascii() {
        return Err("ピン留め設定が不正です".into());
    }
    let bytes: Result<Vec<_>, _> = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16))
        .collect();
    let text = String::from_utf8(bytes.map_err(|_| "ピン留め設定が不正です")?)
        .map_err(|_| "ピン留め設定が不正です")?;
    let path = PathBuf::from(&text);
    if !path.is_absolute() || text.contains('\0') {
        return Err("ピン留めには絶対パスが必要です".into());
    }
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_operation_settings_migrate_round_trip_and_reject_invalid_values() {
        let defaults = Settings::decode("version=1\n").unwrap();
        assert_eq!(
            defaults.file_transfer_engine,
            FileTransferEngine::OsIntegrated
        );
        assert_eq!(defaults.activity_display, ActivityDisplay::StatusBar);
        let changed = Settings {
            file_transfer_engine: FileTransferEngine::OsDialog,
            activity_display: ActivityDisplay::Window,
            ..defaults
        };
        assert_eq!(Settings::decode(&changed.encode()).unwrap(), changed);
        assert!(Settings::decode("version=1\nfile_transfer_engine=builtin").is_err());
        assert!(Settings::decode("version=1\nactivity_display=other").is_err());
    }
    #[test]
    fn hidden_visibility_migrates_off_and_persists_independently_of_tab_restore() {
        let mut value = Settings::decode("version=1\nrestore_tabs=false\n").unwrap();
        assert!(!value.show_hidden);
        value.show_hidden = true;
        assert_eq!(Settings::decode(&value.encode()).unwrap(), value);
        assert!(Settings::decode("version=1\nshow_hidden=maybe\n").is_err());
    }
    #[test]
    fn tab_restore_migrates_defaults_and_can_be_disabled() {
        let mut value = Settings::decode("version=1\ncompact=true\n").unwrap();
        assert!(value.restore_tabs);
        value.restore_tabs = false;
        assert_eq!(Settings::decode(&value.encode()).unwrap(), value);
        assert!(Settings::decode("version=1\nrestore_tabs=maybe\n").is_err());
    }
    #[test]
    fn pins_round_trip_unicode_and_separators_without_injection() {
        let root = if cfg!(windows) {
            "C:\\資料=記録"
        } else {
            "/資料=記録"
        };
        let mut settings = Settings::default();
        settings.toggle_pin(Path::new(root)).unwrap();
        assert_eq!(Settings::decode(&settings.encode()).unwrap(), settings);
        settings.toggle_pin(Path::new(root)).unwrap();
        assert!(settings.pinned_folders.is_empty());
        assert!(settings.toggle_pin(Path::new("relative")).is_err());
        assert!(Settings::decode("version=1\npin=ff\n").is_err());
    }
    #[test]
    fn pins_limit_is_enforced_before_mutation() {
        let mut settings = Settings::default();
        for n in 0..MAX_PINS {
            let path = if cfg!(windows) {
                format!("C:\\folder{n}")
            } else {
                format!("/folder{n}")
            };
            settings.toggle_pin(Path::new(&path)).unwrap();
        }
        let before = settings.clone();
        let path = if cfg!(windows) {
            "C:\\overflow"
        } else {
            "/overflow"
        };
        assert!(settings.toggle_pin(Path::new(path)).is_err());
        assert_eq!(settings, before);
        assert_eq!(Settings::decode(&settings.encode()).unwrap(), settings);
    }
    #[test]
    fn performance_display_rejects_invalid_values() {
        assert!(Settings::decode("version=1\nperformance_overlay=sometimes\n").is_err());
    }
    #[test]
    fn performance_display_defaults_off_and_survives_round_trip() {
        let mut settings = Settings::decode("version=1\nhigh_refresh_rate=false\n").unwrap();
        assert!(!settings.performance_overlay);
        settings.performance_overlay = true;
        assert_eq!(Settings::decode(&settings.encode()).unwrap(), settings);
        assert!(!settings.high_refresh_rate);
    }
    #[test]
    fn motion_preferences_reject_invalid_values() {
        for key in ["high_refresh_rate", "smooth_scroll"] {
            assert!(Settings::decode(&format!("version=1\n{key}=sometimes\n")).is_err());
        }
    }
    #[test]
    fn motion_preferences_migrate_and_persist_independently() {
        let old = Settings::decode("version=1\ntheme=dark\ncompact=true").unwrap();
        assert!(old.high_refresh_rate && old.smooth_scroll);
        for (high_refresh_rate, smooth_scroll) in [(false, true), (true, false), (false, false)] {
            let changed = Settings {
                high_refresh_rate,
                smooth_scroll,
                ..old.clone()
            };
            assert_eq!(Settings::decode(&changed.encode()).unwrap(), changed);
        }
    }
    #[test]
    fn defaults_and_round_trip() {
        let initial = Settings::default();
        assert!(initial.group_downloads);
        assert!(!initial.automatic_sizes);
        let changed = Settings {
            theme: ThemePreference::Dark,
            compact: true,
            ..initial
        };
        assert_eq!(Settings::decode(&changed.encode()).unwrap(), changed);
    }
    #[test]
    fn corrupt_or_future_configuration_is_not_silently_overwritten() {
        assert!(Settings::decode("version=99\ntheme=light").is_err());
        assert!(Settings::decode("version=1\nautomatic_sizes=yes").is_err());
        assert!(Settings::decode(&"x".repeat(20_000)).is_err());
    }
    #[test]
    fn missing_fields_use_safe_defaults_and_unknown_fields_are_ignored() {
        assert_eq!(
            Settings::decode("version=1\nfuture_key=value").unwrap(),
            Settings::default()
        );
    }
}
