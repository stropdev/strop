//! Vellum — light. Warm paper, graphite ink, burnt-umber accent; no green
//! cast, so it stays distinct from Paper (0087 §4.3). The surface is one
//! step lighter than the base and the raised plane one step deeper, so
//! the ladder measures 1.08 / 1.19 against the base; the selection is
//! `#d1c9b3` (1.48:1) and the comment `#696d77` to clear 4.5:1.

use super::{hex, Backgrounds, Diagnostics, Diffs, Foregrounds, Modes, Syntax, Theme};

pub const VELLUM: Theme = Theme {
    bg: Backgrounds {
        base: hex(0xf6f2e9),
        surface: hex(0xfdfbf6),
        raised: hex(0xe5dfd0),
        cursorline: hex(0xefeadf),
        selection: hex(0xd1c9b3),
        preview: hex(0xf0dfb8),
        flash: hex(0xe9c98a),
        pair: hex(0xd4cdbd),
        diff_add: hex(0xdfeadb),
        diff_del: hex(0xf3dcd8),
        diff_add_strong: hex(0xc3dbbb),
        diff_del_strong: hex(0xe8bfba),
    },
    fg: Foregrounds {
        text: hex(0x262a30),
        secondary: hex(0x4b525c),
        muted: hex(0x656d7b),
        faint: hex(0xaeb3ba),
        guide: hex(0xd8d3c8),
        on_accent: hex(0xf6f2e9),
    },
    accent: hex(0x9a5a12),
    mode: Modes {
        insert: hex(0x3f7a3a),
        visual: hex(0x6a3faa),
        replace: hex(0xb03a3a),
        pending: hex(0x9a5a12),
        terminal: hex(0x1f6f7a),
    },
    diag: Diagnostics {
        error: hex(0xb8323f),
        warning: hex(0x8a5d13),
        info: hex(0x2a6a94),
        hint: hex(0x656d7b),
    },
    diff: Diffs {
        add: hex(0x3f7a3a),
        del: hex(0xb8323f),
        change: hex(0x8a5d13),
    },
    syn: Syntax {
        keyword: hex(0x6f3aa8),
        function: hex(0x235f8c),
        r#type: hex(0x1f6e5c),
        string: hex(0x4e6f22),
        number: hex(0x9c4a22),
        constant: hex(0x9a5a12),
        attribute: hex(0x7a5a14),
        operator: hex(0x4b525c),
        punctuation: hex(0x656d7b),
        comment: hex(0x696d77),
        variable: hex(0x262a30),
        property: hex(0x3a3f47),
        namespace: hex(0x1f6e5c),
        heading: hex(0x9a5a12),
        link: hex(0x2a6a94),
        code: hex(0x4e6f22),
    },
    ansi16: [
        hex(0x262a30),
        hex(0xb8323f),
        hex(0x4e6f22),
        hex(0x7a5a14),
        hex(0x235f8c),
        hex(0x6f3aa8),
        hex(0x1f6e5c),
        hex(0xe5dfd0),
        hex(0x656d7b),
        hex(0xc9454f),
        hex(0x5f8530),
        hex(0x9a5a12),
        hex(0x2a6a94),
        hex(0x8a55c0),
        hex(0x2a8370),
        hex(0x262a30),
    ],
};
