//! Pattern → program. Vim magic (default) and `\v` very magic; `\m`
//! accepted as the explicit default. Everything else — `\M`, `\V`,
//! lookaround, `~`, `\%V`, `\%#`, `\%23l` — is a typed refusal, never
//! a literal read.

use super::QueryError;

mod atoms;
mod classes;
mod emitter;
mod quantifier;
use emitter::Emitter;

/// Hard ceiling on repeated-atom expansion (vim allows 32767; we keep
/// the program small enough that a bounded engine stays honest).
pub(crate) const MAX_REPEAT: u32 = 10_000;
/// Ceiling on total emitted instructions (expansion bombs are typed
/// errors at compile time, not runtime hangs).
const MAX_INSTS: usize = 200_000;
/// Nesting depth ceiling — the parser recurses on `\(a\(b…`.
const MAX_DEPTH: u32 = 256;

// ---------------------------------------------------------------- machine

#[derive(Debug, Clone)]
pub(crate) enum Inst {
    /// Consume one atom at `pos`; Some(next_pos) or None (fail).
    Consume(Class),
    /// Try `prefer` first, fall back to `alt`.
    Split {
        prefer: u32,
        alt: u32,
    },
    Jmp(u32),
    /// Record `pos` in save slot n (0 = \zs, 1 = \ze, 2+ = groups).
    Save(usize),
    /// Loop-head guard: fails when `pos` did not advance since the
    /// last pass — zero-width bodies (`\(^\)*`) cannot spin.
    Guard(usize),
    LineStart,
    LineEnd,
    BufStart,
    BufEnd,
    WordStart,
    WordEnd,
    Backref {
        group: usize,
    },
    /// Never matches (a backreference to a group that doesn't exist).
    Fail,
    Match,
}

#[derive(Debug, Clone)]
pub(crate) enum Class {
    /// A literal UTF-8 run; `fold` compares case-insensitively.
    Str {
        bytes: Vec<u8>,
        fold: bool,
    },
    /// `.` — any char except a line break (`nl` = `\_.`).
    Any {
        nl: bool,
    },
    /// `\n` — exactly one whole line break (`\r\n` or `\n`).
    Break,
    Set(CharSet),
}

/// A compiled character collection. ASCII membership is a bitmask;
/// Unicode members are inclusive scalar ranges, never expanded or
/// truncated. Line breaks require an explicit `\_x` class.
#[derive(Debug, Clone)]
pub(crate) struct CharSet {
    pub(super) ascii: u128,
    pub(super) negated: bool,
    /// `\_x` forms also consume one whole line break (`\r\n` or `\n`).
    pub(super) nl: bool,
    /// `[]` / `[^]`: matches nothing (vim's quirk, corpus-pinned).
    pub(super) empty: bool,
    /// Singleton characters and ranges share the same membership path.
    pub(super) unicode_ranges: Vec<std::ops::RangeInclusive<char>>,
    /// Case-insensitive matching (`\c`) — folded at match time.
    pub(crate) fold: bool,
}

