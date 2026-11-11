//! Ex commands. Every command in `editor/ex/table.rs` is named by a row
//! here or in another section (`keymap/tests.rs` checks it).
use super::super::{Binding, Handler};

const fn row(keys: &'static str, desc: &'static str, id: &'static str) -> Binding {
    Binding {
        keys,
        desc,
        sections: &["ex"],
        live: true,
        id,
        handler: Handler::TextLine,
    }
}

pub(super) const ROWS: &[Binding] = &[
    row(":", "ex command line (from Visual it opens with '<,'>)", "ex-line"),
    row(
        ":N :N,M :. :$ :% :'a :'<,'> :/pat/ :?pat? :.+N :N;+M",
        "ranges: lines, marks, searches and offsets; :N alone goes to the line",
        "ex-ranges",
    ),
    row(
        ":s/pat/new/[flags] :& :&& :~",
        "substitute with live preview (& \\1 ~ \\r \\u \\U in the replacement; flags g i I n c &) / repeat it",
        "ex-substitute",
    ),
    row(
        ":g/pat/cmd :g!/pat/cmd :v/pat/cmd :norm {keys}",
        "run a command on matching (or the other) lines / type Normal keys on each line",
        "ex-global",
    ),
    row(
        ":d {x} :y {x} :m {addr} :t {addr} :co {addr} :j :pu {x}",
        "delete / yank / move / copy / join / put lines (x: a register; a count may follow)",
        "ex-lines",
    ),
    row(
        ":> :< :sort :!cmd",
        "indent / unindent / sort lines (! reversed; u n i) / run a command, or with a range filter the lines through it",
        "ex-line-filters",
    ),
    row(
        ":k {a} :mark {a} :delm {a} :marks :reg {x} :di {x}",
        "set / delete marks; list marks and registers",
        "ex-marks-registers",
    ),
    row(
        ":u :red :@: :@{x}",
        "undo / redo / repeat the last command line / run register x as commands",
        "ex-history",
    ),
    row(
        ":b {n|name} :bn :bp :bN :bd {n} :ls :buffers :files",
        "go to a buffer by number or name fragment / next / previous / delete / list",
        "ex-buffers",
    ),
    row(
        ":w :w {file} :up :wa",
        "write / write-as / write if modified / write every modified buffer",
        "ex-write",
    ),
    row(
        ":q :q! :wq :x :exi :clo :on :trust",
        "quit (force) / write-quit / close this window / only this window / trust the project",
        "ex-write-quit",
    ),
    row(
        ":qa :qall :quita :wqa :xa",
        "quit all (fails on unsaved; bang discards) / write all and quit",
        "ex-quit-all",
    ),
    row(
        ":e :e {file} :ene :new :vne",
        "edit file / new unnamed buffer (here, split, vertical split)",
        "ex-edit",
    ),
    row(
        ":cd {dir|-} :pwd",
        "change the project root (Files, Search, terminals) / go back / show it",
        "ex-directory",
    ),
    row(
        ":help :mes :explain",
        "help buffer (this text — / searches it) / message history / why: workspaces, LSP, config",
        "ex-help",
    ),
    row(":theme", "choose a built-in interface palette", "ex-theme"),
    row(
        ":jumps :symbols",
        "jumplist / document symbols picker",
        "ex-navigation-pickers",
    ),
    row(
        ":format :rename",
        "format buffer / rename symbol: :rename NEW",
        "ex-lsp-edits",
    ),
    row(
        ":recover",
        "draft recovery: list, restore N, discard N, consent remote|off",
        "ex-recover",
    ),
    row(
        ":collection source|expand|contract",
        "source, expand or contract the excerpt at the caret",
        "ex-collection",
    ),
    row(
        ":select-next :select-all :select-skip :select-pop",
        "occurrence selection: next / all / skip / pop last-added",
        "ex-occurrence-selection",
    ),
    row(
        ":view :set",
        "readonly browsing (:set ro/noro; CLI: -R)",
        "readonly",
    ),
    Binding {
        keys: ":tab-size :indent-style :search-options",
        desc: "source indentation and search visibility controls",
        sections: &["ex"],
        live: true,
        id: "source-and-search-settings",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":fs :browse :filter :remote :terminal-local",
        desc: "filesystem actions, namespace-aware browsing, folder filtering, remote destinations, a local shell",
        sections: &["ex"],
        live: true,
        id: "filesystem",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":apply-change :cancel-change :save-change :undo-change",
        desc: "review: apply text or filesystem changes, cancel, save changed text, or undo the last applied plan",
        sections: &["ex"],
        live: true,
        id: "change-review-actions",
        handler: Handler::Contextual,
    },
];
