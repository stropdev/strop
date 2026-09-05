//! The keymap table (0008 §3 — keymaps as data). v1: this table drives
//! the `Space ?` keybinds popup and the which-key prefix cards; the trie
//! dispatch cutover (0008 §5) makes it the dispatch source later.
//!
//! Coverage contract (0003 §5.7): every dispatchable binding renders in
//! the popup; the tests below pin both directions. `live: false` rows
//! are planned slots — reserved keys with no dispatch yet (0003 §6) —
//! and render muted with a "(soon)" suffix, never as live bindings.
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
pub struct Binding {
    pub keys: &'static str,
    pub desc: &'static str,
    /// Sidebar section: normal · visual · insert · leader · git · ex+panes
    pub section: &'static str,
    /// false = planned slot: no dispatch yet, renders muted "(soon)".
    pub live: bool,
    /// Stable identity per row; per-sequence ids derive as `{id}:{key}`.
    pub id: &'static str,
    pub handler: Handler,
}

pub const SECTIONS: &[&str] = &["normal", "visual", "insert", "leader", "git", "ex+panes"];

pub const BINDINGS: &[Binding] = &[
    // normal: motions
    Binding {
        keys: "h j k l",
        desc: "move (never off the line)",
        section: "normal",
        live: true,
        id: "move",
        handler: Handler::Motion,
    },
    Binding {
        keys: "w b e W B E",
        desc: "word / WORD motions",
        section: "normal",
        live: true,
        id: "word-motions",
        handler: Handler::Motion,
    },
    Binding {
        keys: "zz zt zb",
        desc: "center/top/bottom the cursor line",
        section: "normal",
        live: true,
        id: "view-place",
        handler: Handler::Leaf(|e, k| e.view_place(k)),
    },
    Binding {
        keys: "H M L",
        desc: "top/middle/bottom visible line",
        section: "normal",
        live: true,
        id: "visible-jumps",
        handler: Handler::Leaf(|e, k| e.jump_visible(k, 1)),
    },
    Binding {
        keys: "ZZ",
        desc: "write + close window",
        section: "normal",
        live: true,
        id: "write-quit",
        handler: Handler::Leaf(|e, _| e.write_quit()),
    },
    Binding {
        keys: "gv",
        desc: "reselect last visual",
        section: "normal",
        live: true,
        id: "reselect-visual",
        handler: Handler::Leaf(|e, _| e.reselect_visual()),
    },
    Binding {
        keys: "gi",
        desc: "insert at last insert",
        section: "normal",
        live: true,
        id: "insert-at-last",
        handler: Handler::Leaf(|e, _| e.insert_at_last()),
    },
    Binding {
        keys: "g;",
        desc: "older change (changelist)",
        section: "normal",
        live: true,
        id: "change-back",
        handler: Handler::Leaf(|e, _| e.change_jump(true)),
    },
    Binding {
        keys: "g,",
        desc: "newer change (changelist)",
        section: "normal",
        live: true,
        id: "change-forward",
        handler: Handler::Leaf(|e, _| e.change_jump(false)),
    },
    Binding {
        keys: "ge gE { }",
        desc: "word-end back / paragraph motions",
        section: "normal",
        live: true,
        id: "paragraph-motions",
        handler: Handler::Motion,
    },
    Binding {
        keys: "0 $ G %",
        desc: "line/file/pair jumps",
        section: "normal",
        live: true,
        id: "line-jumps",
        handler: Handler::Motion,
    },
    Binding {
        keys: "gg",
        desc: "top of file",
        section: "normal",
        live: true,
        id: "top",
        handler: Handler::Motion,
    },
    Binding {
        keys: "enter",
        desc: "line down, first non-blank (blame gutter: dive)",
        section: "normal",
        live: true,
        id: "enter",
        handler: Handler::Leaf(|e, _| e.enter_pub()),
    },
    Binding {
        keys: "tab",
        desc: "jump list forward (ctrl-i)",
        section: "normal",
        live: true,
        id: "jump-forward",
        handler: Handler::Leaf(|e, _| e.jump_forward()),
    },
    Binding {
        keys: "ctrl-r",
        desc: "redo",
        section: "normal",
        live: true,
        id: "redo",
        handler: Handler::Leaf(|e, _| e.redo()),
    },
    Binding {
        keys: "ctrl-d ctrl-u ctrl-f ctrl-b",
        desc: "half/full page scroll (count = lines)",
        section: "normal",
        live: true,
        id: "scroll-pages",
        handler: Handler::Leaf(|_, _| {}), // count-aware: dispatch_row
    },
    Binding {
        keys: "ctrl-^",
        desc: "alternate buffer",
        section: "normal",
        live: true,
        id: "alternate-buffer",
        handler: Handler::Leaf(|e, _| e.alternate_buffer()),
    },
    Binding {
        keys: "q<a>",
        desc: "record macro into register",
        section: "normal",
        live: true,
        id: "macro-record",
        handler: Handler::AbsorbChar(AbsorbKind::MacroRecord),
    },
    Binding {
        keys: "@<a>",
        desc: "play macro (count repeats)",
        section: "normal",
        live: true,
        id: "macro-play",
        handler: Handler::AbsorbChar(AbsorbKind::MacroPlay),
    },
    Binding {
        keys: "gr",
        desc: "references (LSP)",
        section: "normal",
        live: true,
        id: "references",
        handler: Handler::Leaf(|e, _| e.lsp_locations_pub(strop_lsp::LocKind::References)),
    },
    Binding {
        keys: "gI",
        desc: "implementation (LSP)",
        section: "normal",
        live: true,
        id: "implementation",
        handler: Handler::Leaf(|e, _| e.lsp_locations_pub(strop_lsp::LocKind::Implementation)),
    },
    Binding {
        keys: "gy",
        desc: "type definition (LSP)",
        section: "normal",
        live: true,
        id: "type-definition",
        handler: Handler::Leaf(|e, _| e.lsp_locations_pub(strop_lsp::LocKind::TypeDefinition)),
    },
    Binding {
        keys: "gD",
        desc: "declaration (LSP)",
        section: "normal",
        live: true,
        id: "declaration",
        handler: Handler::Leaf(|e, _| e.lsp_locations_pub(strop_lsp::LocKind::Declaration)),
    },
    Binding {
        keys: "]d [d",
        desc: "next/prev diagnostic",
        section: "normal",
        live: true,
        id: "diagnostic-jumps",
        handler: Handler::Leaf(|e, k| e.jump_diagnostic_pub(k != '[')),
    },
    Binding {
        keys: "gd",
        desc: "goto definition (LSP)",
        section: "normal",
        live: true,
        id: "goto-definition",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::lsp_goto_definition_pub(e)),
    },
    Binding {
        keys: "gs",
        desc: "switch source/header (clangd)",
        section: "normal",
        live: true,
        id: "switch-source-header",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::lsp_switch_source_header_pub(e)),
    },
    Binding {
        keys: "f<c> F<c> t<c> T<c>",
        desc: "find/till char (candidates light up)",
        section: "normal",
        live: true,
        id: "find-char",
        handler: Handler::AbsorbChar(AbsorbKind::Find),
    },
    Binding {
        keys: ":",
        desc: "ex command line",
        section: "normal",
        live: true,
        id: "ex-line",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "/ ?",
        desc: "search forward / backward",
        section: "normal",
        live: true,
        id: "search",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "n",
        desc: "next match",
        section: "normal",
        live: true,
        id: "search-next",
        handler: Handler::Leaf(|e, _| e.repeat_search_pub(false)),
    },
    Binding {
        keys: "N",
        desc: "previous match",
        section: "normal",
        live: true,
        id: "search-prev",
        handler: Handler::Leaf(|e, _| e.repeat_search_pub(true)),
    },
    Binding {
        keys: "]c [c",
        desc: "next / prev git hunk",
        section: "normal",
        live: true,
        id: "hunk-nav",
        handler: Handler::Leaf(|e, k| e.jump_hunk_pub(k != '[')),
    },
    Binding {
        keys: "m<a>",
        desc: "set mark at cursor",
        section: "normal",
        live: true,
        id: "mark-set",
        handler: Handler::AbsorbChar(AbsorbKind::MarkSet),
    },
    Binding {
        keys: "'<a> `<a>",
        desc: "jump to mark",
        section: "normal",
        live: true,
        id: "mark-jump",
        handler: Handler::AbsorbChar(AbsorbKind::MarkJump),
    },
    Binding {
        keys: "* #",
        desc: "word under cursor, forward / backward",
        section: "normal",
        live: true,
        id: "word-search",
        handler: Handler::Leaf(|e, k| e.search_word_under_cursor_pub(k == '#')),
    },
    Binding {
        keys: "; ,",
        desc: "repeat find, same / reversed",
        section: "normal",
        live: true,
        id: "find-repeat",
        handler: Handler::Leaf(|e, k| e.repeat_find_pub(k == ',')),
    },
    Binding {
        keys: "|",
        desc: "column motion (vim); pipe moved to space |",
        section: "normal",
        live: true,
        id: "column-motion",
        handler: Handler::Motion,
    },
    Binding {
        keys: "space |",
        desc: "pipe line/selection through shell (:! runs)",
        section: "leader",
        live: true,
        id: "pipe-shell",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "Q",
        desc: "toggle cursor at point (multicursor)",
        section: "normal",
        live: true,
        id: "cursor-toggle",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::toggle_cursor(e)),
    },
    // normal: operators
    Binding {
        keys: "d y c > <",
        desc: "operators + motion/object (live preview)",
        section: "normal",
        live: true,
        id: "operators",
        handler: Handler::Operator,
    },
    Binding {
        keys: "dd yy cc",
        desc: "line delete/yank/change",
        section: "normal",
        live: true,
        id: "line-ops",
        handler: Handler::Operator,
    },
    Binding {
        keys: "D",
        desc: "delete to line end",
        section: "normal",
        live: true,
        id: "op-alias-d",
        handler: Handler::Alias("d$"),
    },
    Binding {
        keys: "C",
        desc: "change to line end",
        section: "normal",
        live: true,
        id: "op-alias-c",
        handler: Handler::Alias("c$"),
    },
    Binding {
        keys: "Y",
        desc: "yank line",
        section: "normal",
        live: true,
        id: "op-alias-y",
        handler: Handler::Alias("yy"),
    },
    Binding {
        keys: "s",
        desc: "substitute char",
        section: "normal",
        live: true,
        id: "op-alias-s",
        handler: Handler::Alias("cl"),
    },
    Binding {
        keys: "x X",
        desc: "delete char / char back",
        section: "normal",
        live: true,
        id: "char-delete",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::delete_char(e)),
    },
    Binding {
        keys: "iw i\" i' i( i[ i{",
        desc: "inner objects (quotes scan the line)",
        section: "normal",
        live: true,
        id: "objects",
        handler: Handler::ObjectPrefix,
    },
    Binding {
        keys: "ds\" cs\"' ysiw\"",
        desc: "surround: delete / change / add",
        section: "normal",
        live: true,
        id: "surround",
        handler: Handler::ObjectPrefix,
    },
    Binding {
        keys: "i a A o O I",
        desc: "insert (auto-indent)",
        section: "normal",
        live: true,
        id: "insert-entries",
        handler: Handler::Leaf(crate::editor::Editor::insert_entry_pub),
    },
    Binding {
        keys: "p P",
        desc: "paste after / before",
        section: "normal",
        live: true,
        id: "paste",
        handler: Handler::Leaf(|e, k| e.paste_named_pub(None, k == 'P')),
    },
    Binding {
        keys: "r<c>",
        desc: "replace char",
        section: "normal",
        live: true,
        id: "replace-char",
        handler: Handler::AbsorbChar(AbsorbKind::Replace),
    },
    Binding {
        keys: "J .",
        desc: "join lines · repeat change",
        section: "normal",
        live: true,
        id: "join-repeat",
        handler: Handler::Leaf(crate::editor::Editor::join_or_repeat),
    },
    Binding {
        keys: "^",
        desc: "first non-blank",
        section: "normal",
        live: true,
        id: "first-non-blank",
        handler: Handler::Motion,
    },
    Binding {
        keys: "~",
        desc: "toggle case",
        section: "normal",
        live: true,
        id: "toggle-case",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::toggle_case_pub(e)),
    },
    Binding {
        keys: "S",
        desc: "change line",
        section: "normal",
        live: true,
        id: "subst-line",
        handler: Handler::Alias("cc"),
    },
    Binding {
        keys: "u ctrl-r",
        desc: "undo / redo (one unit per command)",
        section: "normal",
        live: true,
        id: "undo-redo",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::undo(e)),
    },
    Binding {
        keys: "\"+y \"+p \"+P",
        desc: "system clipboard: yank / paste after / before",
        section: "normal",
        live: true,
        id: "reg-clipboard",
        handler: Handler::AbsorbRegister,
    },
    Binding {
        keys: "\"xy \"xp",
        desc: "named register: yank / paste",
        section: "normal",
        live: true,
        id: "reg-named",
        handler: Handler::AbsorbRegister,
    },
    Binding {
        keys: "ctrl-v",
        desc: "visual block",
        section: "normal",
        live: true,
        id: "visual-block",
        handler: Handler::Leaf(|e, _| e.enter_block_pub()),
    },
    Binding {
        keys: "v V",
        desc: "visual / visual-line",
        section: "normal",
        live: true,
        id: "visual-enter",
        handler: Handler::Leaf(|e, k| e.enter_visual_pub(k)),
    },
    // visual
    Binding {
        keys: "d y c x > <",
        desc: "operate on selection",
        section: "visual",
        live: true,
        id: "visual-ops",
        handler: Handler::Operator,
    },
    Binding {
        keys: "S<c>",
        desc: "wrap selection in pair",
        section: "visual",
        live: true,
        id: "visual-surround",
        handler: Handler::AbsorbChar(AbsorbKind::Replace),
    },
    Binding {
        keys: "i<a> a<a>",
        desc: "objects select (vi[ works)",
        section: "visual",
        live: true,
        id: "visual-objects",
        handler: Handler::ObjectPrefix,
    },
    Binding {
        keys: "space y",
        desc: "yank selection → clipboard",
        section: "visual",
        live: true,
        id: "clip-yank",
        handler: Handler::Leaf(|e, _| e.clipboard_yank_pub()),
    },
    // insert
    Binding {
        keys: "esc",
        desc: "normal mode (session = one undo unit)",
        section: "insert",
        live: true,
        id: "insert-esc",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "backspace",
        desc: "delete back",
        section: "insert",
        live: true,
        id: "insert-bs",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "enter",
        desc: "new line (auto-indent)",
        section: "insert",
        live: true,
        id: "insert-enter",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "} ] )",
        desc: "closer on indent-only line dedents",
        section: "insert",
        live: true,
        id: "insert-closers",
        handler: Handler::Prefix,
    },
    // leader
    Binding {
        keys: "space f",
        desc: "file finder",
        section: "leader",
        live: true,
        id: "files",
        handler: Handler::Leaf(|e, _| e.open_picker(strop_picker::Kind::Files)),
    },
    Binding {
        keys: "space b",
        desc: "buffers (MRU)",
        section: "leader",
        live: true,
        id: "buffers",
        handler: Handler::Leaf(|e, _| e.open_picker(strop_picker::Kind::Buffers)),
    },
    Binding {
        keys: "space /",
        desc: "live grep",
        section: "leader",
        live: true,
        id: "grep",
        handler: Handler::Leaf(|e, _| e.open_picker(strop_picker::Kind::Grep)),
    },
    Binding {
        keys: "space R",
        desc: "global search & replace",
        section: "leader",
        live: true,
        id: "replace-global",
        handler: Handler::Leaf(|e, _| e.open_picker(strop_picker::Kind::Replace)),
    },
    Binding {
        keys: "space ?",
        desc: "this popup",
        section: "leader",
        live: true,
        id: "help",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::open_help(e)),
    },
    Binding {
        keys: "space y",
        desc: "yank motion → system clipboard",
        section: "leader",
        live: true,
        id: "clip-yank",
        handler: Handler::Leaf(|e, _| e.clipboard_yank_pub()),
    },
    Binding {
        keys: "space p",
        desc: "paste clipboard after",
        section: "leader",
        live: true,
        id: "clip-paste",
        handler: Handler::Leaf(|e, k| e.clipboard_paste_pub(k == 'P')),
    },
    Binding {
        keys: "space P",
        desc: "paste clipboard before",
        section: "leader",
        live: true,
        id: "clip-paste-before",
        handler: Handler::Leaf(|e, k| e.clipboard_paste_pub(k == 'P')),
    },
    Binding {
        keys: "space d",
        desc: "diagnostics picker",
        section: "leader",
        live: true,
        id: "diagnostics",
        handler: Handler::Leaf(|e, _| e.open_diagnostics_picker()),
    },
    Binding {
        keys: "space k",
        desc: "hover docs",
        section: "leader",
        live: true,
        id: "hover",
        handler: Handler::Leaf(|e, _| e.lsp_hover_pub()),
    },
    Binding {
        keys: "space j",
        desc: "jumplist picker",
        section: "leader",
        live: false,
        id: "jumplist-picker",
        handler: Handler::Soon,
    },
    Binding {
        keys: "space u",
        desc: "undo-tree browser",
        section: "leader",
        live: true,
        id: "undo-tree",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::open_undo_tree(e)),
    },
    Binding {
        keys: "space c",
        desc: "cursor on next line too (multicursor)",
        section: "leader",
        live: true,
        id: "cursor-stack",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::add_cursor_next_line(e)),
    },
    // git
    Binding {
        keys: "space g",
        desc: "git…",
        section: "git",
        live: true,
        id: "git-prefix",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "space g l",
        desc: "commit browser",
        section: "git",
        live: true,
        id: "git-log",
        handler: Handler::Leaf(|e, _| e.open_log_pub(false)),
    },
    Binding {
        keys: "space g h",
        desc: "file history (visual: selected lines)",
        section: "git",
        live: true,
        id: "git-file-history",
        handler: Handler::Leaf(|e, _| e.open_log_pub(true)),
    },
    Binding {
        keys: "space g b",
        desc: "toggle blame gutter / card",
        section: "git",
        live: true,
        id: "git-blame",
        handler: Handler::Leaf(|e, _| e.toggle_blame_gutter()),
    },
    Binding {
        keys: "space g y",
        desc: "permalink: copy",
        section: "git",
        live: true,
        id: "git-permalink-yank",
        handler: Handler::Leaf(|e, _| e.yank_permalink()),
    },
    Binding {
        keys: "space g o",
        desc: "permalink: open",
        section: "git",
        live: true,
        id: "git-permalink-open",
        handler: Handler::Leaf(|e, _| e.open_permalink()),
    },
    Binding {
        keys: "space g u",
        desc: "hunk: undo unstaged (restore from index)",
        section: "git",
        live: true,
        id: "git-hunk-undo",
        handler: Handler::Leaf(|e, _| e.undo_hunk()),
    },
    Binding {
        keys: "space g s",
        desc: "hunk: stage (live→index)",
        section: "git",
        live: true,
        id: "git-hunk-stage",
        handler: Handler::Leaf(|e, _| e.stage_hunk()),
    },
    Binding {
        keys: "space g S",
        desc: "hunk: unstage (index→HEAD)",
        section: "git",
        live: true,
        id: "git-hunk-unstage",
        handler: Handler::Leaf(|e, _| e.unstage_hunk()),
    },
    Binding {
        keys: "space g p",
        desc: "hunk: preview",
        section: "git",
        live: true,
        id: "git-hunk-preview",
        handler: Handler::Leaf(|e, _| e.preview_hunk()),
    },
    Binding {
        keys: "]f [f",
        desc: "next / prev file in commit diff",
        section: "git",
        live: true,
        id: "commit-file-nav",
        handler: Handler::Soon,
    },
    Binding {
        keys: "enter",
        desc: "dive into the line's commit (blame gutter)",
        section: "git",
        live: true,
        id: "surface-dive",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "q",
        desc: "close surface (readonly buffers)",
        section: "git",
        live: true,
        id: "surface-close",
        handler: Handler::Prefix,
    },
    // ex + panes
    Binding {
        keys: ":w :q :q! :wq :w {file}",
        desc: "write / quit (force) / write-quit / write-as",
        section: "ex+panes",
        live: true,
        id: "ex-write-quit",
        handler: Handler::TextLine,
    },
    Binding {
        keys: ":[range]s/a/b/[g] :N :% :N,Md :N,My",
        desc: "substitute (literal) / goto line / ranged delete+yank",
        section: "ex+panes",
        live: true,
        id: "ex-ranges",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "ctrl-d ctrl-u ctrl-f ctrl-b",
        desc: "half/full page scroll (count = lines)",
        section: "ex+panes",
        live: true,
        id: "scroll-pages",
        handler: Handler::TextLine,
    },
    Binding {
        keys: ":e",
        desc: "edit file",
        section: "ex+panes",
        live: true,
        id: "ex-edit",
        handler: Handler::TextLine,
    },
    Binding {
        keys: ":help",
        desc: "help buffer (this text — / searches it)",
        section: "ex+panes",
        live: true,
        id: "ex-help",
        handler: Handler::TextLine,
    },
    Binding {
        keys: ":vs :sp",
        desc: "split vertical / horizontal",
        section: "ex+panes",
        live: true,
        id: "ex-split",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "ctrl-w h / l / j / k / w",
        desc: "pane move / cycle",
        section: "ex+panes",
        live: true,
        id: "pane-nav",
        handler: Handler::Leaf(crate::editor::Editor::pane_move_pub),
    },
    Binding {
        keys: "ctrl-o / ctrl-i (tab)",
        desc: "jump back / forward (jumplist)",
        section: "ex+panes",
        live: true,
        id: "jumplist",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::jump_back(e)),
    },
    Binding {
        keys: "ctrl-w v / s",
        desc: "pane split (vs / sp)",
        section: "ex+panes",
        live: true,
        id: "pane-split",
        handler: Handler::Leaf(crate::editor::Editor::split_pub),
    },
    Binding {
        keys: ":view / -R / :set ro,noro",
        desc: "readonly browsing",
        section: "ex+panes",
        live: true,
        id: "readonly",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "ctrl-w q",
        desc: "close pane (last → buffer)",
        section: "ex+panes",
        live: true,
        id: "pane-close",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::pane_close_pub(e)),
    },
    Binding {
        keys: "up down left right tab s-tab",
        desc: "picker navigation / arrows = hjkl everywhere",
        section: "ex+panes",
        live: true,
        id: "picker-nav",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "ctrl-x",
        desc: "replace picker: exclude/include match",
        section: "ex+panes",
        live: true,
        id: "replace-exclude",
        handler: Handler::Prefix,
    },
];