impl CharSet {
    fn new() -> Self {
        Self {
            ascii: 0,
            negated: false,
            nl: false,
            empty: true,
            unicode_ranges: Vec::new(),
            fold: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Program {
    pub(crate) insts: Vec<Inst>,
    /// Save slots: 0 = \zs, 1 = \ze, then 2 per capture group.
    pub(crate) n_slots: usize,
    /// Loop guards in play.
    pub(crate) n_loops: usize,
    pub(crate) fold: bool,
    pub(crate) whole_word: bool,
    /// The whole program is one case-sensitive literal run — the
    /// chunk-streaming fast path may take it (identical semantics,
    /// faster walk; never a fallback for anything the dialect rejects).
    pub(crate) literal: Option<Vec<u8>>,
}

// -------------------------------------------------------------------- AST

#[derive(Debug, Clone)]
enum Node {
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Rep {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
    Group {
        idx: Option<usize>,
        node: Box<Node>,
    },
    /// `\%[abc]` — prefix optionals: `a(b(c)?)?` (vim's nested shape:
    /// items match in order, stopping at the first miss).
    PrefixOpt(Vec<Node>),
    Literal(Vec<u8>),
    Set(CharSet),
    Any {
        nl: bool,
    },
    /// `\n` — one whole line break and nothing else.
    Break,
    Backref(usize),
    LineStart,
    LineEnd,
    BufStart,
    BufEnd,
    WordStart,
    WordEnd,
    SetStart,
    SetEnd,
}

// ------------------------------------------------------------------ parser

#[derive(Clone, Copy, PartialEq)]
enum Magic {
    Magic,
    VeryMagic,
}

struct Parser<'a> {
    pat: &'a [u8],
    at: usize,
    mode: Magic,
    ngroups: usize,
    fold: Option<bool>,
    depth: u32,
    /// At the start of a branch — `^` anchors here in magic mode.
    branch_start: bool,
}

pub(super) fn compile(pattern: &str, whole_word: bool) -> Result<Program, QueryError> {
    let mut p = Parser {
        pat: pattern.as_bytes(),
        at: 0,
        mode: Magic::Magic,
        ngroups: 0,
        fold: None,
        depth: 0,
        branch_start: true,
    };
    // Magic level must be the first thing in the pattern (vim's rule).
    if p.pat.starts_with(b"\\v") {
        p.mode = Magic::VeryMagic;
        p.at = 2;
    } else if p.pat.starts_with(b"\\m") {
        p.at = 2;
    }
    let node = p.parse_alt()?;
    if p.at < p.pat.len() {
        // the only token that stops a parse early is an unmatched `)`
        return Err(QueryError::UnbalancedGroup { at: p.at });
    }
    let fold = p.fold.unwrap_or(false);
    let mut e = Emitter {
        insts: Vec::new(),
        n_slots: 2 + 2 * p.ngroups,
        n_loops: 0,
        fold,
    };
    e.node(&node)?;
    e.insts.push(Inst::Match);
    // literal fast-path detection: one plain, case-sensitive run
    let literal = if whole_word || fold || e.insts.len() != 2 {
        None
    } else {
        match (&e.insts[0], &e.insts[1]) {
            (Inst::Consume(Class::Str { bytes, fold: false }), Inst::Match) => Some(bytes.clone()),
            _ => None,
        }
    };
    Ok(Program {
        insts: e.insts,
        n_slots: e.n_slots,
        n_loops: e.n_loops,
        fold,
        whole_word,
        literal,
    })
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.pat.get(self.at).copied()
    }
    fn peek2(&self) -> Option<u8> {
        self.pat.get(self.at + 1).copied()
    }
    fn starts_with(&self, s: &[u8]) -> bool {
        self.pat[self.at.min(self.pat.len())..].starts_with(s)
    }
    fn char_at(&self, at: usize) -> Result<char, QueryError> {
        let lead = *self.pat.get(at).ok_or(QueryError::Unsupported {
            construct: "invalid UTF-8 in pattern",
            at,
        })?;
        let len = utf8_len(lead);
        let end = (at + len).min(self.pat.len());
        std::str::from_utf8(&self.pat[at..end])
            .ok()
            .and_then(|s| s.chars().next())
            .ok_or(QueryError::Unsupported {
                construct: "invalid UTF-8 in pattern",
                at,
            })
    }

    /// alt := seq ( '|' seq )*
    fn parse_alt(&mut self) -> Result<Node, QueryError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(QueryError::TooComplex);
        }
        let mut branches = vec![self.parse_seq()?];
        loop {
            let sep: &[u8] = match self.mode {
                Magic::Magic => b"\\|",
                Magic::VeryMagic => b"|",
            };
            if !self.starts_with(sep) {
                break;
            }
            self.at += sep.len();
            self.branch_start = true;
            branches.push(self.parse_seq()?);
        }
        self.depth -= 1;
        Ok(if branches.len() == 1 {
            branches.pop().unwrap_or(Node::Seq(vec![]))
        } else {
            Node::Alt(branches)
        })
    }

    /// seq := rep* — stops at the branch separator / group closer.
    fn parse_seq(&mut self) -> Result<Node, QueryError> {
        let mut items: Vec<Node> = Vec::new();
        while let Some(c) = self.peek() {
            match self.mode {
                Magic::Magic => {
                    if self.starts_with(b"\\|") || self.starts_with(b"\\)") {
                        break;
                    }
                }
                Magic::VeryMagic => {
                    if c == b'|' || c == b')' {
                        break;
                    }
                }
            }
            let atom = self.parse_atom()?;
            let atom = self.parse_quantifier(atom)?;
            // `^` re-anchors after a consumed line break (vim's rule)
            // and at real branch starts; `$` mirrors it via lookahead
            self.branch_start = Self::is_break_atom(&atom);
            // merge adjacent literal runs (one Consume per run)
            if let Node::Literal(bytes) = &atom {
                if let Some(Node::Literal(prev)) = items.last_mut() {
                    prev.extend_from_slice(bytes);
                    continue;
                }
            }
            items.push(atom);
        }
        Ok(Node::Seq(items))
    }
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}
