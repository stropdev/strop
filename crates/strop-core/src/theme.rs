//! The strop-owned color seeds (0065 D3; built-in light 0.42.0). Every
//! surface — TUI render colors and the terminal's default palette —
//! derives from the CURRENT theme's values; nothing restates a hex
//! triple locally. `:theme`/`--theme` switch between the two compiled-in
//! palettes; config-file overrides remain 0005's follow-on.

use std::sync::atomic::{AtomicU8, Ordering};

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

/// A compiled-in palette. Field for field the same surface in two
/// lights: chroma and roles are identical, only their lightness is
/// tuned — dark values glow on the dark base, light values are inked
/// down for contrast on paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub base: Rgb,
    pub text: Rgb,
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
    pub class_keyword: Rgb,
    pub class_function: Rgb,
    pub class_type: Rgb,
    pub class_string: Rgb,
    pub class_number: Rgb,
    pub class_operator: Rgb,
    pub class_punctuation: Rgb,
    pub class_attribute: Rgb,
    /// The terminal's default 16-color set, harmonized with the fields
    /// above: red is the diagnostic error, blue the diagnostic info,
    /// magenta the keyword, cyan the type, green the string.
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

/// The strop default (plan 0004 site, --accent amber).
pub const DARK: Theme = Theme {
    base: rgb(0x16, 0x16, 0x1e),
    text: rgb(0xe8, 0xe4, 0xda),
    muted: rgb(0x6b, 0x6f, 0x7e),
    accent: rgb(0xf0, 0xa3, 0x5e),
    preview_bg: rgb(0x4a, 0x33, 0x1c),
    flash_bg: rgb(0x6b, 0x47, 0x22),
    select_bg: rgb(0x2a, 0x2c, 0x3a),
    pair_bg: rgb(0x3a, 0x3d, 0x4d),
    secondary: rgb(0x9b, 0xa0, 0xb1),
    diag_error: rgb(0xe8, 0x67, 0x7a),
    diag_info: rgb(0x7f, 0xb4, 0xca),
    class_keyword: rgb(0xc5, 0x8a, 0xe8),
    class_function: rgb(0x7f, 0xb4, 0xca),
    class_type: rgb(0x94, 0xd2, 0xbd),
    class_string: rgb(0xa9, 0xc4, 0x7c),
    class_number: rgb(0xe8, 0x97, 0x7a),
    class_operator: rgb(0x9a, 0xa0, 0xae),
    class_punctuation: rgb(0x56, 0x5b, 0x6e),
    class_attribute: rgb(0xd0, 0xa4, 0x5e),
    ansi16: [
        rgb(0x10, 0x10, 0x18), // black: one step below base
        rgb(0xe8, 0x67, 0x7a), // red: diagnostic error
        rgb(0xa9, 0xc4, 0x7c), // green: string
        rgb(0xe0, 0xaf, 0x68), // yellow: the held-back amber
        rgb(0x7f, 0xb4, 0xca), // blue: diagnostic info
        rgb(0xc5, 0x8a, 0xe8), // magenta: keyword
        rgb(0x94, 0xd2, 0xbd), // cyan: type
        rgb(0xcf, 0xcb, 0xc0), // white: a step under text
        rgb(0x56, 0x5b, 0x6e), // bright black: punctuation
        rgb(0xf2, 0x88, 0x96), // bright red
        rgb(0xbc, 0xd7, 0x95), // bright green
        rgb(0xf0, 0xa3, 0x5e), // bright yellow: the accent itself
        rgb(0x97, 0xc8, 0xdc), // bright blue
        rgb(0xd3, 0xa4, 0xee), // bright magenta
        rgb(0xab, 0xe2, 0xd0), // bright cyan
        rgb(0xe8, 0xe4, 0xda), // bright white: the text itself
    ],
};

