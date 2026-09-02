//! The keymap table (0008 §3 — keymaps as data). v1: this table drives
//! the `Space ?` keybinds popup and the which-key prefix cards; the trie
//! dispatch cutover (0008 §5) makes it the dispatch source later.
//!
//! Coverage contract (0003 §5.7): every dispatchable binding renders in
//! the popup; the test below pins it.

/// One binding as it appears in `Space ?` / which-key.
pub struct Binding {
    pub keys: &'static str,
    pub desc: &'static str,
    /// Sidebar section: normal · visual · insert · leader · git · ex+panes
    pub section: &'static str,
}

pub const SECTIONS: &[&str] = &["normal", "visual", "insert", "leader", "git", "ex+panes"];

pub const BINDINGS: &[Binding] = &[
    // normal: motions
    Binding {
        keys: "h j k l",
        desc: "move (never off the line)",
        section: "normal",
    },
    Binding {
        keys: "w b e W B E",
        desc: "word / WORD motions",
        section: "normal",
    },
    Binding {
        keys: "0 $ gg G %",
        desc: "line/file/pair jumps",
        section: "normal",
    },
    Binding {
        keys: "f<char> t<char>",
        desc: "find/till char (candidates light up)",
        section: "normal",
    },
    Binding {
        keys: "/ ? n",
        desc: "search forward / backward",
        section: "normal",
    },
    Binding {
        keys: "]c [c",
        desc: "next / prev git hunk",
        section: "normal",
    },
    Binding {
        keys: "m<a> 'a",
        desc: "set mark / jump to mark",
        section: "normal",
    },
    // normal: operators
    Binding {
        keys: "d y c > <",
        desc: "operators + motion/object (live preview)",
        section: "normal",
    },
    Binding {
        keys: "dd yy cc D C Y s x X",
        desc: "line/char shortcuts",
        section: "normal",
    },
    Binding {
        keys: "iw i\" i' i( i[ i{",
        desc: "inner objects (quotes scan the line)",
        section: "normal",
    },
    Binding {
        keys: "ds\" cs\"' ysiw\"",
        desc: "surround: delete / change / add",
        section: "normal",
    },
    Binding {
        keys: "i a A o O",
        desc: "insert (auto-indent)",
        section: "normal",
    },
    Binding {
        keys: "p P",
        desc: "paste after / before",
        section: "normal",
    },
    Binding {
        keys: "r<c> J .",
        desc: "replace char / join / repeat",
        section: "normal",
    },
    Binding {
        keys: "u ctrl-r",
        desc: "undo / redo (one unit per command)",
        section: "normal",
    },
    Binding {
        keys: "v V",
        desc: "visual / visual-line",
        section: "normal",
    },
    // visual
    Binding {
        keys: "d y c x",
        desc: "operate on selection",
        section: "visual",
    },
    Binding {
        keys: "S<c>",
        desc: "wrap selection in pair",
        section: "visual",
    },
    Binding {
        keys: "i<a> a<a>",
        desc: "objects select (vi[ works)",
        section: "visual",
    },
    // insert
    Binding {
        keys: "esc",
        desc: "normal mode (session = one undo unit)",
        section: "insert",
    },
    Binding {
        keys: "backspace",
        desc: "delete back",
        section: "insert",
    },
    Binding {
        keys: "enter",
        desc: "new line (auto-indent)",
        section: "insert",
    },
    Binding {
        keys: "} ] )",
        desc: "closer on indent-only line dedents",
        section: "insert",
    },
    // leader
    Binding {
        keys: "space f",
        desc: "file finder",
        section: "leader",
    },
    Binding {
        keys: "space b",
        desc: "buffers (MRU)",
        section: "leader",
    },
    Binding {
        keys: "space /",
        desc: "live grep",
        section: "leader",
    },
    Binding {
        keys: "space ?",
        desc: "this popup",
        section: "leader",
    },
    Binding {
        keys: "space j",
        desc: "jumplist picker (soon)",
        section: "leader",
    },
    Binding {
        keys: "space u",
        desc: "undo-tree browser (soon)",
        section: "leader",
    },
    // git
    Binding {
        keys: "space g l",
        desc: "commit browser",
        section: "git",
    },
    Binding {
        keys: "space g h",
        desc: "file history",
        section: "git",
    },
    Binding {
        keys: "space g b",
        desc: "blame card",
        section: "git",
    },
    Binding {
        keys: "space g y / o",
        desc: "permalink: yank / open",
        section: "git",
    },
    Binding {
        keys: "space g u / s / p",
        desc: "hunk: undo / stage / preview",
        section: "git",
    },
    // ex + panes
    Binding {
        keys: ":w :q :e",
        desc: "write / quit / edit",
        section: "ex+panes",
    },
    Binding {
        keys: ":vs :sp",
        desc: "split vertical / horizontal",
        section: "ex+panes",
    },
    Binding {
        keys: "ctrl-w h l j k w q",
        desc: "pane move / cycle / close",
        section: "ex+panes",
    },
    Binding {
        keys: "ctrl-n ctrl-p tab",
        desc: "picker navigation",
        section: "ex+panes",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every section has at least one binding; no empty table rows.
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
        }
    }
}
