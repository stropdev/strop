//! Filter ranges are server-owned when textEdit is present. Conversion walks
//! only the bounded prefix before the already-negotiated request position.
use super::{CompletionDecodeError, CompletionItem, CompletionTextEdit, MAX_FILTER_BYTES};
use crate::{PositionEncoding, RequestInput, ServerColumn};
use ropey::RopeSlice;
use strop_core::id::ByteColumn;

pub struct CompletionFilter<'a> {
    line: usize,
    text: RopeSlice<'a>,
    caret: usize,
    caret_character: usize,
    server_caret: usize,
    prefix_start: usize,
    encoding: PositionEncoding,
}
impl<'a> CompletionFilter<'a> {
    /// `server_caret` is the position already computed by the ordered wire's
    /// canonical encoder; do not rescan a megabyte line for every result item.
    pub fn new(
        input: &'a RequestInput,
        prefix_start: ByteColumn,
        server_caret: ServerColumn,
        encoding: PositionEncoding,
    ) -> Result<Self, CompletionDecodeError> {
        let text = input.line_text.as_slice();
        let caret = input.byte_col.get();
        let start = prefix_start.get();
        let caret_character = text
            .try_byte_to_char(caret)
            .map_err(|_| CompletionDecodeError::FilterPosition)?;
        let start_character = text
            .try_byte_to_char(start)
            .map_err(|_| CompletionDecodeError::FilterPosition)?;
        if start > caret
            || caret - start > MAX_FILTER_BYTES
            || text.char_to_byte(caret_character) != caret
            || text.char_to_byte(start_character) != start
        {
            return Err(CompletionDecodeError::FilterPosition);
        }
        Ok(Self {
            line: input.line.get(),
            text,
            caret,
            caret_character,
            server_caret: server_caret.get(),
            prefix_start: start,
            encoding,
        })
    }

    pub(super) fn matches(&self, item: &CompletionItem) -> Result<bool, CompletionDecodeError> {
        let start = match &item.protocol.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => Some(edit.range.start),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => Some(edit.replace.start),
            None => None,
        };
        let byte = match start {
            None => self.prefix_start,
            Some(position) if position.line as usize == self.line => {
                self.byte_start(position.character as usize)?
            }
            Some(_) => return Err(CompletionDecodeError::FilterPosition),
        };
        let prefix = self.text.byte_slice(byte..self.caret);
        Ok(item.matches_prefix(prefix.chars()))
    }

    fn byte_start(&self, column: usize) -> Result<usize, CompletionDecodeError> {
        let mut units = self
            .server_caret
            .checked_sub(column)
            .ok_or(CompletionDecodeError::FilterPosition)?;
        if self.encoding == PositionEncoding::Utf8 {
            let start = self
                .caret
                .checked_sub(units)
                .ok_or(CompletionDecodeError::FilterPosition)?;
            let character = self
                .text
                .try_byte_to_char(start)
                .map_err(|_| CompletionDecodeError::FilterPosition)?;
            return if units <= MAX_FILTER_BYTES && self.text.char_to_byte(character) == start {
                Ok(start)
            } else {
                Err(CompletionDecodeError::FilterPosition)
            };
        }
        let mut byte = self.caret;
        let mut previous = self.text.chars_at(self.caret_character);
        while units > 0 {
            let character = previous
                .prev()
                .ok_or(CompletionDecodeError::FilterPosition)?;
            units = units
                .checked_sub(character.len_utf16())
                .ok_or(CompletionDecodeError::FilterPosition)?;
            byte -= character.len_utf8();
            if self.caret - byte > MAX_FILTER_BYTES {
                return Err(CompletionDecodeError::FilterPosition);
            }
        }
        Ok(byte)
    }
}
