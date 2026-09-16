use super::attach::{AttachDecision, AttachRecord};
use super::*;
use crate::editor::{document::Document, events::AppEvent, Key};
use strop_core::worker::WorkerId;
use strop_core::{
    id::{BufferRevision, ByteColumn, LineIndex},
    Buffer,
};
use strop_lsp::protocol::{Diag, ResolvedDiag, Severity};
use strop_lsp::{
    LspEvent, PositionEncoding, ReplyContext, RequestId, RequestKind, RequestStamp, ServerColumn,
    ServerId, ServerPosition, WireVersion,
};

fn editor(text: &str) -> Editor {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some(PathBuf::from("/workspace/origin.txt"));
    Editor::new_in(buffer, PathBuf::from("/workspace"))
}

fn attach_key(e: &Editor, language: &str) -> super::attach::AttachKey {
    super::attach::AttachKey {
        target: strop_workspace::Filesystem::Local,
        language: language.into(),
        path: e.buf().path.clone().unwrap(),
    }
}

fn diag_key(path: &std::path::Path) -> strop_workspace::ResourceLocation {
    strop_workspace::ResourceLocation::local(path.to_path_buf())
}

fn arm(e: &mut Editor, id: u64, kind: RequestKind, encoding: PositionEncoding) -> ReplyContext {
    let document = e.current();
    let revision = e.buf().revision();
    let server = ServerId::new(7);
    let path = e.buf().path.clone().unwrap();
    e.lsp_state.bindings.insert(
        document,
        state::Binding {
            server,
            revision,
            path,
            root: PathBuf::from("/workspace"),
            language: "rust".into(),
            target: strop_workspace::Filesystem::Local,
        },
    );
    let stamp = RequestStamp {
        request: RequestId::new(id),
        server,
        document,
        revision,
    };
    if kind == RequestKind::Hover {
        e.lsp_state.hover = Some(stamp);
    } else {
        e.lsp_state.navigation = Some(stamp);
    }
    ReplyContext {
        stamp,
        encoding,
        kind,
    }
}

fn hover(e: &mut Editor, context: ReplyContext, text: &str) {
    e.handle_app_event(AppEvent::Lsp(LspEvent::HoverText {
        context,
        text: text.into(),
    }));
}

fn diagnostics(
    e: &mut Editor,
    context: ReplyContext,
    encoding: PositionEncoding,
    diags: Vec<Diag>,
) {
    let path = e.buf().path.clone().unwrap();
    e.handle_app_event(AppEvent::Lsp(LspEvent::Diagnostics {
        context: strop_lsp::DiagnosticContext {
            server: context.stamp.server,
            document: context.stamp.document,
            revision: context.stamp.revision,
            encoding,
            version: Some(WireVersion::new(1)),
        },
        doc: diag_key(&path),
        diags,
    }));
}

#[test]
fn moving_the_cursor_revokes_a_pending_hover() {
    let mut editor = editor("abc\n");
    let context = arm(&mut editor, 0, RequestKind::Hover, PositionEncoding::Utf8);
    editor.feed(Key::Char('l'));
    hover(&mut editor, context, "old cursor documentation");
    assert!(editor.hover_card.is_none());
}

#[test]
fn older_hover_cannot_overwrite_newer_reply_at_revision_zero() {
    let mut e = editor("let x = 1;\n");
    assert_eq!(e.buf().revision(), BufferRevision::new(0));
    let old = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    let new = arm(&mut e, 1, RequestKind::Hover, PositionEncoding::Utf8);
    hover(&mut e, new, "new answer");
    hover(&mut e, old, "old answer");
    assert_eq!(e.hover_card.as_deref(), Some("new answer"));
}

#[test]
fn equal_revision_documents_do_not_share_hover_ownership() {
    let mut e = editor("a\n");
    e.feed_text("ix");
    e.feed(Key::Esc);
    let old = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    let mut buffer = Buffer::from_text("b\n");
    buffer.path = Some(PathBuf::from("/workspace/other.txt"));
    let other = e.docs.insert(Document::new(buffer));
    e.switch_to(other);
    e.feed_text("iy");
    e.feed(Key::Esc);
    let new = arm(&mut e, 1, RequestKind::Hover, PositionEncoding::Utf8);
    assert_eq!(old.stamp.revision, new.stamp.revision);
    assert_ne!(old.stamp.document, new.stamp.document);
    hover(&mut e, old, "wrong document");
    assert!(e.hover_card.is_none());
    hover(&mut e, new, "right document");
    assert_eq!(e.hover_card.as_deref(), Some("right document"));
}

