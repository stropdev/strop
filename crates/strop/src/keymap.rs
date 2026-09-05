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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AbsorbKind {
    Replace,
    MarkSet,
    MarkJump,
    Find,
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
        keys: ":view / -R",
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
pub(crate) fn find_dispatch(toks: &[String]) -> Option<&'static Binding> {
    // a row token is one KEY only when it's a named key or a
    // placeholder; everything else is a char sequence ("gg", "]c")
    fn row_tokens<'a>(seq: &[&'a str]) -> Vec<std::borrow::Cow<'a, str>> {
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
        ];
        let mut out: Vec<std::borrow::Cow<'a, str>> = Vec::new();
        for t in seq {
            // ex-command tokens (":view") are whole; so are named keys
            // and <c>/<a> placeholders
            if NAMED.contains(t) || t.starts_with('<') || t.starts_with(':') {
                out.push((*t).into());
                continue;
            }
            if let Some(i) = t.find('<') {
                // "r<c>": the r, then the placeholder
                for c in t[..i].chars() {
                    out.push(c.to_string().into());
                }
                out.push(t[i..].into());
                continue;
            }
            for c in t.chars() {
                out.push(c.to_string().into());
            }
        }
        out
    }
    BINDINGS.iter().find(|b| {
        if !b.live {
            return false;
        }
        expand(b.keys).iter().any(|seq| {
            let seq = row_tokens(seq);
            seq.len() == toks.len()
                && seq
                    .iter()
                    .zip(toks)
                    .all(|(k, t)| (k.len() > 2 && k.starts_with('<')) || k.as_ref() == t)
        })
    })
}

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
        Mode::Visual | Mode::VisualLine => &["visual"],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Editor, Mode};
    use strop_core::Buffer;

    /// Every section has at least one binding; no empty rows; the
    /// "(soon)" marker comes from `live: false` only, never the desc —
    /// a desc carrying it would render the suffix twice.
    #[test]
    fn coverage_shape() {
        for section in SECTIONS {
            assert!(
                BINDINGS.iter().any(|b| b.section == *section),
                "section {section} is empty"
            );
        }
        for b in BINDINGS {
            assert!(!b.keys.is_empty() && !b.desc.is_empty());
            assert!(
                !b.desc.contains("(soon)"),
                "{}: '(soon)' belongs to live:false, not the desc",
                b.keys
            );
        }
    }

    /// The notation expands: leaders glue, alternatives fan out, a
    /// leading `/` is the search-forward key (never an alternative).
    #[test]
    fn keys_notation_expands() {
        assert_eq!(expand("space f"), vec![vec!["space", "f"]]);
        assert_eq!(expand("space /"), vec![vec!["space", "/"]]);
        assert_eq!(
            expand("space g u / s / p"),
            vec![
                vec!["space", "g", "u"],
                vec!["space", "g", "s"],
                vec!["space", "g", "p"]
            ]
        );
        assert_eq!(expand("/ ?"), vec![vec!["/"], vec!["?"]]);
        assert_eq!(expand("h j k l").len(), 4);
        assert_eq!(
            expand("ctrl-w h / l"),
            vec![vec!["ctrl-w", "h"], vec!["ctrl-w", "l"]]
        );
        for b in BINDINGS {
            let seqs = expand(b.keys);
            assert!(!seqs.is_empty(), "{}: no sequences", b.keys);
            assert!(seqs.iter().all(|s| !s.is_empty()));
        }
    }

    /// Every pending prefix with a card has hints; the space card points
    /// `g` at the git prefix row (shortest sequence wins over the
    /// verbs); soon rows come through as soon.
    #[test]
    fn which_key_children() {
        for p in [" ", " g", "g", "[", "]", "m", "'", "`"] {
            assert!(
                !children_of(p, Mode::Normal).is_empty(),
                "no hints for pending {p:?}"
            );
        }
        let space = children_of(" ", Mode::Normal);
        let g = space.iter().find(|h| h.key == "g").expect("space g hint");
        assert_eq!(g.desc, "git…");
        assert!(g.live);
        assert!(
            space.iter().any(|h| h.key == "j" && !h.live),
            "space j renders as soon"
        );
        let git = children_of(" g", Mode::Normal);
        assert_eq!(git.len(), 9); // l h b y o u s S p
        assert!(git.iter().any(|h| h.key == "u" && h.desc.contains("undo")));
        assert_eq!(children_of("m", Mode::Normal)[0].key, "<a>");
        assert_eq!(children_of("g", Mode::Normal).len(), 9); // gg gd gs ge gE gv gi g; g,
                                                             // visual mode: only the visual table feeds the card
        let v = children_of(" ", Mode::Visual);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key, "y");
        assert!(v[0].desc.contains("selection"));
    }

    /// Every dispatchable sequence, enumerated from the match arms in
    /// editor/{normal,visual,insert,git,git_memory,picker,panes}.rs and
    /// mod.rs's surface/card handlers. Adding a dispatch arm: add its
    /// sequence here AND the row it needs in BINDINGS — this test fails
    /// until both exist. Single-key motions, operators, aliases and
    /// count prefixes are the grammar's leaves (the resolver owns them;
    /// 0008's Count/Dynamic trie nodes make that structural); only
    /// sequences the hand-written match tree completes on its own are
    /// listed.
    ///
    /// The trie cutover (0008 §5) DELETES this list: dispatch walks the
    /// keymap trie, so leaves == BINDINGS rows mechanically and this
    /// hand enumeration is dead weight.
    const DISPATCHED: &[&str] = &[
        // leader (normal mode)
        "space f",
        "space b",
        "space /",
        "space R",
        "space ?",
        "space d",
        "space k",
        "space y",
        "space p",
        "space P",
        // git namespace
        "space g u",
        "space g s",
        "space g S",
        "space g p",
        "space g l",
        "space g h",
        "space g b",
        "space g y",
        "space g o",
        "space u",
        // multicursor (0013)
        "Q",
        "space c",
        // search repeat (normal mode)
        "n",
        "N",
        "*",
        "#",
        ";",
        ",",
        "^",
        "~",
        "S",
        "I",
        ":view",
        "space |",
        // prefixes with their own which-key cards
        "gg",
        "gd",
        "gs",
        "]c",
        "[c",
        "]f",
        "[f",
        "m<a>",
        "'a",
        "`a",
        "r<c>",
        // registers
        "\"xy",
        "\"xp",
        "\"+y",
        "\"+p",
        "\"+P",
        // ex + panes
        ":w",
        ":q",
        ":q!",
        ":wq",
        ":e",
        ":help",
        ":e!",
        ":vs",
        ":vsplit",
        ":sp",
        ":split",
        "ctrl-w h",
        "ctrl-w l",
        "ctrl-w j",
        "ctrl-w k",
        "ctrl-w w",
        "ctrl-w v",
        "ctrl-w s",
        "ctrl-w q",
        // picker (0007 replace incl. ctrl-x), insert closers, surfaces
        "up",
        "down",
        "left",
        "right",
        "tab",
        "s-tab",
        "ctrl-x",
        "}",
        "]",
        ")",
        "enter",
        "esc",
        "backspace",
        "q",
        // visual
        "S<c>",
        "space y",
    ];

    /// Long-form ex commands dispatch to the same arms as their short
    /// forms; they need no separate rows.
    const EX_ALIASES: &[(&str, &str)] = &[(":vsplit", ":vs"), (":split", ":sp"), (":e!", ":e")];

    /// 0003 §5.7, reverse direction: every dispatchable sequence has a
    /// BINDINGS row. A new match arm without a row fails here.
    #[test]
    fn every_dispatched_sequence_has_a_row() {
        for entry in DISPATCHED {
            let canonical = EX_ALIASES
                .iter()
                .find(|(a, _)| a == entry)
                .map(|(_, c)| *c)
                .unwrap_or(*entry);
            // match the way dispatch does: placeholders and char runs
            let toks: Vec<String> = if [
                "up",
                "down",
                "left",
                "right",
                "tab",
                "s-tab",
                "esc",
                "enter",
                "backspace",
            ]
            .contains(&canonical)
            {
                vec![canonical.to_string()]
            } else if canonical.starts_with(':') || canonical.starts_with("ctrl-") {
                canonical.split(' ').map(|t| t.to_string()).collect()
            } else {
                // walker keys: the leader is a space char, then chars;
                // <a>/<c> placeholders materialize to a concrete char
                let concretized = canonical.replace("<a>", "a").replace("<c>", "x");
                let keys = concretized
                    .strip_prefix("space ")
                    .map(|rest| format!(" {}", rest.replace(' ', "")))
                    .unwrap_or(concretized);
                crate::editor::normal::seq_tokens(&keys)
            };
            assert!(
                find_dispatch(&toks).is_some(),
                "{entry} dispatches but has no BINDINGS row (0003 §5.7)"
            );
        }
    }

    /// 0008 stage 2 structural pin: every live normal/leader row
    /// dispatches observably through the walker — the table IS dispatch.
    #[test]
    fn live_rows_dispatch_through_the_table() {
        for b in BINDINGS.iter().filter(|b| b.live) {
            for seq in expand(b.keys) {
                let keys = seq
                    .iter()
                    .map(|t| match *t {
                        "space" => " ".to_string(),
                        "<a>" => "a".into(),
                        "<c>" => "x".into(),
                        t if t.starts_with("ctrl-") || t == "up" || t == "down" || t == "tab" => {
                            String::new() // key events, not walker chars
                        }
                        t if t.contains('<') => {
                            // "f<c>": the key, then a concrete char
                            let i = t.find('<').unwrap();
                            format!("{}x", &t[..i])
                        }
                        t => t.to_string(),
                    })
                    .collect::<String>();
                if keys.is_empty() || keys.starts_with(':') || keys.starts_with('-') {
                    continue; // event-layer, ex-line, and CLI-flag rows
                }
                if matches!(b.handler, Handler::Soon) {
                    continue; // surface-only verbs (]f/[f) dispatch in the
                              // readonly layer, not on plain buffers
                }
                // legal no-ops (motions at edges) say nothing; what must
                // never happen is the unknown-key marker
                let mut e = Editor::new(Buffer::from_text("fn f(x) {\n    let y = f(x);\n}\n"));
                e.set_head(14);
                e.feed_text(&keys);
                assert!(
                    !e.message.starts_with("not an editor command"),
                    "{} (fed as {keys:?}) failed to dispatch — table drift",
                    b.keys
                );
            }
        }
    }

    /// 0003 §5.7, forward direction: every live leader/git-space
    /// binding, fed through a real editor, does something observable.
    /// A silent no-op means dispatch drifted from the table (or the row
    /// lies about being live) — either way the test fails.
    ///
    /// Probe heuristic: git/lsp-dependent verbs legitimately degrade to
    /// a message ("no hunk here", "no language server") in a bare
    /// scratch buffer; what must never happen is the unknown-key no-op
    /// (empty pending, empty message, no state change) that an unmapped
    /// key produces.
    #[test]
    fn live_leader_bindings_reach_dispatch() {
        for b in BINDINGS
            .iter()
            .filter(|b| b.live && b.keys.starts_with("space"))
        {
            for seq in expand(b.keys) {
                let keys = seq
                    .iter()
                    .map(|t| if *t == "space" { " " } else { t })
                    .collect::<String>();
                let mut e = Editor::new(Buffer::from_text("x\n"));
                e.feed_text(&keys);
                if !dispatched_something(&e) {
                    panic!(
                        "{} (fed as {keys:?}) no-op: msg={:?} pending={:?} prefix={:?}",
                        b.keys, e.message, e.pending, e.walker.prefix
                    );
                }
            }
        }
    }

    fn dispatched_something(e: &Editor) -> bool {
        !e.message.is_empty()
            || !e.pending.is_empty()
            || e.picker_open()
            || !e.walker.prefix.is_empty()
            || e.clip_paste_pending.is_some()
            || e.osc52.is_some()
            || e.mode != Mode::Normal
            || e.docs.len() != 1
            || e.head() != 0
            || e.hover_card.is_some()
            || e.blame_card.is_some()
    }

    /// Parameterized leaves the match tree completes on its own, fed
    /// concretely: marks, hunk jumps, gd without an LSP, and the
    /// clipboard register staged via OSC52.
    #[test]
    fn parameterized_leaves_reach_dispatch() {
        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("ma");
        assert_eq!(e.message, "mark a set");
        e.feed_text("'b");
        assert!(e.message.contains("not set"));

        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("]c");
        assert!(
            !e.message.is_empty() || e.head() != 0,
            "]c must jump or report, never no-op"
        );

        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("gd");
        assert!(!e.message.is_empty(), "gd with no LSP must say so");

        let mut e = Editor::new(Buffer::from_text("int x;\n"));
        e.feed_text("gs");
        assert!(!e.message.is_empty(), "gs with no LSP must say so");

        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("\"+yiw");
        assert_eq!(e.register(Some('+')).0, "hello");
        assert!(e.osc52.is_some(), "clipboard yank stages OSC52");
    }
}
