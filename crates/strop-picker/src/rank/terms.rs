//! Query terms and the two things they can match (0093 §3.2–§3.3).
//!
//! A query is its whitespace-separated terms; every term must match. On a
//! path, a term is tried against the file name first: a name match outranks
//! any directory match, and a term that *is* the name outranks a name it
//! merely occurs in. Only a term the name rejects — or one that spells a
//! `/` — is matched against the whole path. That is VS Code's label-first
//! scorer and fzf's `--scheme=path`, expressed over nucleo's atoms.

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Matcher, Utf32Str};

/// A term matched inside the file name. Larger than any score a whole
/// path can earn, so no directory match ever outranks a name match.
const NAME: u32 = 1 << 14;
/// A term that spells the whole file name: typing a name finds that file
/// ahead of longer names that contain it.
const WHOLE_NAME: u32 = 1 << 15;
/// The share of the name a term covers, in points: `sched` prefers
/// `sched.h` to `sched_clock_internal.c`.
const COVERAGE: usize = 100;

struct Term {
    atom: Atom,
    chars: usize,
    /// A term that spells a separator addresses the path, never the name.
    path_only: bool,
}

pub(super) struct Terms {
    terms: Vec<Term>,
    unicode: Vec<char>,
}

impl Terms {
    pub(super) fn new(query: &str, case: CaseMatching) -> Self {
        let terms = query
            .split_whitespace()
            .map(|term| Term {
                atom: Atom::new(term, case, Normalization::Smart, AtomKind::Fuzzy, false),
                chars: term.chars().count(),
                path_only: term.contains('/'),
            })
            .collect();
        Self {
            terms,
            unicode: Vec::new(),
        }
    }

    /// Every term against the whole text: the score is their sum.
    pub(super) fn score_text(
        &mut self,
        text: &str,
        matcher: &mut Matcher,
        columns: &mut Vec<u32>,
    ) -> Option<u32> {
        let haystack = haystack(text, &mut self.unicode);
        let mut total = 0u32;
        for term in &self.terms {
            total += u32::from(term.atom.indices(haystack, matcher, columns)?);
        }
        Some(total)
    }

    /// Every term against a path, file name first.
    pub(super) fn score_path(
        &mut self,
        path: &str,
        matcher: &mut Matcher,
        columns: &mut Vec<u32>,
    ) -> Option<u32> {
        let name_at = path.rfind('/').map_or(0, |slash| slash + 1);
        let name_column = path[..name_at].chars().count();
        let whole = haystack(path, &mut self.unicode);
        let name = whole.slice(name_column..);
        let name_chars = name.len();
        let mut total = 0u32;
        for term in &self.terms {
            let before = columns.len();
            let in_name = if term.path_only || name_chars == 0 {
                None
            } else {
                term.atom.indices(name, matcher, columns)
            };
            total += match in_name {
                Some(score) => {
                    for column in &mut columns[before..] {
                        *column += name_column as u32;
                    }
                    let covered = term.chars.min(name_chars) * COVERAGE / name_chars;
                    let whole_name = if term.chars == name_chars {
                        WHOLE_NAME
                    } else {
                        0
                    };
                    NAME + whole_name + covered as u32 + u32::from(score)
                }
                None => u32::from(term.atom.indices(whole, matcher, columns)?),
            };
        }
        Some(total)
    }
}

fn haystack<'a>(text: &'a str, unicode: &'a mut Vec<char>) -> Utf32Str<'a> {
    if text.is_ascii() {
        Utf32Str::Ascii(text.as_bytes())
    } else {
        unicode.clear();
        unicode.extend(text.chars());
        Utf32Str::Unicode(unicode)
    }
}