#[test]
fn a_hover_card_arriving_mid_insert_swallows_no_keystroke() {
    // field report: "esc from insert sometimes needs a second press" —
    // an async hover reply opened its card over insert mode and the next
    // key died dismissing it. Now the card dismisses AND the key lands.
    let mut e = editor("a\n");
    e.feed_text("i");
    assert_eq!(e.mode, crate::editor::Mode::Insert);
    e.hover_card = Some("late answer".into());
    e.feed(Key::Esc);
    assert!(e.hover_card.is_none(), "card dismissed");
    assert_eq!(
        e.mode,
        crate::editor::Mode::Normal,
        "esc still exits insert"
    );
    // a typed char dismisses the card and still inserts
    e.feed_text("i");
    e.hover_card = Some("late again".into());
    e.feed_text("z");
    assert_eq!(e.buf().text().to_string(), "za\n");
    assert!(e.hover_card.is_none());
}

#[test]
fn revision_zero_is_valid_but_does_not_bypass_edit_rejection() {
    let mut e = editor("a\n");
    let zero = arm(&mut e, 0, RequestKind::SwitchHeader, PositionEncoding::Utf8);
    e.handle_lsp_event(LspEvent::Note {
        context: zero,
        text: "valid zero".into(),
    });
    assert_eq!(e.message, "valid zero");
    let stale = arm(&mut e, 1, RequestKind::SwitchHeader, PositionEncoding::Utf8);
    e.feed_text("ix");
    e.feed(Key::Esc);
    assert_ne!(e.buf().revision(), stale.stamp.revision);
    e.message = "after edit".into();
    e.handle_lsp_event(LspEvent::Note {
        context: stale,
        text: "stale zero".into(),
    });
    assert_eq!(e.message, "after edit");
}

#[test]
fn close_reopen_same_path_rejects_old_incarnation_reply() {
    let mut e = editor("old disk text\n");
    let old = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    let path = e.buf().path.clone().unwrap();
    let mut other = Buffer::from_text("keep editor alive\n");
    other.path = Some(PathBuf::from("/workspace/other.txt"));
    e.docs.insert(Document::new(other));
    assert!(e.close_buffer(true));
    let mut reopened = Buffer::from_text("externally replaced content\n");
    reopened.path = Some(path);
    let replacement = e.docs.insert(Document::new(reopened));
    e.switch_to(replacement);
    let new = arm(&mut e, 1, RequestKind::Hover, PositionEncoding::Utf8);
    assert_ne!(old.stamp.document, new.stamp.document);
    assert_eq!(old.stamp.revision, new.stamp.revision);
    hover(&mut e, old, "closed document");
    assert!(e.hover_card.is_none());
    hover(&mut e, new, "fresh document");
    assert_eq!(e.hover_card.as_deref(), Some("fresh document"));
    assert_eq!(e.buf().text().to_string(), "externally replaced content\n");
}

#[test]
fn old_server_reply_is_not_accepted_by_replacement_binding() {
    let mut e = editor("a\n");
    let old = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    e.lsp_state
        .bindings
        .get_mut(&old.stamp.document)
        .unwrap()
        .server = ServerId::new(8);
    hover(&mut e, old, "dead connection");
    assert!(e.hover_card.is_none());
}

