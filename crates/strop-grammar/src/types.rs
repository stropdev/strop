//! The vocabulary: operators, motions, objects, commands.
//! Pure data — resolution lives in `resolve`, parsing in `parse`.

use crate::query::{CompiledQuery, QueryError};
use strop_core::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Op {
    Delete,
    Yank,
    Change,
    /// > — indent.
    Indent,
    /// < — dedent.
    Dedent,
    /// gU — uppercase.
    Upper,
    /// gu — lowercase.
    Lower,
    /// g~ — toggle case.
    ToggleCase,
    /// g? — rot13.
    Rot13,
    /// ! — filter the lines through a shell command.
    Filter,
    /// = — reindent. Parsed so that it consumes its motion; not implemented.
    Reindent,
    /// gq / gw — reflow. Parsed so that they consume their motion; not
    /// implemented.
    Format,
    FormatKeep,
    /// zf — fold. Parsed so that it consumes its motion; not implemented.
    Fold,
}

impl Op {
    /// Every operator with its keys. The parser, the walker and the
    /// keymap rows all read this one list.
    pub const ALL: &'static [(Op, &'static str)] = &[
        (Op::Delete, "d"),
        (Op::Yank, "y"),
        (Op::Change, "c"),
        (Op::Indent, ">"),
        (Op::Dedent, "<"),
        (Op::Upper, "gU"),
        (Op::Lower, "gu"),
        (Op::ToggleCase, "g~"),
        (Op::Rot13, "g?"),
        (Op::Filter, "!"),
        (Op::Reindent, "="),
        (Op::Format, "gq"),
        (Op::FormatKeep, "gw"),
        (Op::Fold, "zf"),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Delete => "delete",
            Op::Yank => "yank",
            Op::Change => "change",
            Op::Indent => "indent",
            Op::Dedent => "dedent",
            Op::Upper => "uppercase",
            Op::Lower => "lowercase",
            Op::ToggleCase => "toggle case",
            Op::Rot13 => "rot13",
            Op::Filter => "filter",
            Op::Reindent => "reindent",
            Op::Format | Op::FormatKeep => "reflow",
            Op::Fold => "fold",
        }
    }

    /// Keys → operator (the machine's typed op entry, 0016).
    pub fn from_keys(keys: &str) -> Option<Op> {
        Self::ALL
            .iter()
            .find(|(_, operator)| *operator == keys)
            .map(|(op, _)| *op)
    }

    /// The operator works on whole lines whatever the motion.
    pub fn on_lines(self) -> bool {
        matches!(
            self,
            Op::Indent
                | Op::Dedent
                | Op::Filter
                | Op::Reindent
                | Op::Format
                | Op::FormatKeep
                | Op::Fold
        )
    }

    /// Neovim has it and strop does not: it consumes its motion, so the
    /// motion's keys never run as commands, and then says so.
    pub fn missing(self) -> bool {
        matches!(self, Op::Reindent | Op::Format | Op::FormatKeep | Op::Fold)
    }

    /// The operator's keys — the walker assembles grammar strings from
    /// typed parts (0008 stage 2).
    pub fn key(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(op, _)| *op == self)
            .map_or("", |(_, keys)| *keys)
    }
}

