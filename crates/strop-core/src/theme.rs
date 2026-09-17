//! The one strop-owned color seed (0065 D3). Every surface — TUI render
//! constants and the terminal's default palette — derives from these
//! values; nothing restates a hex triple locally. Config-file overrides
//! are 0005's follow-on; until then this module is the single source.

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

// strop default palette (plan 0004 site, --accent amber)
pub const BASE: Rgb = rgb(0x16, 0x16, 0x1e);
pub const TEXT: Rgb = rgb(0xe8, 0xe4, 0xda);
pub const MUTED: Rgb = rgb(0x6b, 0x6f, 0x7e);
pub const ACCENT: Rgb = rgb(0xf0, 0xa3, 0x5e);
/// Accent, dimmed — the peek/preview backdrop.
pub const PREVIEW_BG: Rgb = rgb(0x4a, 0x33, 0x1c);
/// Accent, stronger — the substitution flash.
pub const FLASH_BG: Rgb = rgb(0x6b, 0x47, 0x22);
pub const SELECT_BG: Rgb = rgb(0x2a, 0x2c, 0x3a);
/// Matching-delimiter overlay: quiet slate one step above the selection.
pub const PAIR_BG: Rgb = rgb(0x3a, 0x3d, 0x4d);
/// Useful secondary context: between TEXT and MUTED.
pub const SECONDARY: Rgb = rgb(0x9b, 0xa0, 0xb1);

// Diagnostic severity colors (one source for gutter sign and EOL note).
pub const DIAG_ERROR: Rgb = rgb(0xe8, 0x67, 0x7a);
pub const DIAG_INFO: Rgb = rgb(0x7f, 0xb4, 0xca);

// Syntax-class colors beyond the base constants above.
pub const CLASS_KEYWORD: Rgb = rgb(0xc5, 0x8a, 0xe8);
pub const CLASS_FUNCTION: Rgb = DIAG_INFO;
pub const CLASS_TYPE: Rgb = rgb(0x94, 0xd2, 0xbd);
pub const CLASS_STRING: Rgb = rgb(0xa9, 0xc4, 0x7c);
pub const CLASS_NUMBER: Rgb = rgb(0xe8, 0x97, 0x7a);
pub const CLASS_OPERATOR: Rgb = rgb(0x9a, 0xa0, 0xae);
pub const CLASS_PUNCTUATION: Rgb = rgb(0x56, 0x5b, 0x6e);
pub const CLASS_ATTRIBUTE: Rgb = rgb(0xd0, 0xa4, 0x5e);

/// The terminal's default 16-color set, harmonized with the palette
/// above: red is the diagnostic error, yellow the held-back amber (its
/// bright is the accent), blue the diagnostic info, magenta the keyword
/// violet, cyan the type teal, white a step under TEXT. `ls`, `git` and
/// prompt themes all draw from these indices, so they must speak the
/// same language as the chrome around them.
pub const ANSI16: [Rgb; 16] = [
    rgb(0x10, 0x10, 0x18), // black: one step below BASE
    DIAG_ERROR,            // red
    CLASS_STRING,          // green
    rgb(0xe0, 0xaf, 0x68), // yellow: the held-back amber
    DIAG_INFO,             // blue
    CLASS_KEYWORD,         // magenta
    CLASS_TYPE,            // cyan
    rgb(0xcf, 0xcb, 0xc0), // white: a step under TEXT
    CLASS_PUNCTUATION,     // bright black
    rgb(0xf2, 0x88, 0x96), // bright red
    rgb(0xbc, 0xd7, 0x95), // bright green
    ACCENT,                // bright yellow: the accent itself
    rgb(0x97, 0xc8, 0xdc), // bright blue
    rgb(0xd3, 0xa4, 0xee), // bright magenta
    rgb(0xab, 0xe2, 0xd0), // bright cyan
    TEXT,                  // bright white
];

/// One cube/grayscale component: the standard xterm ramp.
const fn cube_level(index: u8) -> u8 {
    if index == 0 {
        0
    } else {
        55 + 40 * index
    }
}

/// The full 256-index default palette: ANSI16 harmonized by the seed,
/// then the standard 6×6×6 color cube and grayscale ramp unchanged —
/// programs addressing those indices expect the conventional ramp.
pub const fn ansi256() -> [Rgb; 256] {
    let mut colors = [rgb(0, 0, 0); 256];
    let mut index = 0;
    while index < 16 {
        colors[index] = ANSI16[index];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi256_plants_the_seed_and_the_standard_ramps() {
        let colors = ansi256();
        assert_eq!(&colors[..16], &ANSI16);
        // cube corners: black, pure red (5·36 into the cube), white
        assert_eq!(colors[16], rgb(0, 0, 0));
        assert_eq!(colors[196], rgb(255, 0, 0));
        assert_eq!(colors[231], rgb(255, 255, 255));
        // grayscale bounds
        assert_eq!(colors[232], rgb(8, 8, 8));
        assert_eq!(colors[255], rgb(238, 238, 238));
    }
}
