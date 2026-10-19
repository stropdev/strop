//! Nord's Polar Night / Snow Storm / Frost / Aurora semantic hierarchy.
//! Source: https://www.nordtheme.com/docs/colors-and-palettes
//!
//! Nord3 (#4c566a) remains a structural border/guide color, not comment ink.
//! Comments are Nord3 blended 45% toward Nord4 (#d8dee9), retaining the cool
//! hue while clearing 3:1 on both Nord0 and elevated Nord1. Selection stays
//! Nord2; preview/diff backgrounds are compile-time Strop-specific washes.
//! Other foregrounds and terminal ANSI colors use the upstream palette.

use super::{blend, hex, Theme};

pub const NORD: Theme = Theme {
    base: hex(0x2e3440),
    surface: hex(0x3b4252),
    statusline_bg: hex(0x3b4252),
    cursor_row_bg: blend(hex(0x2e3440), hex(0x434c5e), 50),
    inactive_bg: blend(hex(0x2e3440), hex(0x3b4252), 40),
    border: hex(0x4c566a),
    guide: hex(0x434c5e),
    text: hex(0xeceff4),
    muted: blend(hex(0x4c566a), hex(0xd8dee9), 45),
    accent: hex(0x88c0d0),
    preview_bg: blend(hex(0x2e3440), hex(0x88c0d0), 15),
    flash_bg: blend(hex(0x2e3440), hex(0x88c0d0), 25),
    select_bg: hex(0x434c5e),
    pair_bg: hex(0x4c566a),
    secondary: hex(0xd8dee9),
    diag_error: hex(0xbf616a),
    diag_info: hex(0x81a1c1),
    warning: hex(0xebcb8b),
    diff_add_bg: blend(hex(0x2e3440), hex(0xa3be8c), 12),
    diff_del_bg: blend(hex(0x2e3440), hex(0xbf616a), 12),
    diff_add_strong_bg: blend(hex(0x2e3440), hex(0xa3be8c), 24),
    diff_del_strong_bg: blend(hex(0x2e3440), hex(0xbf616a), 24),
    class_keyword: hex(0x81a1c1),
    class_function: hex(0x88c0d0),
    class_type: hex(0x8fbcbb),
    class_string: hex(0xa3be8c),
    class_number: hex(0xb48ead),
    class_operator: hex(0x81a1c1),
    class_punctuation: hex(0xd8dee9),
    class_attribute: hex(0xd08770),
    ansi16: [
        hex(0x3b4252),
        hex(0xbf616a),
        hex(0xa3be8c),
        hex(0xebcb8b),
        hex(0x81a1c1),
        hex(0xb48ead),
        hex(0x88c0d0),
        hex(0xe5e9f0),
        hex(0x4c566a),
        hex(0xbf616a),
        hex(0xa3be8c),
        hex(0xebcb8b),
        hex(0x81a1c1),
        hex(0xb48ead),
        hex(0x8fbcbb),
        hex(0xeceff4),
    ],
};
