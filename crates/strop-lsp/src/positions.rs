//! Mutation coordinates reject invalid lines, columns and encoded-character
//! boundaries. Navigation's deliberately clamping conversion is not an edit API.
use crate::{PositionEncoding, ServerPosition};
use ropey::Rope;
use strop_core::id::ByteOffset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PositionError {
    #[error("language-server edit names a nonexistent source line")]
    Line,
    #[error("language-server edit column is outside the source line")]
    Column,
    #[error("language-server edit splits an encoded character")]
    CharacterBoundary,
}

pub fn checked_byte_offset(
    text: &Rope,
    position: ServerPosition,
    encoding: PositionEncoding,
) -> Result<ByteOffset, PositionError> {
    let index = position.line.get();
    let line = text.get_line(index).ok_or(PositionError::Line)?;
    let mut characters = line.len_chars();
    if characters > 0 && line.char(characters - 1) == '\n' {
        characters -= 1;
    }
    if characters > 0 && line.char(characters - 1) == '\r' {
        characters -= 1;
    }
    let line = line.slice(..characters);
    let wanted = position.column.get();
    let start = text.line_to_byte(index);
    if encoding == PositionEncoding::Utf8 {
        let character = line
            .try_byte_to_char(wanted)
            .map_err(|_| PositionError::Column)?;
        if line.char_to_byte(character) != wanted {
            return Err(PositionError::CharacterBoundary);
        }
        return Ok(ByteOffset::new(start + wanted));
    }
    // Ropey's subtree UTF-16 counters keep even a megabyte line logarithmic.
    // Its conversion rounds a split surrogate down, so validate the inverse.
    let character = line
        .try_utf16_cu_to_char(wanted)
        .map_err(|_| PositionError::Column)?;
    if line.char_to_utf16_cu(character) != wanted {
        return Err(PositionError::CharacterBoundary);
    }
    Ok(ByteOffset::new(start + line.char_to_byte(character)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerColumn;
    use strop_core::id::LineIndex;

    fn at(line: usize, column: usize) -> ServerPosition {
        ServerPosition {
            line: LineIndex::new(line),
            column: ServerColumn::new(column),
        }
    }

    #[test]
    fn edits_reject_half_characters_and_columns_inside_crlf() {
        let source = Rope::from_str("a😀z\r\nβ\n");
        assert_eq!(
            checked_byte_offset(&source, at(0, 3), PositionEncoding::Utf16),
            Ok(ByteOffset::new(5))
        );
        assert_eq!(
            checked_byte_offset(&source, at(0, 2), PositionEncoding::Utf16),
            Err(PositionError::CharacterBoundary)
        );
        assert_eq!(
            checked_byte_offset(&source, at(0, 2), PositionEncoding::Utf8),
            Err(PositionError::CharacterBoundary)
        );
        assert_eq!(
            checked_byte_offset(&source, at(0, 7), PositionEncoding::Utf8),
            Err(PositionError::Column)
        );
        assert_eq!(
            checked_byte_offset(&source, at(0, 5), PositionEncoding::Utf16),
            Err(PositionError::Column)
        );
        assert_eq!(
            checked_byte_offset(&source, at(1, 1), PositionEncoding::Utf16),
            Ok(ByteOffset::new(10))
        );
        assert_eq!(
            checked_byte_offset(&source, at(2, 0), PositionEncoding::Utf16),
            Ok(ByteOffset::new(11))
        );
        assert_eq!(
            checked_byte_offset(&source, at(3, 0), PositionEncoding::Utf16),
            Err(PositionError::Line)
        );
    }

    #[test]
    fn utf16_edits_remain_exact_across_rope_chunks_and_nonzero_line_origins() {
        let source = Rope::from_str(&format!("Ω\r\n{}\r\n", "😀β".repeat(4096)));
        assert_eq!(
            checked_byte_offset(&source, at(1, 4095 * 3), PositionEncoding::Utf16),
            Ok(ByteOffset::new(4 + 4095 * 6))
        );
        assert_eq!(
            checked_byte_offset(&source, at(1, 4095 * 3 + 1), PositionEncoding::Utf16),
            Err(PositionError::CharacterBoundary)
        );
        assert_eq!(
            checked_byte_offset(&source, at(1, 4096 * 3), PositionEncoding::Utf16),
            Ok(ByteOffset::new(4 + 4096 * 6))
        );
        assert_eq!(
            checked_byte_offset(&source, at(1, 4096 * 3 + 1), PositionEncoding::Utf16),
            Err(PositionError::Column)
        );
    }
}