/// Expand a row's `keys` into its sequences (see the notation above).
/// Dispatch lookup: the sequence (walker's tokens) → its row.
/// `<c>`/`<a>` in a row's keys match any char (parameterized rows).
pub(crate) fn expand(keys: &str) -> Vec<Vec<&str>> {
    let toks: Vec<&str> = keys.split(' ').filter(|t| !t.is_empty()).collect();
    let mut seqs: Vec<Vec<&str>> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            // a leading bare `/` is the search-forward key
            "/" if seqs.is_empty() => seqs.push(vec!["/"]),
            // alternatives replace the previous sequence's last key and
            // run to the row's end
            "/" => {
                let base: Vec<&str> = seqs
                    .last()
                    .map(|s| s[..s.len() - 1].to_vec())
                    .unwrap_or_default();
                for alt in &toks[i + 1..] {
                    if *alt != "/" {
                        let mut seq = base.clone();
                        seq.push(alt);
                        seqs.push(seq);
                    }
                }
                break;
            }
            // the leader: its sequence runs to the row's end — or the
            // first `/` with keys after it (a trailing `/` is the
            // grep key: "space /" is one sequence)
            "space" => {
                let end = (i + 1..toks.len())
                    .find(|&j| toks[j] == "/" && j + 1 < toks.len())
                    .unwrap_or(toks.len());
                seqs.push(toks[i..end].to_vec());
                i = end;
                continue;
            }
            // window commands take exactly one key
            "ctrl-w" => {
                if let Some(k) = toks.get(i + 1) {
                    seqs.push(vec!["ctrl-w", k]);
                    i += 2;
                } else {
                    seqs.push(vec!["ctrl-w"]);
                    i += 1;
                }
                continue;
            }
            t => seqs.push(vec![t]),
        }
        i += 1;
    }
    seqs
}

