use gpui_kit::{Rgba, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn resolve(self, appearance: WindowAppearance) -> Scheme {
        match self {
            Self::Light => Scheme::Light,
            Self::Dark => Scheme::Dark,
            Self::System => match appearance {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Scheme::Dark,
                _ => Scheme::Light,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Light,
    Dark,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub scheme: Scheme,
    pub surface: Rgba,
    pub sidebar: Rgba,
    pub panel: Rgba,
    pub soft: Rgba,
    pub hover: Rgba,
    pub line: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub on_accent: Rgba,
    pub accent_soft: Rgba,
    pub hero: Rgba,
    pub green: Rgba,
    pub green_soft: Rgba,
    pub danger: Rgba,
    pub dial_edge: Rgba,
    pub dial_surface: Rgba,
    pub scrim: Rgba,
}

impl Palette {
    pub fn of(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Light => Self {
                scheme,
                surface: rgb(0xfafbfb),
                sidebar: rgb(0xeff1f2),
                panel: rgb(0xffffff),
                soft: rgb(0xf3f5f6),
                hover: rgb(0xe9edf0),
                line: rgb(0xd9dfe3),
                text: rgb(0x242b30),
                muted: rgb(0x677179),
                accent: rgb(0x365f87),
                accent_hover: rgb(0x2a4d70),
                on_accent: rgb(0xffffff),
                accent_soft: rgb(0xe6edf3),
                hero: rgb(0xffffff),
                green: rgb(0x24735c),
                green_soft: rgb(0xedf5f1),
                danger: rgb(0xae3944),
                dial_edge: rgb(0xbdc7cd),
                dial_surface: rgb(0xffffff),
                scrim: rgba(0x080c1040),
            },
            Scheme::Dark => Self {
                scheme,
                surface: rgb(0x1b1e21),
                sidebar: rgb(0x171a1d),
                panel: rgb(0x22262a),
                soft: rgb(0x202428),
                hover: rgb(0x2a3035),
                line: rgb(0x353d43),
                text: rgb(0xe2e7eb),
                muted: rgb(0xa0aab3),
                accent: rgb(0xa0bdd6),
                accent_hover: rgb(0xb9cede),
                on_accent: rgb(0x172735),
                accent_soft: rgb(0x293946),
                hero: rgb(0x1e2226),
                green: rgb(0x8ec5b0),
                green_soft: rgb(0x263b32),
                danger: rgb(0xe8999e),
                dial_edge: rgb(0x53616b),
                dial_surface: rgb(0x272d32),
                scrim: rgba(0x080c1099),
            },
        }
    }
}

#[cfg(target_os = "macos")]
pub const FONT: &str = ".SystemUIFont";
#[cfg(target_os = "windows")]
pub const FONT: &str = "Segoe UI Variable";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const FONT: &str = "sans-serif";

#[cfg(target_os = "macos")]
pub const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const MONO: &str = "monospace";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_mode_follows_the_window_appearance() {
        assert_eq!(
            ThemeMode::System.resolve(WindowAppearance::Dark),
            Scheme::Dark
        );
        assert_eq!(
            ThemeMode::System.resolve(WindowAppearance::Light),
            Scheme::Light
        );
        assert_eq!(
            ThemeMode::Light.resolve(WindowAppearance::Dark),
            Scheme::Light
        );
        assert_eq!(
            ThemeMode::Dark.resolve(WindowAppearance::Light),
            Scheme::Dark
        );
    }
}