/// A cpp binding at a custom root, mirroring arm().
fn arm_cpp(e: &mut Editor, id: u64, server_id: u64, root: &str) -> ReplyContext {
    let document = e.current();
    let revision = e.buf().revision();
    let server = ServerId::new(server_id);
    let path = e.buf().path.clone().unwrap();
    e.lsp_state.bindings.insert(
        document,
        state::Binding {
            server,
            revision,
            path,
            root: PathBuf::from(root),
            language: "cpp".into(),
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
        kind: RequestKind::Goto,
    }
}

fn cpp_editor() -> Editor {
    let mut buffer = Buffer::from_text("#include <vector.hpp>\n");
    buffer.path = Some(PathBuf::from("/proj/main.cpp"));
    Editor::new_in(buffer, PathBuf::from("/proj"))
}

fn insert_target(e: &mut Editor, path: &str, text: &str) -> strop_core::id::DocumentId {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some(PathBuf::from(path));
    e.docs.insert(Document::new(buffer))
}

fn at_origin() -> ServerPosition {
    ServerPosition {
        line: LineIndex::new(0),
        column: ServerColumn::new(0),
    }
}

/// window/showMessage reaches the modeline from its owning server;
/// a message for an unknown server is refused, not shown.
#[test]
fn server_message_reaches_the_modeline_from_its_owner() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let server = ServerId::new(6);
    e.lsp_servers.push(LspServer {
        id: server,
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: true,
    });
    e.handle_lsp_event(LspEvent::ServerMessage {
        server,
        name: "pyright".into(),
        text: "stubPath is not a valid directory".into(),
    });
    assert_eq!(e.message, "lsp: pyright: stubPath is not a valid directory");
    e.message = "later state".into();
    e.handle_lsp_event(LspEvent::ServerMessage {
        server: ServerId::new(99),
        name: "ghost".into(),
        text: "stale".into(),
    });
    assert_eq!(e.message, "later state");
}

#[test]
fn resolved_diag_round_trips_serde() {
    let diag = ResolvedDiag {
        line: LineIndex::new(2),
        col: ByteColumn::new(7),
        end_line: LineIndex::new(2),
        end_col: ByteColumn::new(11),
        severity: Severity::Warning,
        message: "unused".into(),
    };
    let bytes = serde_json::to_vec(&diag).unwrap();
    let back: ResolvedDiag = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(diag, back);
}

mod remote;

mod navigation;

mod diagnostics;

mod attachment;

mod warm_up {
    use super::super::attach::{AttachDecision, AttachKey, Attachment};
    use super::*;
    use strop_workspace::Filesystem;

    fn warm_editor(dir: &std::path::Path) -> Editor {
        let mut editor = Editor::new_in(Buffer::from_text(""), dir.to_path_buf());
        editor.lsp_state.attach.enabled = true;
        editor.open_picker(strop_picker::Kind::WorkspaceSymbols);
        editor
    }

    fn rust_key(root: &std::path::Path) -> AttachKey {
        AttachKey {
            target: Filesystem::Local,
            language: "rust".into(),
            path: root.to_path_buf(),
        }
    }

    #[test]
    fn marker_families_map_to_languages_and_ambiguity_stays_cold() {
        use super::super::lifecycle::warm_language;
        assert_eq!(warm_language("Cargo.toml"), Some(("rust", ".rs")));
        assert_eq!(warm_language("pyproject.toml"), Some(("python", ".py")));
        assert_eq!(warm_language("setup.py"), Some(("python", ".py")));
        assert_eq!(warm_language("CMakeLists.txt"), Some(("cpp", ".cpp")));
        assert_eq!(warm_language("go.mod"), Some(("go", ".go")));
        assert_eq!(warm_language("package.json"), None);
    }

    #[test]
    fn warm_up_requires_services_and_the_symbols_picker() {
        let dir = tempfile::tempdir().unwrap();
        // Services never started: nothing queues, nothing spawns.
        let mut cold = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
        cold.open_picker(strop_picker::Kind::WorkspaceSymbols);
        cold.lsp_warm_scope_projects(vec![(dir.path().join("p"), "Cargo.toml".into())]);
        assert!(cold.lsp_state.attach.warm_queue.is_empty());
        let mut warm = warm_editor(dir.path());
        // An ambiguous marker queues but maps to no language: the
        // queue drains past it with nothing in flight.
        warm.lsp_warm_scope_projects(vec![(dir.path().join("web"), "package.json".into())]);
        assert!(warm.lsp_state.attach.warm_queue.is_empty());
        assert!(warm.lsp_state.attach.pending.is_empty());
        // A project already pending discovery is not rediscovered:
        // the candidate drains, the pending key stays exactly one.
        let root = dir.path().join("p");
        let key = rust_key(&root);
        let ticket = WorkerId::new(77);
        warm.lsp_state.attach.pending.insert(key.clone(), ticket);
        warm.lsp_warm_scope_projects(vec![(root, "Cargo.toml".into())]);
        assert!(warm.lsp_state.attach.warm_queue.is_empty());
        assert_eq!(
            warm.lsp_state.attach.pending.get(&key),
            Some(&ticket),
            "the live attempt is neither duplicated nor superseded"
        );
        // Installing another surface ends warm-up entirely.
        warm.lsp_state
            .attach
            .warm_queue
            .push_back((dir.path().join("q"), "Cargo.toml".into()));
        warm.open_picker(strop_picker::Kind::Files);
        assert!(
            warm.lsp_state.attach.warm_queue.is_empty(),
            "installing another surface ends warm-up"
        );
    }

