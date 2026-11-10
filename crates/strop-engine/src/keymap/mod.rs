//! The keymap table (0008 §3 — keymaps as data). This table drives the
//! `Space ?` help buffer, the which-key prefix cards and the dispatch
//! trie: one source, grouped by section (0088 A7 — `bindings/` holds one
//! file per section, in the curated order `SECTIONS` fixes).
//!
//! Coverage contract (0003 §5.7): every dispatchable binding renders in
//! help; the tests pin both directions. `live: false` rows are planned
//! slots — reserved keys with no dispatch yet (0003 §6) — and render
//! muted with a "(soon)" suffix, never as live bindings.
//!
//! ## Notation
//!
//! `keys` is space-separated tokens, one key per token (`esc`, `:w`,
//! `"+y`; `<a>`/`<c>` parameterize a char):
//!
//! - `space` is the leader: its sequence runs to the row's end.
//! - `ctrl-w` takes exactly one following key.
//! - ` / ` separates alternatives that replace the last key of the
//!   sequence before them (`space d / k` → `space d`, `space k`);
//!   alternatives run to the row's end.
//! - Every other token is one single-key sequence; a row may list
//!   several (`h j k l`), and a leading bare `/` is the search-forward
//!   key (`/ ?`).

pub mod completion;

/// How a sequence dispatches (0008 stage 2). The walker consults these;
/// `?`/which-key read the same row — the table is the single source.
#[derive(Clone, Copy)]
pub enum Handler {
    /// A direct command.
    /// The key that completed the sequence rides along (J vs ., p vs P).
    Leaf(fn(&mut crate::editor::Editor, char)),
    /// Alias: expands to these keys through the walker (D → d$).
    Alias(&'static str),
    /// A grammar motion: the key completes a motion (or the pending
    /// operator's target) via strop_grammar.
    Motion,
    /// A grammar operator (d/y/c/>/<): the walker's typed op-pending.
    Operator,
    /// An object prefix (i/a) after an operator or in visual mode —
    /// bare i/a in normal mode is the insert entry (the walker knows
    /// from ParserState).
    ObjectPrefix,
    /// A prefix: children follow; which-key renders them.
    Prefix,
    /// Free-text line (`:` `/` `?` `|`…): a modal text field (0003 §1),
    /// NOT a key sequence — the text layer owns it.
    TextLine,
    /// One-char absorbers: r<c> replace, m<a> mark, '/` jump, f<c> find.
    AbsorbChar(AbsorbKind),
    /// `"x` — register selection.
    AbsorbRegister,
    /// Implemented by a modal field or Ex host, not the document trie.
    Contextual,
    /// Planned slot: no dispatch yet (renders muted "(soon)").
    Soon,
}

/// The one-char absorber flavors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsorbKind {
    Replace,
    MarkSet,
    MarkJump,
    Find,
    /// q<a> — toggle macro recording into register a.
    MacroRecord,
    /// @<a> — replay register a's macro.
    MacroPlay,
}

/// One binding as it appears in `Space ?` / which-key AND in dispatch
/// (0008 stage 2: one table). `id` is the stable command identity —
/// macros, dot-repeat, a future palette, and config rebinds ride it.
#[derive(Clone, Copy)]
pub struct Binding {
    pub keys: &'static str,
    pub desc: &'static str,
    /// Section ids (`SECTIONS`); the first is the row's home group.
    pub sections: &'static [&'static str],
    pub live: bool,
    pub id: &'static str,
    pub handler: Handler,
}

/// A help/which-key group: the stable id rows name and the title the
/// help buffer prints. Order here is the curated reading order.
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
}

pub const SECTIONS: &[Section] = &[
    Section {
        id: "moving",
        title: "Moving",
    },
    Section {
        id: "editing",
        title: "Editing",
    },
    Section {
        id: "insert",
        title: "Insert",
    },
    Section {
        id: "visual",
        title: "Visual",
    },
    Section {
        id: "search",
        title: "Search",
    },
    Section {
        id: "leader",
        title: "Leader (space)",
    },
    Section {
        id: "panes",
        title: "Panes",
    },
    Section {
        id: "git",
        title: "Git",
    },
    Section {
        id: "pickers",
        title: "Pickers",
    },
    Section {
        id: "terminal",
        title: "Terminal",
    },
    Section {
        id: "ex",
        title: "Ex commands",
    },
];

impl Section {
    /// Look a section up by id or title, case-insensitively (`:help git`).
    pub fn find(name: &str) -> Option<&'static Section> {
        let name = name.trim();
        SECTIONS.iter().find(|section| {
            section.id.eq_ignore_ascii_case(name) || section.title.eq_ignore_ascii_case(name)
        })
    }
}

mod bindings;
pub use bindings::BINDINGS;

pub(crate) mod lookup;

pub use lookup::{any_child, children_of, compat_report, find_row, Hint};

#[cfg(test)]
mod tests;
