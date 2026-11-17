//! Bounded session data. No filesystem or platform calls.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_WINDOWS: usize = 8;
pub const MAX_TABS: usize = 32;
pub const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedSort {
    #[default]
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedDisplay {
    pub gallery: Option<bool>,
    pub sort: SavedSort,
    pub ascending: bool,
    /// Preserve a manual sort that disabled Downloads date grouping.
    pub manual_sort: bool,
}
impl Default for SavedDisplay {
    fn default() -> Self {
        Self {
            gallery: None,
            sort: SavedSort::Name,
            ascending: true,
            manual_sort: false,
        }
    }
}
fn default_pane() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    #[serde(default = "default_pane")]
    pub pane: bool,
    /// Empty for legacy sessions; otherwise one item per saved tab.
    #[serde(default)]
    pub tab_display: Vec<SavedDisplay>,
    #[serde(default)]
    pub display: Option<String>,
    pub rect: [f32; 4],
    pub maximized: bool,
    /// None is Home. Archives restore their containing folder.
    pub tabs: Vec<Option<PathBuf>>,
    pub active: usize,
}
impl Default for SavedWindow {
    fn default() -> Self {
        Self {
            pane: true,
            tab_display: Vec::new(),
            display: None,
            rect: [80., 80., 1280., 800.],
            maximized: false,
            tabs: vec![None],
            active: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub windows: Vec<SavedWindow>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: 1,
            windows: Vec::new(),
        }
    }
}
impl Session {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.windows.len() > MAX_WINDOWS {
            return Err("未対応のセッション形式、またはウィンドウ数の上限超過".into());
        }
        for w in &self.windows {
            if !w.tab_display.is_empty() && w.tab_display.len() != w.tabs.len() {
                return Err("タブ表示設定の数が一致しません".into());
            }
            if w.display.as_ref().is_some_and(|s| s.len() > 128) {
                return Err("画面識別子が長すぎます".into());
            }
            if w.tabs.is_empty()
                || w.tabs.len() > MAX_TABS
                || w.active >= w.tabs.len()
                || !w.rect.iter().all(|v| v.is_finite())
                || w.rect[2] <= 0.
                || w.rect[3] <= 0.
            {
                return Err("セッションのウィンドウ情報が不正です".into());
            }
            for p in w.tabs.iter().flatten() {
                if !valid_path(p) {
                    return Err("セッションのパスが不正です".into());
                }
            }
        }
        Ok(())
    }
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > MAX_BYTES {
            return Err("セッションファイルが大きすぎます".into());
        }
        let value: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if text.len() > MAX_BYTES {
            return Err("セッションの保存上限を超えています".into());
        }
        Ok(text)
    }
    pub fn without_tabs(&mut self) {
        for w in &mut self.windows {
            w.tabs = vec![None];
            w.tab_display.clear();
            w.active = 0;
        }
    }
    pub fn startup(&self, restore: bool, explicit: bool) -> Vec<SavedWindow> {
        if restore && !explicit {
            return self.windows.clone();
        }
        self.windows
            .first()
            .map(|w| SavedWindow {
                tabs: vec![None],
                tab_display: Vec::new(),
                active: 0,
                ..w.clone()
            })
            .into_iter()
            .collect()
    }
}
pub fn valid_path(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| s.len() <= 4096 && !s.contains('\0'))
}

