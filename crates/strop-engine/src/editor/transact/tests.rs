//! Marks through a batch (0108 D1.10): every edit of a batch is judged in
//! the coordinates it addressed, so a batch decides like the same edits made
//! one by one.
use super::ChangeSet;
use crate::editor::Editor;
use strop_core::{Buffer, Range, Replacement};

fn batch(e: &mut Editor, edits: &[(usize, usize, &str)]) {
    let document = e.current();
    let base = e.buf().revision();
    let edits = edits
        .iter()
        .map(|&(start, end, text)| Replacement::new(Range::charwise(start, end), text.to_string()))
        .collect();
    e.apply(
        document,
        base,
        ChangeSet {
            edits,
            undo_open: false,
        },
    )
    .unwrap();
}

#[test]
fn a_mark_goes_with_its_line_in_any_edit_of_a_batch() {
    let mut e = Editor::new(Buffer::from_text("aa\nbb\ncc\n"));
    let document = e.current();
    // 'a' on "cc", 'b' on "bb"
    e.marks.insert('a', (document, 6));
    e.marks.insert('b', (document, 3));
    // delete the first and the last line in one batch
    batch(&mut e, &[(0, 3, ""), (6, 9, "")]);
    assert_eq!(e.buf().text().to_string(), "bb\n");
    assert_eq!(e.marks.get(&'a'), None, "the mark's line was deleted");
    assert_eq!(e.marks.get(&'b'), Some(&(document, 0)));
}

#[test]
fn a_join_in_a_batch_keeps_the_mark_on_the_joined_line() {
    let mut e = Editor::new(Buffer::from_text("aa\nbb\ncc\n"));
    let document = e.current();
    // on the newline ending "bb"
    e.marks.insert('a', (document, 5));
    // delete the first line, and join "bb" with "cc" (a mid-line start)
    batch(&mut e, &[(0, 3, ""), (5, 6, "")]);
    assert_eq!(e.buf().text().to_string(), "bbcc\n");
    assert_eq!(e.marks.get(&'a'), Some(&(document, 2)));
}
