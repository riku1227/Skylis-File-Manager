use gpui::{Rgba, WindowAppearance};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}
impl ThemeMode {
    pub fn next(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
            Self::System => Self::Light,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "ライト",
            Self::Dark => "ダーク",
            Self::System => "システム",
        }
    }
}

/// Values generated from the approved semantic design tokens.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
}
impl Theme {
    pub fn light() -> Self {
        Self { dark: false }
    }
    pub fn resolve(mode: ThemeMode, appearance: WindowAppearance) -> Self {
        Self {
            dark: match mode {
                ThemeMode::Light => false,
                ThemeMode::Dark => true,
                ThemeMode::System => matches!(
                    appearance,
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                ),
            },
        }
    }
    pub fn c(self, name: &str) -> Rgba {
        crate::theme_tokens::color(name, self.dark)
    }
}