/// Use the nearest display, constrain dimensions and keep the whole titlebar reachable.
pub fn fit_rect(rect: [f32; 4], screens: &[[f32; 4]]) -> [f32; 4] {
    let rect = if rect.iter().all(|v| v.is_finite()) {
        rect
    } else {
        SavedWindow::default().rect
    };
    let fallback = [0., 0., 1920., 1080.];
    let screen = screens
        .iter()
        .filter(|s| s.iter().all(|v| v.is_finite()) && s[2] > 0. && s[3] > 0.)
        .max_by(|a, b| {
            let overlap = |s: &&[f32; 4]| {
                ((rect[0] + rect[2]).min(s[0] + s[2]) - rect[0].max(s[0])).max(0.)
                    * ((rect[1] + rect[3]).min(s[1] + s[3]) - rect[1].max(s[1])).max(0.)
            };
            let distance = |s: &&[f32; 4]| (rect[0] - s[0]).hypot(rect[1] - s[1]);
            overlap(a)
                .total_cmp(&overlap(b))
                .then_with(|| distance(b).total_cmp(&distance(a)))
        })
        .unwrap_or(&fallback);
    let w = rect[2].max(900.).min(screen[2]);
    let h = rect[3].max(600.).min(screen[3]);
    [
        rect[0].clamp(screen[0], screen[0] + screen[2] - w),
        rect[1].clamp(screen[1], screen[1] + screen[3] - h),
        w,
        h,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_session_defaults_and_display_state_survive_roundtrip() {
        let old=Session::decode(r#"{"version":1,"windows":[{"rect":[0,0,1000,700],"maximized":false,"tabs":[null],"active":0}]}"#).unwrap();
        assert!(old.windows[0].pane);
        assert!(old.windows[0].tab_display.is_empty());
        let mut value = old;
        value.windows[0].pane = false;
        value.windows[0].tab_display = vec![SavedDisplay {
            gallery: Some(true),
            sort: SavedSort::Size,
            ascending: false,
            manual_sort: true,
        }];
        assert_eq!(Session::decode(&value.encode().unwrap()).unwrap(), value);
        let off = value.startup(false, false);
        assert!(!off[0].pane);
        assert!(off[0].tab_display.is_empty());
        value.without_tabs();
        assert!(!value.windows[0].pane);
        assert!(value.windows[0].tab_display.is_empty());
    }

    fn window() -> SavedWindow {
        SavedWindow {
            pane: true,
            tab_display: Vec::new(),
            display: Some("test-display".into()),
            rect: [40., 50., 1100., 700.],
            maximized: true,
            tabs: vec![None, Some(std::env::current_dir().unwrap())],
            active: 1,
        }
    }

    #[test]
    fn round_trip_preserves_windows_tabs_order_and_active() {
        let value = Session {
            version: 1,
            windows: vec![window(), window()],
        };
        assert_eq!(Session::decode(&value.encode().unwrap()).unwrap(), value);
    }

    #[test]
    fn corrupt_future_oversized_and_unbounded_sessions_are_rejected() {
        assert!(Session::decode("{}").is_err());
        assert!(Session::decode(&"x".repeat(MAX_BYTES + 1)).is_err());
        let mut value = Session {
            version: 99,
            windows: vec![window()],
        };
        assert!(value.encode().is_err());
        value.version = 1;
        value.windows = vec![window(); MAX_WINDOWS + 1];
        assert!(value.encode().is_err());
        value.windows.truncate(1);
        value.windows[0].tabs = vec![None; MAX_TABS + 1];
        assert!(value.encode().is_err());
        value.windows[0].tabs = vec![Some("relative".into())];
        assert!(value.encode().is_err());
        value.windows[0].tabs = vec![None];
        value.windows[0].tab_display = vec![SavedDisplay::default(); 2];
        assert!(value.encode().is_err());
    }

    #[test]
    fn off_and_explicit_launch_keep_geometry_but_do_not_restore_tabs() {
        let value = Session {
            version: 1,
            windows: vec![window(), window()],
        };
        let off = value.startup(false, false);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].rect, window().rect);
        assert_eq!(off[0].tabs, vec![None]);
        assert_eq!(off[0].active, 0);
        assert_eq!(value.startup(true, true), off);
        assert_eq!(value.startup(true, false), value.windows);
    }

    #[test]
    fn geometry_is_finite_clamped_and_recovers_after_monitor_removal() {
        let screen = [0., 0., 1920., 1080.];
        assert_eq!(
            fit_rect([5000., -1000., 1100., 700.], &[screen]),
            [820., 0., 1100., 700.]
        );
        assert_eq!(
            fit_rect([0., 0., 9000., 7000.], &[[0., 0., 800., 500.]]),
            [0., 0., 800., 500.]
        );
        let fitted = fit_rect([f32::NAN, 0., 1., f32::INFINITY], &[screen]);
        assert!(fitted.iter().all(|v| v.is_finite()));
        assert!(fitted[2] >= 900. && fitted[3] >= 600.);
        assert_eq!(
            fit_rect(
                [1400., 100., 1100., 700.],
                &[[0., 0., 2560., 1440.], [2560., 0., 1920., 1080.]]
            ),
            [1400., 100., 1100., 700.]
        );
    }

    #[test]
    fn disabling_restore_removes_all_paths_and_resets_selection() {
        let mut value = Session {
            version: 1,
            windows: vec![window()],
        };
        value.without_tabs();
        assert_eq!(value.windows[0].tabs, vec![None]);
        assert_eq!(value.windows[0].active, 0);
    }
}
