//! Compose sequential journal geometry into disjoint pre-lease replacements.
//! Only inserted bytes are materialized; unchanged rope contents are never copied.
use strop_core::{Buffer, Change, Range, Replacement};

#[derive(Clone, Copy)]
struct Piece {
    original: Option<usize>,
    length: usize,
}

fn split(pieces: &mut Vec<Piece>, position: usize) -> usize {
    let mut offset = 0;
    for index in 0..pieces.len() {
        let piece = pieces[index];
        if position == offset {
            return index;
        }
        if position < offset + piece.length {
            let left = position - offset;
            pieces[index].length = left;
            pieces.insert(
                index + 1,
                Piece {
                    original: piece.original.map(|start| start + left),
                    length: piece.length - left,
                },
            );
            return index + 1;
        }
        offset += piece.length;
    }
    debug_assert_eq!(position, offset);
    pieces.len()
}

pub(super) fn replacements(buffer: &Buffer, changes: &[Change]) -> Vec<Replacement> {
    let original_length = changes
        .iter()
        .rev()
        .fold(buffer.len_bytes(), |length, change| {
            length + change.edit.old_end_byte - change.edit.new_end_byte
        });
    let mut pieces = vec![Piece {
        original: Some(0),
        length: original_length,
    }];
    for change in changes {
        let edit = change.edit;
        let start = split(&mut pieces, edit.start_byte);
        let end = split(&mut pieces, edit.old_end_byte);
        let inserted = edit.new_end_byte - edit.start_byte;
        pieces.splice(
            start..end,
            (inserted != 0).then_some(Piece {
                original: None,
                length: inserted,
            }),
        );
    }
    let mut result = Vec::new();
    let mut original = 0;
    let mut current = 0;
    let mut inserted_start = 0;
    for piece in pieces.into_iter().chain([Piece {
        original: Some(original_length),
        length: 0,
    }]) {
        if let Some(start) = piece.original {
            if original != start || inserted_start != current {
                result.push(Replacement::new(
                    Range::charwise(original, start),
                    buffer
                        .text()
                        .byte_slice(inserted_start..current)
                        .to_string(),
                ));
            }
            original = start + piece.length;
            current += piece.length;
            inserted_start = current;
        } else {
            current += piece.length;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_sequential_edits_preserve_only_the_final_inserted_text() {
        let mut buffer = Buffer::from_text("alpha beta gamma");
        buffer.edit().insert(6, "new ").unwrap();
        buffer.edit().replace(Range::charwise(6, 14), "B").unwrap();
        buffer.edit().insert(0, "!").unwrap();
        let edits = replacements(&buffer, buffer.changes());
        let mut source = Buffer::from_text("alpha beta gamma");
        let prepared = source
            .prepare_replacements(source.revision(), edits)
            .unwrap();
        source.apply_prepared(prepared, false).unwrap();
        assert_eq!(source.text().to_string(), buffer.text().to_string());
        assert_eq!(source.text().to_string(), "!alpha B gamma");
    }

    #[test]
    fn disjoint_reverse_order_edits_keep_pre_edit_coordinates() {
        let mut buffer = Buffer::from_text("one two one");
        buffer
            .edit()
            .replace(Range::charwise(8, 11), "three")
            .unwrap();
        buffer.edit().replace(Range::charwise(0, 3), "1").unwrap();
        let edits = replacements(&buffer, buffer.changes());
        let mut source = Buffer::from_text("one two one");
        let prepared = source
            .prepare_replacements(source.revision(), edits)
            .unwrap();
        source.apply_prepared(prepared, false).unwrap();
        assert_eq!(source.text().to_string(), "1 two three");
    }
}
