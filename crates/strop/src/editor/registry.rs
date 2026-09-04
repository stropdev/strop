//! The normal-mode leaf table (0008 stage 1, 0014 wave 3): single-key
//! commands are data. Dispatch consults it after the structural layers
//! (counts, prefixes, arrows, pending); the `?` popup and which-key are
//! pinned to dispatch by keymap's coverage tests — a leaf without a
//! doc row, or a doc row whose key no-ops, fails the build.
//!
//! Multi-key sequences (operators, counts, registers, f<char>, /pat)
//! are the grammar's pending layer — unchanged; the trie absorbs them
//! in 0008 stage 2. Command IDs (macros, palette, config rebinds) land
//! on these rows then.

use super::Editor;

/// One normal-mode leaf command.
pub struct Leaf {
    pub key: char,
    /// Doc text lives in keymap::BINDINGS — the parity test fuses the
    /// two tables; the trie (0008 stage 2) absorbs both into one.
    pub run: fn(&mut Editor),
}

macro_rules! motion {
    ($key:literal) => {
        Leaf {
            key: $key,
            run: |e: &mut Editor| e.run_motion($key.encode_utf8(&mut [0; 4])),
        }
    };
}

/// Every single-key normal command. Order is presentation order.
pub static LEAVES: &[Leaf] = &[
    motion!('h'),
    motion!('j'),
    motion!('k'),
    motion!('l'),
    motion!('w'),
    motion!('b'),
    motion!('e'),
    motion!('W'),
    motion!('B'),
    motion!('E'),
    motion!('0'),
    motion!('^'),
    motion!('$'),
    motion!('|'),
    motion!('%'),
    motion!('G'),
    Leaf {
        key: 'n',
        run: |e| e.repeat_search(false),
    },
    Leaf {
        key: 'N',
        run: |e| e.repeat_search(true),
    },
    Leaf {
        key: '*',
        run: |e| e.search_word_under_cursor(false),
    },
    Leaf {
        key: '#',
        run: |e| e.search_word_under_cursor(true),
    },
    Leaf {
        key: ';',
        run: |e| e.repeat_find(false),
    },
    Leaf {
        key: ',',
        run: |e| e.repeat_find(true),
    },
    Leaf {
        key: 'x',
        run: Editor::delete_char,
    },
    Leaf {
        key: 'p',
        run: |e| e.paste_named(None, false),
    },
    Leaf {
        key: 'P',
        run: |e| e.paste_named(None, true),
    },
    Leaf {
        key: 'u',
        run: Editor::undo,
    },
    Leaf {
        key: '.',
        run: |e| e.dot_repeat_pub(),
    },
    Leaf {
        key: '~',
        run: |e| e.toggle_case_pub(),
    },
    Leaf {
        key: 'J',
        run: |e| e.join_lines_pub(),
    },
    Leaf {
        key: 'Q',
        run: |e| e.toggle_cursor(),
    },
    Leaf {
        key: 'i',
        run: |e| e.enter_insert_from("i"),
    },
    Leaf {
        key: 'a',
        run: Editor::append,
    },
    Leaf {
        key: 'A',
        run: Editor::append_eol,
    },
    Leaf {
        key: 'o',
        run: Editor::open_below,
    },
    Leaf {
        key: 'O',
        run: Editor::open_above,
    },
    Leaf {
        key: 'v',
        run: |e| e.enter_visual(false),
    },
    Leaf {
        key: 'V',
        run: |e| e.enter_visual(true),
    },
    Leaf {
        key: 'D',
        run: |e| e.alias("D", "d$"),
    },
    Leaf {
        key: 'C',
        run: |e| e.alias("C", "c$"),
    },
    Leaf {
        key: 'Y',
        run: |e| e.alias("Y", "yy"),
    },
    Leaf {
        key: 's',
        run: |e| e.alias("s", "cl"),
    },
    Leaf {
        key: 'X',
        run: |e| e.alias("X", "dh"),
    },
    Leaf {
        key: 'S',
        run: |e| e.alias("S", "cc"),
    },
    Leaf {
        key: 'I',
        run: |e| e.alias("I", "^i"),
    },
];

/// The leaf for a key, if one exists.
pub fn leaf_for(key: char) -> Option<&'static Leaf> {
    LEAVES.iter().find(|l| l.key == key)
}