/// A row's sequences at PER-KEY granularity (0016: the machine's trie
/// walks keys, not row tokens): "gg" is ["g","g"], "ctrl-w h" is
/// ["ctrl-w","h"], placeholders ("<a>") stay whole and match any key.
pub(crate) fn key_seqs(row: &Binding) -> Vec<Vec<String>> {
    expand(row.keys)
        .iter()
        .map(|seq| {
            let mut out: Vec<String> = Vec::new();
            for t in seq {
                if t.len() > 1 && !t.starts_with('<') && !t.starts_with(':') && !NAMED.contains(t) {
                    if let Some(i) = t.find('<') {
                        // "r<c>": the key chars, then the placeholder whole
                        for c in t[..i].chars() {
                            out.push(c.to_string());
                        }
                        out.push(t[i..].to_string());
                    } else {
                        for c in t.chars() {
                            out.push(c.to_string());
                        }
                    }
                } else {
                    out.push(t.to_string());
                }
            }
            out
        })
        .collect()
}

const NAMED: &[&str] = &[
    "space",
    "ctrl-w",
    "ctrl-o",
    "ctrl-i",
    "up",
    "down",
    "left",
    "right",
    "tab",
    "s-tab",
    "esc",
    "enter",
    "backspace",
    "ctrl-r",
    "ctrl-x",
    "ctrl-d",
    "ctrl-u",
    "ctrl-f",
    "ctrl-b",
    "ctrl-^",
    "ctrl-v",
];

