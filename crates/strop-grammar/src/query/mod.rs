//! The search query (0031 R5): a compiled, bounded Vim-magic regex.
//!
//! One `CompiledQuery` serves every consumer — the resolver, `n`/`N`,
//! incsearch, the render highlight — so filtering (whole-word) and
//! semantics live in exactly one place. Unsupported constructs are a
//! typed [`QueryError`], never a silent literal reading.
//!
//! The dialect is Vim's default magic mode (plus `\v` very magic),
//! pinned against real vim by `corpora/vim-query-corpus.txt` (buffer
//! semantics: `.` and `[...]` never cross a line break, `\n` consumes
//! one whole line break — `\r\n` counts as one in CRLF buffers).
//! Documented extensions over vim: `\<`/`\>` and case folding follow
//! the editor's char-classified word model (`is_alphanumeric`/`_`), so
//! `é` is a word char for boundaries like it is for `w`/`b`/`*`.

pub(crate) mod exec;
mod parse;

use strop_core::id::ByteOffset;
use strop_core::Buffer;

pub use exec::STEP_BUDGET;
use parse::Program;

/// An explicit match range `[start, end)` in buffer bytes. The length
/// is whatever the pattern matched — never assume `end - start` equals
/// the pattern's length (0031: highlighting reads this range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchMatch {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl SearchMatch {
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Why a query cannot compile or run. Typed end to end — the walker
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// A construct outside the supported dialect (lookaround, `\%V`,
    /// `\%#`, `\%23l`-style positions, `~`, the `\M`/`\V` magic
    /// levels, the `\%=` engine selector). `at` is the byte offset in
    /// the pattern where the construct starts.
    Unsupported { construct: &'static str, at: usize },
    /// `\(` never closed, or a stray `\)`.
    UnbalancedGroup { at: usize },
    /// `[` never closed.
    UnclosedClass { at: usize },
    /// `\%[` never closed.
    UnclosedOptional { at: usize },
    /// Malformed `\{...}` (bad numbers, missing `\}`), a quantifier
    /// with nothing to repeat, or a quantifier after a quantifier.
    BadRepeat { at: usize },
    /// Bad collection contents: reverse range, unknown POSIX class,
    /// or `&&` intersection.
    BadClass { at: usize },
    /// Malformed `\%d`/`\%x`/`\%u`/`\%U` char code.
    BadCharCode { at: usize },
    /// The pattern ends in a lone backslash.
    TrailingBackslash { at: usize },
    /// The bounded engine's step budget ran out (vim's E363 family) —
    /// the pattern is too expensive to run, not wrong.
    TooComplex,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (head, at): (&str, Option<usize>) = match self {
            QueryError::Unsupported { construct, at } => {
                return write!(f, "unsupported query syntax: {construct} (at byte {at})")
            }
            QueryError::UnbalancedGroup { at } => ("unbalanced \\( \\)", Some(*at)),
            QueryError::UnclosedClass { at } => ("unclosed [...]", Some(*at)),
            QueryError::UnclosedOptional { at } => ("unclosed \\%[", Some(*at)),
            QueryError::BadRepeat { at } => ("malformed repetition", Some(*at)),
            QueryError::BadClass { at } => ("malformed [...]", Some(*at)),
            QueryError::BadCharCode { at } => ("malformed character code", Some(*at)),
            QueryError::TrailingBackslash { at } => ("trailing backslash", Some(*at)),
            QueryError::TooComplex => {
                return write!(f, "query too complex (over {STEP_BUDGET} steps)")
            }
        };
        match at {
            Some(at) => write!(f, "{head} (at byte {at})"),
            None => write!(f, "{head}"),
        }
    }
}

impl std::error::Error for QueryError {}

/// A compiled search: everything about the query — the source text,
/// the whole-word flag (`*`/`#` fold it in here, so no consumer
/// re-filters), the case mode (`\c`/`\C`), and the program.
///
/// Deterministic compilation makes equality source-level: two queries
/// with the same text and whole-word flag are the same query.
#[derive(Clone)]
pub struct CompiledQuery {
    source: String,
    whole_word: bool,
    prog: Program,
}

impl CompiledQuery {
    /// Compile `pattern` in Vim magic syntax. `whole_word` flanks the
    /// match with word-boundary assertions (the `*`/`#` search shape).
    pub fn compile(pattern: &str, whole_word: bool) -> Result<Self, QueryError> {
        let prog = parse::compile(pattern, whole_word)?;
        Ok(Self {
            source: pattern.to_string(),
            whole_word,
            prog,
        })
    }

    /// The pattern as typed (spec footers, `/` line echo, `n` replay).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether matches are flanked to whole words (render and `n`/`N`
    /// ask this instead of re-implementing the filter).
    pub fn whole_word(&self) -> bool {
        self.whole_word
    }

    pub(super) fn program(&self) -> &Program {
        &self.prog
    }
}

impl std::fmt::Debug for CompiledQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledQuery")
            .field("source", &self.source)
            .field("whole_word", &self.whole_word)
            .finish()
    }
}

impl PartialEq for CompiledQuery {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.whole_word == other.whole_word
    }
}
impl Eq for CompiledQuery {}

/// First match starting at/after `from` (clamped up to a char
/// boundary). Errors only from the step budget ([`QueryError::TooComplex`]).
pub fn search_forward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    super::resolve::search::forward(buf, from, query)
}

/// Last match whose start precedes `from`, even if its end crosses the
/// cursor (`?pat` from inside a hit still finds it).
pub fn search_backward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    super::resolve::search::backward(buf, from, query)
}

/// Non-overlapping matches from the start of the buffer; an empty
/// match advances one char (the historical `match_indices` contract,
/// and vim's `cpo+=c` walk). The highlight range source.
pub fn search_all(buf: &Buffer, query: &CompiledQuery) -> Result<Vec<SearchMatch>, QueryError> {
    super::resolve::search::all(buf, query)
}
