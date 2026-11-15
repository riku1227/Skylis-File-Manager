//! Small versioned preferences; no UI types or unbounded configuration parsing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: ThemePreference,
    pub compact: bool,
    pub group_downloads: bool,
    pub auto_gallery: bool,
    pub automatic_sizes: bool,
    pub high_refresh_rate: bool,
    pub smooth_scroll: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            compact: false,
            group_downloads: true,
            auto_gallery: true,
            automatic_sizes: false,
            high_refresh_rate: true,
            smooth_scroll: true,
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
        format!(
            "version=1\ntheme={theme}\ncompact={}\ngroup_downloads={}\nauto_gallery={}\nautomatic_sizes={}\nhigh_refresh_rate={}\nsmooth_scroll={}\n",
            self.compact,
            self.group_downloads,
            self.auto_gallery,
            self.automatic_sizes,
            self.high_refresh_rate,
            self.smooth_scroll
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
