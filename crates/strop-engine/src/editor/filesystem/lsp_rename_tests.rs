//! The staged will-rename choreography end to end inside the editor:
//! a parked review resumes when the language server replies, its
//! preparation edits apply to the open buffer, the mutation commits,
//! and a cancelled stage applies nothing. The wire half of the
//! exchange is covered in strop-lsp's client tests.
use super::tests::{command, fixture};
use super::*;
use crate::editor::lsp::state::Binding;
use strop_lsp::{
    LspEvent, PositionEncoding, ReplyContext, RequestId, RequestKind, RequestStamp, ServerColumn,
    ServerEdit, ServerId, ServerPosition,
};
use strop_workspace::Filesystem;

fn staged_rename(
    root: &tempfile::TempDir,
    editor: &mut Editor,
    document: DocumentId,
) -> RequestStamp {
    let proposal = editor
        .filesystem
        .pending
        .take()
        .expect("review prepared a pending proposal");
    let server = ServerId::new(7);
    let revision = editor.doc(document).buf.revision();
    editor.lsp_state.bindings.insert(
        document,
        Binding {
            server,
            path: root.path().join("original.txt"),
            root: root.path().to_path_buf(),
            language: "rust".into(),
            target: Filesystem::Local,
            revision,
        },
    );
    let stamp = RequestStamp {
        request: RequestId::new(1),
        server,
        document,
        revision,
    };
    editor.filesystem.will_rename = Some(WillRenameStage {
        proposal,
        stamps: vec![stamp],
        servers: vec![(server, Vec::new())],
    });
    stamp
}

#[test]
fn will_rename_reply_applies_preparation_edits_then_commits_the_rename() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    let renamed = root.path().join("renamed.txt");
    std::fs::write(&original, "import a;\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    command(&mut editor, ":fs rename renamed.txt<cr>");
    let stamp = staged_rename(&root, &mut editor, document);
    editor.handle_lsp_event(LspEvent::WorkspaceEdits {
        context: ReplyContext {
            stamp,
            encoding: PositionEncoding::Utf8,
            kind: RequestKind::WillRenameFiles,
        },
        edits: vec![(
            ResourceLocation::local(original.clone()),
            vec![ServerEdit {
                start: ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: ServerColumn::new(7),
                },
                end: ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: ServerColumn::new(8),
                },
                new_text: "b".into(),
            }],
        )],
    });
    // The reply both applied the preparation edit and resumed the
    // parked filesystem apply (focus sits on the review report).
    assert_eq!(editor.doc(document).buf.text().to_string(), "import b;\n");
    assert!(editor.filesystem.will_rename.is_none());
    editor.wait_io().unwrap();
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "import a;\n");
    // The document follows the rename and still holds the server's edit.
    assert_eq!(editor.doc(document).buf.path.as_ref(), Some(&renamed));
    assert_eq!(editor.doc(document).buf.text().to_string(), "import b;\n");
}

#[test]
fn cancelling_a_staged_will_rename_applies_nothing() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    std::fs::write(&original, "import a;\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    command(&mut editor, ":fs rename renamed.txt<cr>");
    let stamp = staged_rename(&root, &mut editor, document);
    command(&mut editor, ":cancel-change<cr>");
    assert!(editor.filesystem.will_rename.is_none());
    assert!(editor.filesystem.pending.is_none());
    assert!(original.exists());
    assert!(!root.path().join("renamed.txt").exists());
    // The late reply is dead weight: no edit, no dispatch, no panic.
    editor.handle_lsp_event(LspEvent::WorkspaceEdits {
        context: ReplyContext {
            stamp,
            encoding: PositionEncoding::Utf8,
            kind: RequestKind::WillRenameFiles,
        },
        edits: vec![(
            ResourceLocation::local(original.clone()),
            vec![ServerEdit {
                start: ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: ServerColumn::new(0),
                },
                end: ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: ServerColumn::new(1),
                },
                new_text: "X".into(),
            }],
        )],
    });
    assert_eq!(editor.doc(document).buf.text().to_string(), "import a;\n");
    assert!(original.exists());
}
