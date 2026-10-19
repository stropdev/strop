//! Compiled-in semantic palettes shared by every editor and terminal surface.
//! The registry owns canonical selection names and metadata; palette data is
//! static, and the live selector remains a single atomic value.

use std::sync::atomic::{AtomicU8, Ordering};

mod builtin;
mod catppuccin;
mod ink;
mod nord;
#[cfg(test)]
mod tests;

pub use builtin::{DARK, LIGHT};
pub use catppuccin::{CATPPUCCIN_LATTE, CATPPUCCIN_MOCHA};
pub use ink::STROP_INK;
pub use nord::NORD;

/// Full upstream notices, embedded in the executable without runtime assets.
pub const THEME_NOTICES: &str = concat!(
    "Catppuccin palettes — https://catppuccin.com/palette/\n\n",
    include_str!("../LICENSE-CATPPUCCIN"),
    "\nNord palette — https://www.nordtheme.com/docs/colors-and-palettes\n\n",
    include_str!("../LICENSE-NORD"),
);

/// An sRGB triple. Frontend and terminal layers convert to their own
/// color types; the seed itself stays representation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

const fn hex(value: u32) -> Rgb {
    rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Opaque sRGB wash, used only in compiled-in palette initializers.
const fn blend(base: Rgb, overlay: Rgb, percent: u8) -> Rgb {
    assert!(percent <= 100);
    const fn channel(base: u8, overlay: u8, percent: u8) -> u8 {
        ((base as u32 * (100 - percent as u32) + overlay as u32 * percent as u32) / 100) as u8
    }
    rgb(
        channel(base.r, overlay.r, percent),
        channel(base.g, overlay.g, percent),
        channel(base.b, overlay.b, percent),
    )
}

/// A complete semantic palette; syntax roles are language-independent.
/// Decorative borders/guides and child-selected ANSI colors are not text ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub base: Rgb,
    /// Elevated panels, pickers and help.
    pub surface: Rgb,
    pub statusline_bg: Rgb,
    pub cursor_row_bg: Rgb,
    pub inactive_bg: Rgb,
    pub border: Rgb,
    pub guide: Rgb,
    pub text: Rgb,
    /// Comment ink and subdued UI text, not decorative dimming.
    pub muted: Rgb,
    pub accent: Rgb,
    /// Accent, dimmed — the peek/preview backdrop.
    pub preview_bg: Rgb,
    /// Accent, stronger — the substitution flash.
    pub flash_bg: Rgb,
    pub select_bg: Rgb,
    /// Matching-delimiter overlay: quiet, one step above the selection.
    pub pair_bg: Rgb,
    /// Useful secondary context: between text and muted.
    pub secondary: Rgb,
    pub diag_error: Rgb,
    pub diag_info: Rgb,
    pub warning: Rgb,
    pub diff_add_bg: Rgb,
    pub diff_del_bg: Rgb,
    pub diff_add_strong_bg: Rgb,
    pub diff_del_strong_bg: Rgb,
    pub class_keyword: Rgb,
    pub class_function: Rgb,
    pub class_type: Rgb,
    pub class_string: Rgb,
    pub class_number: Rgb,
    pub class_operator: Rgb,
    pub class_punctuation: Rgb,
    pub class_attribute: Rgb,
    /// The terminal's default 16-color set. Terminal programs choose their
    /// own foreground/background pairings and may override these defaults.
    pub ansi16: [Rgb; 16],
}

impl Theme {
    /// The full 256-index default palette: the theme's ANSI-16, then
    /// the standard 6×6×6 color cube and grayscale ramp unchanged —
    /// programs addressing those indices expect the conventional ramp.
    pub const fn ansi256(&self) -> [Rgb; 256] {
        let mut colors = [rgb(0, 0, 0); 256];
        let mut index = 0;
        while index < 16 {
            colors[index] = self.ansi16[index];
            index += 1;
        }
        while index < 232 {
            let offset = index - 16;
            colors[index] = rgb(
                cube_level((offset / 36) as u8),
                cube_level(((offset % 36) / 6) as u8),
                cube_level((offset % 6) as u8),
            );
            index += 1;
        }
        while index < 256 {
            let level = 8 + 10 * (index - 232) as u8;
            colors[index] = rgb(level, level, level);
            index += 1;
        }
        colors
    }
}

/// One cube/grayscale component: the standard xterm ramp.
const fn cube_level(index: u8) -> u8 {
    if index == 0 {
        0
    } else {
        55 + 40 * index
    }
}

/// The compiled-in palettes, selectable by name (`:theme`, `--theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum ThemeId {
    Dark,
    Light,
    CatppuccinMocha,
    CatppuccinLatte,
    Nord,
    StropInk,
}

impl ThemeId {
    pub const ALL: [ThemeId; 6] = [
        Self::Dark,
        Self::Light,
        Self::CatppuccinMocha,
        Self::CatppuccinLatte,
        Self::Nord,
        Self::StropInk,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::CatppuccinLatte => "catppuccin-latte",
            Self::Nord => "nord",
            Self::StropInk => "strop-ink",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::Nord => "Nord",
            Self::StropInk => "Strop Ink",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Dark => "Strop's original warm amber on midnight",
            Self::Light => "Warm paper with amber, violet and teal ink",
            Self::CatppuccinMocha => "Pastel dark with a mauve accent",
            Self::CatppuccinLatte => "Pastel light with contrast-tuned semantic ink",
            Self::Nord => "Restrained cool dark with frost-blue accents",
            Self::StropInk => "Original graphite and ivory with brass and slate",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|id| id.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Pure palette access: does not read or change the live selection.
    pub const fn palette(self) -> &'static Theme {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
            Self::CatppuccinMocha => &CATPPUCCIN_MOCHA,
            Self::CatppuccinLatte => &CATPPUCCIN_LATTE,
            Self::Nord => &NORD,
            Self::StropInk => &STROP_INK,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The palette every surface renders with right now.
pub fn current() -> &'static Theme {
    current_id().palette()
}

pub fn current_id() -> ThemeId {
    match CURRENT.load(Ordering::Relaxed) {
        1 => ThemeId::Light,
        2 => ThemeId::CatppuccinMocha,
        3 => ThemeId::CatppuccinLatte,
        4 => ThemeId::Nord,
        5 => ThemeId::StropInk,
        _ => ThemeId::Dark,
    }
}

/// Select the editor palette. The editor separately queues updates to
/// existing terminal services; native terminal work never runs here.
pub fn set_current(id: ThemeId) {
    CURRENT.store(id as u8, Ordering::Relaxed);
}