/// A placeholder token ("<a>") matches any key; the operator "<" is
/// a literal (len-1) and must not.
fn is_placeholder(k: &str) -> bool {
    k.len() > 1 && k.starts_with('<')
}

fn seq_matches(seq: &[String], path: &[String]) -> bool {
    seq.len() == path.len()
        && seq
            .iter()
            .zip(path)
            .all(|(k, t)| is_placeholder(k) || k == t)
}

fn seq_has_prefix(seq: &[String], path: &[String]) -> bool {
    seq.len() > path.len()
        && seq
            .iter()
            .zip(path)
            .all(|(k, t)| is_placeholder(k) || k == t)
}

/// The row a key path completes exactly (the machine's trie lookup).
pub(crate) fn find_row(path: &[String]) -> Option<&'static Binding> {
    BINDINGS
        .iter()
        .find(|b| b.live && key_seqs(b).iter().any(|seq| seq_matches(seq, path)))
}

/// Any live row whose sequence EXTENDS this path (trie prefix check —
/// the machine's prefixes derive from the table, never a list).
pub(crate) fn any_child(path: &[String]) -> bool {
    BINDINGS
        .iter()
        .any(|b| b.live && key_seqs(b).iter().any(|seq| seq_has_prefix(seq, path)))
}

/// One which-key hint row: the next key after a pending prefix.
pub struct Hint {
    pub key: String,
    pub desc: &'static str,
    pub live: bool,
}

