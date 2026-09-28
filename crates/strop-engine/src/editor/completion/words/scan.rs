//! Tokenization uses the grammar's keyword class and one bounded scratch token.
use super::{IndexError, IndexWork, MAX_WORD_BYTES};
use ropey::Rope;
use std::ops::Range;
use strop_grammar::is_keyword;

pub(super) fn visit(
    rope: &Rope,
    range: Range<usize>,
    cancelled: &mut impl FnMut() -> bool,
    mut word: impl FnMut(Option<&str>) -> Result<(), IndexError>,
) -> Result<IndexWork, IndexError> {
    if cancelled() {
        return Err(IndexError::Cancelled);
    }
    let start = boundary(rope, range.start)?;
    let end = boundary(rope, range.end)?;
    if start > end {
        return Err(IndexError::InvalidGeometry);
    }
    if start == end {
        return Ok(IndexWork::default());
    }
    let mut omitted = start > 0 && is_keyword(rope.char(start - 1)) && is_keyword(rope.char(start));
    let continued =
        end < rope.len_chars() && is_keyword(rope.char(end - 1)) && is_keyword(rope.char(end));
    let mut token = String::with_capacity(range.len().min(MAX_WORD_BYTES));
    let mut work = IndexWork::default();
    let mut quantum = 0usize;
    for character in rope.slice(start..end).chars() {
        work.scanned_bytes += character.len_utf8();
        quantum += character.len_utf8();
        if quantum >= 4096 {
            quantum = 0;
            if cancelled() {
                return Err(IndexError::Cancelled);
            }
        }
        if is_keyword(character) {
            if !omitted && token.len() + character.len_utf8() <= MAX_WORD_BYTES {
                token.push(character);
            } else {
                omitted = true;
            }
        } else {
            if omitted {
                word(None)?;
            } else if !token.is_empty() {
                word(Some(&token))?;
            }
            token.clear();
            omitted = false;
        }
    }
    if cancelled() {
        return Err(IndexError::Cancelled);
    }
    if omitted || continued {
        word(None)?;
    } else if !token.is_empty() {
        word(Some(&token))?;
    }
    Ok(work)
}

pub(super) fn boundary(rope: &Rope, byte: usize) -> Result<usize, IndexError> {
    let character = rope
        .try_byte_to_char(byte)
        .map_err(|_| IndexError::InvalidGeometry)?;
    if rope.char_to_byte(character) != byte {
        return Err(IndexError::InvalidGeometry);
    }
    Ok(character)
}

/// At most one eligible word on each boundary. A longer token cannot be a
/// candidate; `visit` recognizes cropped token edges instead of indexing them.
pub(super) fn expand(rope: &Rope, range: Range<usize>) -> Result<Range<usize>, IndexError> {
    let start = boundary(rope, range.start)?;
    let end = boundary(rope, range.end)?;
    if start > end {
        return Err(IndexError::InvalidGeometry);
    }
    let mut result = range;
    let mut before = rope.chars_at(start);
    let mut expanded = 0usize;
    while let Some(character) = before.prev() {
        if !is_keyword(character) || expanded + character.len_utf8() > MAX_WORD_BYTES {
            break;
        }
        expanded += character.len_utf8();
        result.start -= character.len_utf8();
    }
    let mut after = rope.chars_at(end);
    expanded = 0;
    for character in &mut after {
        if !is_keyword(character) || expanded + character.len_utf8() > MAX_WORD_BYTES {
            break;
        }
        expanded += character.len_utf8();
        result.end += character.len_utf8();
    }
    Ok(result)
}
