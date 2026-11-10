//! What the differential harness cannot see: messages, listings, more
//! than one buffer, the confirm question, the project root.

mod buffers;
mod commands;
mod confirm;
mod directory;

use std::path::Path;

use strop_core::id::DocumentId;
use strop_core::Buffer;

use crate::editor::{Document, Editor};

fn editor(text: &str) -> Editor {
    Editor::new_in(Buffer::from_text(text), "/project".into())
}

fn text(editor: &Editor) -> String {
    editor.buf().text().to_string()
}

/// One more file buffer, as if opened: named, not shown.
fn add_file(editor: &mut Editor, path: &Path, text: &str) -> DocumentId {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some(path.to_owned());
    let id = editor.docs.try_insert(Document::new(buffer)).unwrap();
    editor.mru.push(id);
    id
}

/// An editor on `/project/a.txt` with `b.txt` and `src/lib.rs` open too.
fn three_buffers() -> (Editor, [DocumentId; 3]) {
    let mut buffer = Buffer::from_text("alpha\n");
    buffer.path = Some("/project/a.txt".into());
    let mut editor = Editor::new_in(buffer, "/project".into());
    let a = editor.current();
    let b = add_file(&mut editor, Path::new("/project/b.txt"), "beta\n");
    let c = add_file(&mut editor, Path::new("/project/src/lib.rs"), "gamma\n");
    (editor, [a, b, c])
}