/// The light reading surface: same hue family, inked down so every role
/// keeps its contrast on warm paper. The accent stays amber, keywords
/// stay violet, types stay teal — only their depth changes.
pub const LIGHT: Theme = Theme {
    base: rgb(0xf4, 0xf1, 0xea),
    text: rgb(0x2a, 0x2a, 0x33),
    muted: rgb(0x7a, 0x7d, 0x88),
    accent: rgb(0xb0, 0x60, 0x14),
    preview_bg: rgb(0xee, 0xdc, 0xbe),
    flash_bg: rgb(0xe6, 0xc6, 0x94),
    select_bg: rgb(0xd8, 0xda, 0xe4),
    pair_bg: rgb(0xc6, 0xc9, 0xd5),
    secondary: rgb(0x5d, 0x61, 0x70),
    diag_error: rgb(0xc0, 0x39, 0x4b),
    diag_info: rgb(0x2f, 0x6f, 0x8f),
    class_keyword: rgb(0x7c, 0x3f, 0xb0),
    class_function: rgb(0x2f, 0x6f, 0x8f),
    class_type: rgb(0x2e, 0x7d, 0x68),
    class_string: rgb(0x5a, 0x7d, 0x2a),
    class_number: rgb(0xb3, 0x50, 0x2e),
    class_operator: rgb(0x56, 0x5c, 0x68),
    class_punctuation: rgb(0xa4, 0xa8, 0xb4),
    class_attribute: rgb(0x96, 0x6a, 0x1e),
    ansi16: [
        rgb(0x1e, 0x1e, 0x26), // black: one step below the ink
        rgb(0xc0, 0x39, 0x4b), // red: diagnostic error
        rgb(0x5a, 0x7d, 0x2a), // green: string
        rgb(0x8a, 0x5a, 0x10), // yellow: the held-back amber
        rgb(0x2f, 0x6f, 0x8f), // blue: diagnostic info
        rgb(0x7c, 0x3f, 0xb0), // magenta: keyword
        rgb(0x2e, 0x7d, 0x68), // cyan: type
        rgb(0xe4, 0xe0, 0xd6), // white: a step under the paper
        rgb(0xa4, 0xa8, 0xb4), // bright black: punctuation
        rgb(0xd4, 0x55, 0x6a), // bright red
        rgb(0x6e, 0x93, 0x3e), // bright green
        rgb(0xb0, 0x60, 0x14), // bright yellow: the accent itself
        rgb(0x3f, 0x7f, 0x9f), // bright blue
        rgb(0x8e, 0x51, 0xc0), // bright magenta
        rgb(0x3e, 0x93, 0x7e), // bright cyan
        rgb(0x2a, 0x2a, 0x33), // bright white: the ink itself
    ],
};

/// The compiled-in palettes, selectable by name (`:theme`, `--theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeId {
    Dark,
    Light,
}

impl ThemeId {
    pub const ALL: [ThemeId; 2] = [Self::Dark, Self::Light];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|id| id.name().eq_ignore_ascii_case(name.trim()))
    }

    const fn theme(self) -> &'static Theme {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The palette every surface renders with right now.
pub fn current() -> &'static Theme {
    current_id().theme()
}

pub fn current_id() -> ThemeId {
    match CURRENT.load(Ordering::Relaxed) {
        1 => ThemeId::Light,
        _ => ThemeId::Dark,
    }
}

/// Switch the live palette. Render and the terminal palette read
/// `current()` per frame, so the next frame follows — no invalidation.
pub fn set_current(id: ThemeId) {
    CURRENT.store(id as u8, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi256_plants_the_seed_and_the_standard_ramps() {
        let colors = DARK.ansi256();
        assert_eq!(&colors[..16], &DARK.ansi16);
        // cube corners: black, pure red (5·36 into the cube), white
        assert_eq!(colors[16], rgb(0, 0, 0));
        assert_eq!(colors[196], rgb(255, 0, 0));
        assert_eq!(colors[231], rgb(255, 255, 255));
        // grayscale bounds
        assert_eq!(colors[232], rgb(8, 8, 8));
        assert_eq!(colors[255], rgb(238, 238, 238));
    }

    #[test]
    fn light_rethemes_every_role_without_losing_identity() {
        // The accent stays amber, the keyword violet, the type teal:
        // hue families survive, lightness does the retheming.
        const {
            assert!(LIGHT.accent.r > LIGHT.accent.b);
            assert!(LIGHT.class_keyword.b > LIGHT.class_keyword.g);
            assert!(LIGHT.class_type.g > LIGHT.class_type.r);
            // Paper and ink invert the dark pair's ordering.
            assert!(LIGHT.base.r > LIGHT.text.r);
            assert!(DARK.base.r < DARK.text.r);
        }
        // Every shared role actually moved — no silently copied field.
        assert_ne!(LIGHT.accent, DARK.accent);
        assert_ne!(LIGHT.select_bg, DARK.select_bg);
        assert_ne!(LIGHT.ansi16, DARK.ansi16);
    }

    #[test]
    fn theme_ids_round_trip_names() {
        for id in ThemeId::ALL {
            assert_eq!(ThemeId::from_name(id.name()), Some(id));
            assert_eq!(ThemeId::from_name(&id.name().to_uppercase()), Some(id));
        }
        assert_eq!(ThemeId::from_name(" solarized "), None);
        assert_eq!(ThemeId::from_name(""), None);
    }

    #[test]
    fn current_follows_the_switch_and_restores() {
        let before = current_id();
        set_current(ThemeId::Light);
        assert_eq!(current_id(), ThemeId::Light);
        assert_eq!(current(), &LIGHT);
        set_current(before);
        assert_eq!(current_id(), before);
        assert_eq!(current(), &DARK);
    }
}