/// The which-key card for a pending `prefix` in `mode`: every binding
/// that continues the prefix, keyed by the immediately-next key. When
/// several rows share a next key (the `space g` verbs under `space`),
/// the shortest sequence wins — the prefix's own row, not its first
/// verb. Only mode-appropriate sections feed the card: visual mode
/// never shows normal-mode leader verbs it can't run.
pub fn children_of(prefix: &str, mode: crate::editor::Mode) -> Vec<Hint> {
    use crate::editor::Mode;
    let sections: &[&str] = match mode {
        Mode::Normal => &["normal", "leader", "git", "ex+panes"],
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => &["visual"],
        Mode::Insert => &[],
    };
    let mut cands: Vec<(usize, Hint)> = Vec::new();
    for b in BINDINGS.iter().filter(|b| sections.contains(&b.section)) {
        for seq in expand(b.keys) {
            if let Some(key) = child_key(&seq, prefix) {
                let len: usize = seq
                    .iter()
                    .map(|t| if *t == "space" { 1 } else { t.len() })
                    .sum();
                cands.push((
                    len,
                    Hint {
                        key,
                        desc: b.desc,
                        live: b.live,
                    },
                ));
            }
        }
    }
    cands.sort_by_key(|(len, _)| *len); // shortest wins; table order breaks ties
    let mut out: Vec<Hint> = Vec::new();
    for (_, h) in cands {
        if !out.iter().any(|x| x.key == h.key) {
            out.push(h);
        }
    }
    out
}