    #[test]
    fn live_placements_and_sticky_refusals_are_never_rediscovered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("engine");
        let mut editor = warm_editor(dir.path());
        // An attachment already serves this (language, root): warm-up
        // must not rediscover or supersede it.
        editor.lsp_state.attach.attached.push(Attachment {
            language: "rust".into(),
            root: root.clone(),
            server: ServerId::new(31),
            target: Filesystem::Local,
        });
        editor.lsp_warm_scope_projects(vec![(root.clone(), "Cargo.toml".into())]);
        assert!(editor.lsp_state.attach.warm_queue.is_empty());
        assert!(editor.lsp_state.attach.pending.is_empty());
        // A sticky refusal for another project is respected — no spawn,
        // the queue drains past it.
        let other = dir.path().join("tools");
        editor.lsp_state.attach.refused.insert(
            AttachKey {
                target: Filesystem::Local,
                language: "rust".into(),
                path: other.clone(),
            },
            AttachDecision::NoServer,
        );
        editor.lsp_warm_scope_projects(vec![(other, "Cargo.toml".into())]);
        assert!(editor.lsp_state.attach.pending.is_empty());
    }

    #[test]
    fn ambiguous_marker_records_a_cold_status_row() {
        use crate::editor::picker::status::{ProjectStatus, ProjectStatusKind};
        let dir = tempfile::tempdir().unwrap();
        let mut editor = warm_editor(dir.path());
        let web = dir.path().join("web");
        editor.lsp_warm_scope_projects(vec![(web.clone(), "package.json".into())]);
        let glue = editor.picker.as_ref().unwrap();
        assert_eq!(
            glue.project_statuses,
            vec![(
                web.clone(),
                ProjectStatus {
                    kind: ProjectStatusKind::Ambiguous,
                    reason: glue.project_statuses[0].1.reason.clone(),
                }
            )],
            "the ambiguous project records exactly one cold status"
        );
        assert!(
            glue.project_statuses[0].1.reason.contains("package.json"),
            "the reason names the ambiguous marker"
        );
        // The row is the pinned tail: chip + `name  path · reason`,
        // never filtered, never a symbol candidate.
        assert_eq!(glue.picker.pinned_tail, 1);
        let row = glue.picker.items.iter().last().unwrap();
        assert_eq!(row.badge.as_deref(), Some("cold"));
        assert!(row.text.starts_with("web  "), "{row:?}");
        assert!(row.text.contains("· "), "{row:?}");
        assert!(
            matches!(row.payload, strop_picker::Payload::ProjectStatus(_)),
            "status rows are never symbol locations"
        );
    }

    #[test]
    fn sticky_refusal_records_its_status_row() {
        use crate::editor::picker::status::ProjectStatusKind;
        let dir = tempfile::tempdir().unwrap();
        let mut editor = warm_editor(dir.path());
        let root = dir.path().join("tools");
        editor.lsp_state.attach.refused.insert(
            AttachKey {
                target: Filesystem::Local,
                language: "rust".into(),
                path: root.clone(),
            },
            AttachDecision::NoServer,
        );
        editor.lsp_warm_scope_projects(vec![(root.clone(), "Cargo.toml".into())]);
        let glue = editor.picker.as_ref().unwrap();
        assert_eq!(glue.project_statuses.len(), 1);
        assert_eq!(glue.project_statuses[0].0, root);
        assert_eq!(glue.project_statuses[0].1.kind, ProjectStatusKind::NoServer);
        assert!(
            glue.project_statuses[0].1.reason.contains("rust"),
            "the refusal reason is actionable"
        );
        assert!(
            editor.lsp_state.attach.pending.is_empty(),
            "sticky refusals are not rediscovered"
        );
    }

    #[test]
    fn surface_change_clears_project_statuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = warm_editor(dir.path());
        editor.lsp_warm_scope_projects(vec![(dir.path().join("web"), "package.json".into())]);
        assert_eq!(editor.picker.as_ref().unwrap().project_statuses.len(), 1);
        // An in-flight warm attempt's tracking dies with the surface too.
        let key = AttachKey {
            target: Filesystem::Local,
            language: "rust".into(),
            path: dir.path().join("api"),
        };
        editor
            .lsp_state
            .attach
            .pending
            .insert(key.clone(), WorkerId::new(9));
        editor.lsp_state.attach.warm_attempts.insert(key);
        editor.open_picker(strop_picker::Kind::Files);
        assert!(editor.lsp_state.attach.warm_queue.is_empty());
        assert!(editor.lsp_state.attach.warm_attempts.is_empty());
        let glue = editor.picker.as_ref().unwrap();
        assert!(glue.project_statuses.is_empty());
        assert!(glue
            .picker
            .items
            .iter()
            .all(|item| !matches!(item.payload, strop_picker::Payload::ProjectStatus(_))));
    }

    #[test]
    fn warm_completion_records_no_server_status_row() {
        use crate::editor::picker::status::ProjectStatusKind;
        let dir = tempfile::tempdir().unwrap();
        let mut editor = warm_editor(dir.path());
        let root = dir.path().join("api");
        // The NoServer fixture shape (attach.rs): local target, the
        // project root, no layer diagnostics.
        let key = AttachKey {
            target: Filesystem::Local,
            language: "nosuchlanguage".into(),
            path: root.clone(),
        };
        let ticket = WorkerId::new(41);
        editor.lsp_state.attach.pending.insert(key.clone(), ticket);
        editor.lsp_state.attach.warm_attempts.insert(key);
        editor.handle_lsp_attach(AttachRecord {
            ticket,
            server: None,
            language: "nosuchlanguage".into(),
            name: "nosuchlanguage".into(),
            root: root.clone(),
            target: Filesystem::Local,
            outcome: AttachDecision::NoServer,
            layers: Vec::new(),
        });
        // The transient status line keeps its exact behavior.
        assert_eq!(editor.message, "no language server for nosuchlanguage");
        let glue = editor.picker.as_ref().unwrap();
        assert_eq!(glue.project_statuses.len(), 1);
        assert_eq!(glue.project_statuses[0].0, root);
        assert_eq!(glue.project_statuses[0].1.kind, ProjectStatusKind::NoServer);
        let row = glue.picker.items.iter().last().unwrap();
        assert_eq!(row.badge.as_deref(), Some("no srv"));
        assert!(row.text.contains("api  "), "{row:?}");
        assert!(row.text.contains("no language server"), "{row:?}");
        // A document attach completion (no warm-attempt tracking)
        // never records a project status.
        let doc_key = AttachKey {
            target: Filesystem::Local,
            language: "rust".into(),
            path: dir.path().join("opened.rs"),
        };
        let doc_ticket = WorkerId::new(42);
        editor.lsp_state.attach.pending.insert(doc_key, doc_ticket);
        editor.handle_lsp_attach(AttachRecord {
            ticket: doc_ticket,
            server: None,
            language: "rust".into(),
            name: "rust-analyzer".into(),
            root: dir.path().to_path_buf(),
            target: Filesystem::Local,
            outcome: AttachDecision::NoServer,
            layers: Vec::new(),
        });
        assert_eq!(
            editor.picker.as_ref().unwrap().project_statuses.len(),
            1,
            "only warm-up completions record project statuses"
        );
    }

    #[test]
    fn status_row_accepts_as_directory_never_a_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        let mut editor = warm_editor(dir.path());
        let web = dir.path().join("web");
        editor.lsp_warm_scope_projects(vec![(web.clone(), "package.json".into())]);
        editor.wait_picker();
        {
            let glue = editor.picker.as_ref().unwrap();
            let row = glue
                .picker
                .current()
                .expect("the status row survives ranking");
            assert_eq!(row.badge.as_deref(), Some("cold"));
            assert!(row.text.contains("web"), "{row:?}");
            assert!(row.text.contains('·'), "{row:?}");
        }
        let jumps = editor.jumplist_past.len();
        editor.accept_current_picker();
        editor.wait_io().unwrap();
        assert_eq!(
            editor.directory().unwrap().location.path,
            web,
            "Enter browses the project root"
        );
        assert_eq!(
            editor.jumplist_past.len(),
            jumps,
            "a status row never lands as a symbol jump"
        );
    }
}