/// `H` / `M` / `L`: a line of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WindowLine {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Motion {
    Left,
    Down,
    Up,
    Right,
    WordForward,
    WordBackward,
    WordEnd,
    /// `^` — first non-blank char of the line.
    FirstNonBlank,
    /// `_` — first non-blank char after moving count - 1 lines down.
    Underscore,
    LineStart,
    /// `$` — the line's last character, count - 1 lines down.
    LineEnd,
    /// `g_` — the line's last non-blank, count - 1 lines down.
    LastNonBlank,
    /// `|` — screen column (count, default 1). Restored to vim semantics
    /// in 0014; pipe lives under the leader now.
    Column,
    FirstLine,
    LastLine,
    /// `-` — count lines up, on the first non-blank.
    LineUp,
    /// `+` / Enter — count lines down, on the first non-blank.
    LineDown,
    /// `gj` / `gk` — like `j` / `k`, but characterwise under an operator,
    /// so the column matters: `column` is the one the cursor wants (it
    /// can be past a short line), bound by the engine before resolution.
    ScreenDown {
        column: Option<usize>,
    },
    ScreenUp {
        column: Option<usize>,
    },
    /// f/F (till=false) and t/T (till=true).
    FindChar {
        ch: char,
        till: bool,
        backward: bool,
    },
    /// `;` / `,` — the last f/F/t/T again. The engine binds it to that
    /// `FindChar` before resolution.
    RepeatFind {
        reverse: bool,
    },
    /// `/pat⏎` — the pattern without the terminator, compiled at parse
    /// time; unsupported syntax is a typed `Parse::QueryError`, never
    /// a literal read.
    Search(CompiledQuery),
    /// `?pat⏎` — backward search.
    SearchBackward(CompiledQuery),
    /// `n` / `N` — the last search again. The engine binds it to that
    /// `Search` before resolution.
    RepeatSearch {
        reverse: bool,
    },
    /// W / B / E — WORD motions (whitespace-delimited).
    BigWordForward,
    BigWordBackward,
    BigWordEnd,
    /// `ge` / `gE` — end of the PREVIOUS word (never the current one).
    WordEndBackward,
    BigWordEndBackward,
    /// `{` / `}` — paragraph motions (to the blank line, else edge).
    ParagraphBackward,
    ParagraphForward,
    /// `(` / `)` — sentence motions.
    SentenceBackward,
    SentenceForward,
    /// `[(` / `[{` — the unmatched opening bracket before the cursor.
    UnmatchedOpen(char),
    /// `])` / `]}` — the unmatched closing bracket after the cursor,
    /// named by its opening bracket.
    UnmatchedClose(char),
    /// `%` — jump to the matching pair; with a count, to that percentage
    /// of the buffer's lines.
    MatchPair,
    /// `'a` (line) / `` `a `` (exact). `at` is the mark's position, bound
    /// by the engine before resolution; unbound, the motion fails.
    Mark {
        name: char,
        exact: bool,
        at: Option<usize>,
    },
    /// `H` / `M` / `L`. `line` is bound by the engine from the window.
    Window {
        which: WindowLine,
        line: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Object {
    Word,
    /// iW / aW — WORD object: a whitespace-delimited run.
    BigWord,
    /// ip / ap — paragraph: lines up to a blank line.
    Paragraph,
    /// Quote pair — the delimiter is a char (0014: no ASCII-only grammar).
    Quote(char),
    Bracket {
        open: char,
        close: char,
    },
    /// it / at — the XML/HTML element.
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    Motion(Motion),
    Object {
        inner: bool,
        obj: Object,
    },
    /// dd / yy / cc, or operator + j/k: whole lines.
    Linewise,
    /// `gn` / `gN` — the next (previous) match of the last search, as a
    /// selection. `query` is bound by the engine before resolution.
    Match {
        backward: bool,
        query: Option<CompiledQuery>,
    },
    /// ds" — delete the surrounding pair.
    SurroundDelete(char),
    /// cs"' — change surrounding pair from → to.
    SurroundChange {
        from: char,
        to: char,
    },
    /// ys<motion><char> — wrap the motion's target. Visual S<char>.
    SurroundAdd {
        ch: char,
        inner: Box<Target>,
    },
}

/// `v` / `V` / `Ctrl-V` between an operator and its motion
/// (`:help o_v`): read the motion as this kind of range instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Force {
    Charwise,
    Linewise,
    Blockwise,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Command {
    pub op: Option<Op>,
    /// Named register (`"a` prefix); None = unnamed.
    pub register: Option<char>,
    pub count: Option<usize>,
    pub target: Target,
    pub force: Option<Force>,
    /// The keys that produced this command (dot-repeat, flash, spec footer).
    pub keys: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parse {
    Incomplete,
    Invalid,
    /// The search pattern did not compile — the typed refusal
    /// (unsupported dialect, malformed repetition, …). Shown on the
    /// `/` line; never falls back to a literal search.
    QueryError(QueryError),
    Complete(Command),
}

/// What the resolver found: the affected bytes plus the spec-footer text.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Resolved {
    pub range: Range,
    /// Where the motion itself goes (the cursor when no operator is
    /// pending). Text objects have none.
    pub motion_target: Option<usize>,
    /// The first position of the operated text before a linewise range
    /// widens it to whole lines (Vim's `oap->start`): where an operator
    /// that keeps the text leaves the cursor.
    pub origin: usize,
    /// The range is whole lines only because an exclusive motion stopped
    /// in column 1 from a start in the indent (`:help exclusive-linewise`).
    /// Vim then leaves the cursor on the first non-blank, not in its column.
    pub end_adjusted: bool,
    /// Motion metadata (e.g. "inner [", "word forward", "3 lines").
    /// Inclusivity lives on range.shape (0014).
    pub spec: String,
}
