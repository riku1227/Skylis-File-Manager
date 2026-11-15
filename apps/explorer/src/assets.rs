use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// アイコンをバイナリに埋め込む（起動時のファイルI/Oなし、単一exeで配布できる）
macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "minus",
    "maximize",
    "restore",
    "search",
    "x",
    "plus",
    "sun",
    "settings",
    "view-settings",
    "linux",
    "panel",
    "folder",
    "file",
    "link",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "refresh",
    "chevron-right",
    "chevron-up",
    "chevron-down",
    "home",
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "drive",
    "eye",
    "activity",
    "folder-outline",
    "code",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}