/// The next key of `seq` under pending `prefix`: the whole token when
/// the prefix ends on a token boundary, else the rest of the partial
/// token (pending `g` vs `gg` → `g`; pending `m` vs `m<a>` → `<a>`).
fn child_key(seq: &[&str], prefix: &str) -> Option<String> {
    let mut flat = String::new();
    let mut bounds = Vec::new();
    for t in seq {
        bounds.push(flat.len());
        flat.push_str(if *t == "space" { " " } else { t });
    }
    let plen = prefix.chars().count();
    if plen == 0 || !flat.starts_with(prefix) || flat.chars().count() <= plen {
        return None;
    }
    match bounds.iter().position(|b| *b == plen) {
        Some(i) => Some(seq[i].to_string()),
        None => {
            let i = bounds.iter().rposition(|b| *b < plen)?;
            Some(seq[i].chars().skip(plen - bounds[i]).collect())
        }
    }
}

/// The vim-compatibility report (0016): generated from this table —
/// docs can never drift from dispatch. Checked into docs/vim-compat.md;
/// the test pins freshness (STROP_REGEN=1 cargo test regenerates).
pub fn compat_report() -> String {
    let mut out = String::from(
        "# Vim compatibility\n\nGenerated from the command table (`cargo test` pins freshness; \
         STROP_REGEN=1 rewrites).\n`✓` ships exactly; `(soon)` is a planned slot.\n",
    );
    for section in SECTIONS {
        out.push_str(&format!("\n## {section}\n\n"));
        for b in BINDINGS.iter().filter(|b| b.section == *section) {
            let mark = if b.live { "✓" } else { "·" };
            let soon = if b.live { "" } else { " (soon)" };
            out.push_str(&format!("- `{mark} {}` — {}{}\n", b.keys, b.desc, soon));
        }
    }
    out
}

#[cfg(test)]
mod tests;
