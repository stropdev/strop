//! Compiled-in palettes as one token set (0088 §3). Every surface reads
//! colours through these tokens; nothing downstream names a hex value.
//! The selected palette lives on the editor — there is no process-global
//! selection — and frontends build their per-frame style table from
//! `ThemeId::palette`.

mod carbide;
mod hone;
mod paper;
mod slate;
#[cfg(test)]
mod tests;
mod vellum;

pub use carbide::CARBIDE;
pub use hone::HONE;
pub use paper::{PAPER, PAPER_DARK};
pub use slate::SLATE;
pub use vellum::VELLUM;

/// An sRGB triple. Frontend and terminal layers convert to their own
/// color types; the seed itself stays representation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

pub(crate) const fn hex(value: u32) -> Rgb {
    rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// The three planes and the overlays painted on them (0088 §2 rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backgrounds {
    /// The editor plane.
    pub base: Rgb,
    /// Cards, previews and the statusline.
    pub surface: Rgb,
    /// Selected rows and keycaps.
    pub raised: Rgb,
    pub cursorline: Rgb,
    pub selection: Rgb,
    /// The operator preview wash.
    pub preview: Rgb,
    /// The substitution flash.
    pub flash: Rgb,
    /// Matching-delimiter overlay.
    pub pair: Rgb,
    pub diff_add: Rgb,
    pub diff_del: Rgb,
    pub diff_add_strong: Rgb,
    pub diff_del_strong: Rgb,
}

/// The four greys plus the two non-text inks (0088 §2 rule 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Foregrounds {
    pub text: Rgb,
    pub secondary: Rgb,
    pub muted: Rgb,
    /// Borders, indent guides, inactive panes.
    pub faint: Rgb,
    pub guide: Rgb,
    /// Text painted over the accent (the mode chip).
    pub on_accent: Rgb,
}

/// Mode colours are their own tokens, never syntax slots (rule 2).
/// Normal is the accent itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    pub insert: Rgb,
    pub visual: Rgb,
    pub replace: Rgb,
    pub pending: Rgb,
    pub terminal: Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostics {
    pub error: Rgb,
    pub warning: Rgb,
    pub info: Rgb,
    pub hint: Rgb,
}

/// Foregrounds for `+`/`-` text and gutter signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diffs {
    pub add: Rgb,
    pub del: Rgb,
    pub change: Rgb,
}

/// Language-independent syntax roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syntax {
    pub keyword: Rgb,
    pub function: Rgb,
    pub r#type: Rgb,
    pub string: Rgb,
    pub number: Rgb,
    pub constant: Rgb,
    pub attribute: Rgb,
    pub operator: Rgb,
    pub punctuation: Rgb,
    /// Rendered italic (rule 7); never the same ink as punctuation
    /// unless the italic alone carries the distinction.
    pub comment: Rgb,
    pub variable: Rgb,
    pub property: Rgb,
    pub namespace: Rgb,
    pub heading: Rgb,
    pub link: Rgb,
    pub code: Rgb,
}

/// A complete palette: the 0087 §4.2 token set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Backgrounds,
    pub fg: Foregrounds,
    pub accent: Rgb,
    pub mode: Modes,
    pub diag: Diagnostics,
    pub diff: Diffs,
    pub syn: Syntax,
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeId {
    #[default]
    Hone,
    Slate,
    Carbide,
    Vellum,
    Paper,
    PaperDark,
}

impl ThemeId {
    pub const ALL: [ThemeId; 6] = [
        Self::Hone,
        Self::Slate,
        Self::Carbide,
        Self::Vellum,
        Self::Paper,
        Self::PaperDark,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Hone => "hone",
            Self::Slate => "slate",
            Self::Carbide => "carbide",
            Self::Vellum => "vellum",
            Self::Paper => "paper",
            Self::PaperDark => "paper-dark",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hone => "Hone",
            Self::Slate => "Slate",
            Self::Carbide => "Carbide",
            Self::Vellum => "Vellum",
            Self::Paper => "Paper",
            Self::PaperDark => "Paper Dark",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Hone => "Graphite dark with the amber accent; the default",
            Self::Slate => "Low-chroma dark: syntax by luminance, one brass accent",
            Self::Carbide => "High-contrast dark for projectors and bright rooms",
            Self::Vellum => "Warm paper with graphite ink and a burnt-umber accent",
            Self::Paper => "Warm off-white paper with deep green ink and forest accents",
            Self::PaperDark => "Warm green-black paper with soft ivory and sage accents",
        }
    }

    /// True for the palettes drawn on a light base.
    pub const fn is_light(self) -> bool {
        matches!(self, Self::Vellum | Self::Paper)
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|id| id.name().eq_ignore_ascii_case(name.trim()))
    }

    /// The six names, comma-separated, for refusals and warnings.
    pub fn choices() -> String {
        Self::ALL
            .iter()
            .map(|id| id.name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub const fn palette(self) -> &'static Theme {
        match self {
            Self::Hone => &HONE,
            Self::Slate => &SLATE,
            Self::Carbide => &CARBIDE,
            Self::Vellum => &VELLUM,
            Self::Paper => &PAPER,
            Self::PaperDark => &PAPER_DARK,
        }
    }
}
