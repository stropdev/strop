//! The literal scanner: one needle streamed through a bounded window over
//! the rope (file contents are never materialized). `memchr::memmem` finds
//! exact needles; an ASCII-folded needle is found by `memchr2` on its
//! rarest byte in both cases and verified by a folded compare.
//!
//! Its answers are the bounded engine's, by construction: matches are
//! reported left to right, non-overlapping (the engine's `cpo+=c` walk),
//! and a candidate the caller's word check refuses moves the walk on by
//! one byte, as a failed attempt does. The one text the ASCII fold cannot
//! judge is KELVIN SIGN (U+212A), whose lowercase is `k`: on meeting one
//! where it could be part of a match, the scan stops at a point before
//! which every match is reported and hands the rest to the engine.

use std::ops::ControlFlow;

use memchr::memmem;
use ropey::Rope;

use crate::query::QueryError;

/// Bytes decided per window. Windows overlap by `needle.len() - 1` so a
/// match straddling two windows is whole in the second.
const BLOCK: usize = 64 * 1024;
const KELVIN: &str = "\u{212A}";

/// How a scan ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Scan {
    /// Every match was reported, or the caller stopped the walk.
    Done,
    /// Every match starting before this offset was reported; the bounded
    /// engine continues from here.
    Resume(usize),
}

enum Finder<'a> {
    Exact(memmem::Finder<'a>),
    Fold {
        needle: &'a [u8],
        /// The needle index whose byte (in either case) is searched for.
        anchor: usize,
        bytes: [u8; 2],
        /// Set when the needle has a `k`: KELVIN SIGN folds to it.
        kelvin: Option<memmem::Finder<'static>>,
    },
}

impl<'a> Finder<'a> {
    fn new(needle: &'a [u8], fold: bool) -> Self {
        if !fold || !needle.iter().any(u8::is_ascii_alphabetic) {
            return Self::Exact(memmem::Finder::new(needle));
        }
        debug_assert!(needle.is_ascii(), "folded literals are ASCII");
        let mut anchor = 0;
        for (index, byte) in needle.iter().enumerate() {
            if commonness(*byte) < commonness(needle[anchor]) {
                anchor = index;
            }
        }
        let byte = needle[anchor];
        Self::Fold {
            needle,
            anchor,
            bytes: [byte.to_ascii_lowercase(), byte.to_ascii_uppercase()],
            kelvin: needle
                .iter()
                .any(|byte| byte.eq_ignore_ascii_case(&b'k'))
                .then(|| memmem::Finder::new(KELVIN.as_bytes())),
        }
    }

    /// The first match starting in `hay[at..]` whose end is inside `hay`.
    fn find(&self, hay: &[u8], at: usize) -> Option<usize> {
        match self {
            Self::Exact(finder) => finder.find(&hay[at..]).map(|found| at + found),
            Self::Fold {
                needle,
                anchor,
                bytes,
                ..
            } => {
                let mut from = at + anchor;
                let last = hay.len().checked_sub(needle.len() - anchor)?;
                while from <= last {
                    let found = from + memchr::memchr2(bytes[0], bytes[1], &hay[from..=last])?;
                    let start = found - anchor;
                    if hay[start..start + needle.len()].eq_ignore_ascii_case(needle) {
                        return Some(start);
                    }
                    from = found + 1;
                }
                None
            }
        }
    }

    fn kelvin(&self, hay: &[u8]) -> Option<usize> {
        match self {
            Self::Fold {
                kelvin: Some(kelvin),
                ..
            } => kelvin.find(hay),
            _ => None,
        }
    }
}

/// `text[start..end]` into `window` (the bounds need not be char
/// boundaries, so no rope slice).
fn copy_bytes(text: &Rope, start: usize, end: usize, window: &mut Vec<u8>) {
    window.clear();
    let (chunks, mut at, _, _) = text.chunks_at_byte(start);
    for chunk in chunks {
        if at >= end {
            break;
        }
        let bytes = chunk.as_bytes();
        let from = start.saturating_sub(at);
        let to = bytes.len().min(end - at);
        window.extend_from_slice(&bytes[from..to]);
        at += bytes.len();
    }
}

/// How often a byte appears in source and prose, roughly (lower is
/// rarer): the folded scan searches for the needle's rarest byte. Bytes
/// not listed (digits, most punctuation) count as rarest.
fn commonness(byte: u8) -> u8 {
    const COMMON: &[u8] = b"zqjxkvbpygfw_.mucldrhsnioate ";
    match COMMON
        .iter()
        .position(|common| *common == byte.to_ascii_lowercase())
    {
        Some(rank) => rank as u8 + 1,
        None => 0,
    }
}

/// Walk the matches of `needle` (case-folded when `fold`; then ASCII)
/// starting at or after `from`, a char boundary. `accept` is the caller's
/// word check on `[start, end)`; `matched` receives each accepted start
/// and may stop the walk.
pub(super) fn scan(
    text: &Rope,
    needle: &[u8],
    fold: bool,
    from: usize,
    cancelled: &dyn Fn() -> bool,
    accept: impl Fn(usize, usize) -> bool,
    mut matched: impl FnMut(usize) -> ControlFlow<()>,
) -> Result<Scan, QueryError> {
    if cancelled() {
        return Err(QueryError::Cancelled);
    }
    let len = text.len_bytes();
    let n = needle.len();
    if n == 0 || from + n > len {
        return Ok(Scan::Done);
    }
    let finder = Finder::new(needle, fold);
    let mut window = Vec::with_capacity(BLOCK + n + KELVIN.len());
    // starts below `decided` are reported or ruled out; `next` is the
    // end of the last match (the walk never overlaps one)
    let (mut decided, mut next) = (from, from);
    while decided + n <= len {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        // a KELVIN SIGN straddling the previous window's end is whole here
        let start = decided.saturating_sub(KELVIN.len() - 1).max(from);
        let end = (decided + BLOCK + n - 1).min(len);
        copy_bytes(text, start, end, &mut window);
        // starts in [decided, stop) are decided by this window
        let mut stop = end + 1 - n;
        let mut resume = None;
        if let Some(kelvin) = finder.kelvin(&window) {
            // a match holding it is at most 3 bytes per needle byte long,
            // so none starts before `kelvin - 3n`; the engine re-walks
            // from there (past the last reported match)
            let safe = (start + kelvin).saturating_sub(3 * n);
            stop = safe.max(decided);
            resume = Some(safe.max(next));
        }
        let mut at = decided.max(next);
        while at < stop {
            let hay = &window[..stop + n - 1 - start];
            let Some(found) = finder.find(hay, at - start) else {
                break;
            };
            let hit = start + found;
            if accept(hit, hit + n) {
                if matched(hit).is_break() {
                    return Ok(Scan::Done);
                }
                next = hit + n;
                at = next;
            } else {
                at = hit + 1;
            }
        }
        if let Some(resume) = resume {
            return Ok(Scan::Resume(resume));
        }
        decided = stop;
    }
    if cancelled() {
        Err(QueryError::Cancelled)
    } else {
        Ok(Scan::Done)
    }
}
