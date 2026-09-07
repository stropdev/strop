//! The vocabulary: operators, motions, objects, commands.
//! Pure data — resolution lives in `resolve`, parsing in `parse`.

use crate::query::{CompiledQuery, QueryError};
use strop_core::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Delete,
    Yank,
    Change,
    /// > — indent.
    Indent,
    /// < — dedent.
    Dedent,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Delete => "delete",
            Op::Yank => "yank",
            Op::Change => "change",
            Op::Indent => "indent",
            Op::Dedent => "dedent",
        }
    }

    /// Key → operator (the machine's typed op entry, 0016).
    pub fn from_key(c: char) -> Option<Op> {
        match c {
            'd' => Some(Op::Delete),
            'y' => Some(Op::Yank),
            'c' => Some(Op::Change),
            '>' => Some(Op::Indent),
            '<' => Some(Op::Dedent),
            _ => None,
        }
    }

    /// The operator's key — the walker assembles grammar strings from
    /// typed parts (0008 stage 2).
    pub fn key(self) -> &'static str {
        match self {
            Op::Delete => "d",
            Op::Yank => "y",
            Op::Change => "c",
            Op::Indent => ">",
            Op::Dedent => "<",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    LineStart,
    LineEnd,
    /// `|` — screen column (count, default 1). Restored to vim semantics
    /// in 0014; pipe lives under the leader now.
    Column,
    FirstLine,
    LastLine,
    /// f/F (till=false) and t/T (till=true).
    FindChar {
        ch: char,
        till: bool,
        backward: bool,
    },
    /// `/pat⏎` — the pattern without the terminator, compiled at parse
    /// time; unsupported syntax is a typed `Parse::QueryError`, never
    /// a literal read.
    Search(CompiledQuery),
    /// `?pat⏎` — backward search.
    SearchBackward(CompiledQuery),
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
    /// % — jump to the matching pair.
    MatchPair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Word,
    /// Quote pair — the delimiter is a char (0014: no ASCII-only grammar).
    Quote(char),
    Bracket {
        open: char,
        close: char,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Motion(Motion),
    Object {
        inner: bool,
        obj: Object,
    },
    /// dd / yy / cc, or operator + j/k: whole lines.
    Linewise,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub op: Option<Op>,
    /// Named register (`"a` prefix); None = unnamed.
    pub register: Option<char>,
    pub count: Option<usize>,
    pub target: Target,
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
#[derive(Debug, Clone)]
pub struct Resolved {
    pub range: Range,
    /// Destination retained independently of sorted affected ranges (wrapped search).
    pub motion_target: Option<usize>,
    /// Motion metadata (e.g. "inner [", "word forward", "3 lines").
    /// Inclusivity lives on range.shape (0014).
    pub spec: String,
}
