//! Change-plan engine: application, refusals, grouped undo — driven
//! through injected LspEvents like the live wire delivers them.

use super::*;
use crate::editor::events::AppEvent;
use crate::editor::Editor;
use std::path::PathBuf;
use strop_core::id::LineIndex;
use strop_core::Buffer;
use strop_lsp::{
    LspEvent, PositionEncoding, ReplyContext, RequestId, RequestKind, RequestStamp, ServerColumn,
    ServerEdit, ServerId, ServerPosition,
};
use strop_workspace::ResourceLocation;

fn at(line: usize, col: usize) -> ServerPosition {
    ServerPosition {
        line: LineIndex::new(line),
        column: ServerColumn::new(col),
    }
}

fn edit(sl: usize, sc: usize, el: usize, ec: usize, text: &str) -> ServerEdit {
    ServerEdit {
        start: at(sl, sc),
        end: at(el, ec),
        new_text: text.into(),
    }
}

fn arm(e: &mut Editor, document: DocumentId, id: u64, kind: RequestKind) -> ReplyContext {
    let server = ServerId::new(7);
    let (revision, path) = {
        let doc = e.docs.get(document).unwrap();
        (doc.buf.revision(), doc.buf.path.clone().unwrap())
    };
    e.lsp_state.bindings.insert(
        document,
        crate::editor::lsp::state::Binding {
            server,
            revision,
            path,
            root: PathBuf::from("/workspace"),
            target: strop_workspace::Filesystem::Local,
        },
    );
    let stamp = RequestStamp {
        request: RequestId::new(id),
        server,
        document,
        revision,
    };
    e.lsp_state.navigation = Some(stamp);
    ReplyContext {
        stamp,
        encoding: PositionEncoding::Utf8,
        kind,
    }
}

fn file_editor(dir: &tempfile::TempDir, name: &str, text: &str) -> (Editor, DocumentId) {
    std::fs::write(dir.path().join(name), text).unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&dir.path().join(name)).unwrap();
    let document = e.current();
    (e, document)
}

#[test]
fn format_applies_edits_and_records_a_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(&dir, "a.txt", "hello   world\n");
    let context = arm(&mut e, document, 1, RequestKind::Format);
    e.handle_app_event(AppEvent::Lsp(LspEvent::Edits {
        context,
        edits: vec![edit(0, 0, 0, 5, "hi")],
    }));
    assert_eq!(e.buf().text().to_string(), "hi   world\n");
    assert_eq!(e.message, "format: applied to 1 buffer(s)");
}

#[test]
fn rename_applies_across_open_documents() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, first) = file_editor(&dir, "a.txt", "alpha here\n");
    let b_path = dir.path().join("b.txt");
    std::fs::write(&b_path, "alpha there\n").unwrap();
    e.open_fixture(&b_path).unwrap();
    let second = e.current();
    let _ = arm(&mut e, first, 1, RequestKind::Rename);
    let context = arm(&mut e, second, 2, RequestKind::Rename);
    let path_a = dir.path().join("a.txt");
    e.handle_app_event(AppEvent::Lsp(LspEvent::WorkspaceEdits {
        context,
        edits: vec![
            (
                ResourceLocation::local(path_a.clone()),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
            (
                ResourceLocation::local(b_path.clone()),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
        ],
    }));
    assert_eq!(e.buf().text().to_string(), "omega there\n");
    assert_eq!(e.message, "rename: applied to 2 buffer(s)");
    // Grouped undo restores both buffers through their receipts.
    e.feed_text(":undo-change\r");
    assert_eq!(e.buf().text().to_string(), "alpha there\n");
    e.switch_to(first);
    assert_eq!(e.buf().text().to_string(), "alpha here\n");
}

#[test]
fn unbound_targets_are_named_refusals_never_silent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(&dir, "a.txt", "alpha here\n");
    let context = arm(&mut e, document, 1, RequestKind::Rename);
    e.handle_app_event(AppEvent::Lsp(LspEvent::WorkspaceEdits {
        context,
        edits: vec![
            (
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
            (
                ResourceLocation::local(dir.path().join("ghost.txt")),
                vec![edit(0, 0, 0, 3, "x")],
            ),
        ],
    }));
    assert_eq!(e.buf().text().to_string(), "omega here\n");
    assert_eq!(
        e.message,
        "rename: 1 buffer(s) applied, 1 target(s) refused"
    );
}

#[test]
fn a_document_edited_since_the_plan_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(&dir, "a.txt", "alpha here\n");
    let _ = arm(&mut e, document, 1, RequestKind::Rename); // bind the document
    let location = ResourceLocation::local(dir.path().join("a.txt"));
    let plan = e.build_change_plan(
        ChangeProducer::Rename,
        vec![(location, vec![edit(0, 0, 0, 5, "omega")])],
        PositionEncoding::Utf8,
    );
    e.feed_text("0rx"); // revision moves past the plan's base
    e.apply_change_plan(plan);
    assert_eq!(e.buf().text().to_string(), "xlpha here\n");
    assert!(e.message.contains("refused"), "{}", e.message);
    assert!(e.message.contains("0 buffer(s) applied"), "{}", e.message);
}

#[test]
fn stale_edit_replies_never_touch_the_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(&dir, "a.txt", "alpha here\n");
    let mut context = arm(&mut e, document, 1, RequestKind::Format);
    context.stamp.revision = BufferRevision::new(99); // not the live revision
    e.handle_app_event(AppEvent::Lsp(LspEvent::Edits {
        context,
        edits: vec![edit(0, 0, 0, 5, "SNEAKY")],
    }));
    assert_eq!(e.buf().text().to_string(), "alpha here\n");
    assert_ne!(e.message, "format: applied to 1 buffer(s)");
}

#[test]
fn grouped_undo_skips_buffers_edited_since_the_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, first) = file_editor(&dir, "a.txt", "alpha here\n");
    let b_path = dir.path().join("b.txt");
    std::fs::write(&b_path, "alpha there\n").unwrap();
    e.open_fixture(&b_path).unwrap();
    let second = e.current();
    let _ = arm(&mut e, first, 1, RequestKind::Rename);
    let context = arm(&mut e, second, 2, RequestKind::Rename);
    e.handle_app_event(AppEvent::Lsp(LspEvent::WorkspaceEdits {
        context,
        edits: vec![
            (
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
            (
                ResourceLocation::local(b_path),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
        ],
    }));
    e.feed_text("0rZ"); // edit b.txt after the rename
    e.feed_text(":undo-change\r");
    assert!(e.message.contains("skipped"), "{}", e.message);
    assert_eq!(e.buf().text().to_string(), "Zmega there\n");
    e.switch_to(first);
    assert_eq!(e.buf().text().to_string(), "alpha here\n");
}
