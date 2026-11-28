//! Which programs the literal scanner may run. A program qualifies only
//! when the scanner's answer is provably the bounded engine's: one
//! literal run, optionally flanked by `\<`/`\>` or the whole-word flag,
//! case-sensitive or folding ASCII letters only. Everything else keeps
//! the general engine.

use super::{Class, Inst};

/// A program that is one literal run (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Literal {
    pub(crate) bytes: Vec<u8>,
    /// ASCII case folding. Only set for an all-ASCII needle: the one
    /// non-ASCII char whose lowercase is ASCII (KELVIN SIGN, `k`) is the
    /// scanner's to handle.
    pub(crate) fold: bool,
    /// No word char may end where the match starts.
    pub(crate) word_before: bool,
    /// No word char may start where the match ends.
    pub(crate) word_after: bool,
}

impl Literal {
    pub(super) fn of(insts: &[Inst], whole_word: bool) -> Option<Self> {
        let (word_start, insts) = match insts {
            [Inst::WordStart, rest @ ..] => (true, rest),
            rest => (false, rest),
        };
        let (bytes, fold, word_end) = match insts {
            [Inst::Consume(Class::Str { bytes, fold }), Inst::Match] => (bytes, *fold, false),
            [Inst::Consume(Class::Str { bytes, fold }), Inst::WordEnd, Inst::Match] => {
                (bytes, *fold, true)
            }
            _ => return None,
        };
        let text = std::str::from_utf8(bytes).ok()?;
        let (first, last) = (text.chars().next()?, text.chars().next_back()?);
        // `\<` needs a word char at the match start and `\>` one at its
        // end; with those given, each is exactly a check on the
        // neighbouring char.
        if (word_start && !is_word(first)) || (word_end && !is_word(last)) {
            return None;
        }
        if fold && !text.is_ascii() {
            return None;
        }
        Some(Self {
            bytes: bytes.clone(),
            fold,
            word_before: word_start || whole_word,
            word_after: word_end || whole_word,
        })
    }
}

/// The engine's word classifier (`exec::Matcher::word`).
pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
