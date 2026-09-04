//! The vocabulary: operators, motions, objects, commands.
//! Pure data — resolution lives in `resolve`, parsing in `parse`.

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
        ch: u8,
        till: bool,
        backward: bool,
    },
    /// `/pat⏎` — the pattern without the terminator.
    Search(String),
    /// `?pat⏎` — backward search.
    SearchBackward(String),
    /// W / B / E — WORD motions (whitespace-delimited).
    BigWordForward,
    BigWordBackward,
    BigWordEnd,
    /// % — jump to the matching pair.
    MatchPair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Word,
    Quote(u8),
    Bracket { open: u8, close: u8 },
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
    SurroundDelete(u8),
    /// cs"' — change surrounding pair from → to.
    SurroundChange {
        from: u8,
        to: u8,
    },
    /// ys<motion><char> — wrap the motion's target. Visual S<char>.
    SurroundAdd {
        ch: u8,
        inner: Box<Target>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub op: Option<Op>,
    /// Named register (`"a` prefix); None = unnamed.
    pub register: Option<char>,
    pub count: usize,
    pub target: Target,
    /// The keys that produced this command (dot-repeat, flash, spec footer).
    pub keys: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parse {
    Incomplete,
    Invalid,
    Complete(Command),
}

/// What the resolver found: the affected bytes plus the spec-footer text.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub range: Range,
    pub inclusive: bool,
    /// e.g. "inner [", "word forward", "find ':'", "search /enum", "3 lines".
    pub spec: String,
}
