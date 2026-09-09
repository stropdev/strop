//! One line-local scalar search for f/F/t/T and their repeats. No full-line copy.
use strop_core::{id::ByteOffset, Buffer};

pub fn find_character(
    buffer: &Buffer,
    cursor: ByteOffset,
    character: char,
    backward: bool,
    count: usize,
) -> Option<ByteOffset> {
    let line = buffer.line_of(cursor);
    let start = buffer.line_start(line);
    let end = buffer.line_end(line);
    let char_at = |position| buffer.text().char(buffer.text().byte_to_char(position));
    let mut position = buffer.clamp_boundary(cursor.get().min(end));
    let mut matched = 0;
    if backward {
        while position > start {
            position = buffer.clamp_boundary(position - 1);
            if char_at(position) == character {
                matched += 1;
                if matched == count {
                    return Some(ByteOffset::new(position));
                }
            }
        }
    } else {
        if position < end {
            position += char_at(position).len_utf8();
        }
        while position < end {
            let current = char_at(position);
            if current == character {
                matched += 1;
                if matched == count {
                    return Some(ByteOffset::new(position));
                }
            }
            position += current.len_utf8();
        }
    }
    None
}
