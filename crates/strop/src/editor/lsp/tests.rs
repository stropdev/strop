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
        target: strop_lsp::FsTarget::Local,
        language: language.into(),
        path: e.buf().path.clone().unwrap(),
    }
}

fn diag_key(path: &std::path::Path) -> strop_lsp::DocPath {
    strop_lsp::DocPath::local(path.to_path_buf())
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
            target: strop_lsp::FsTarget::Local,
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

#[test]
fn goto_completion_uses_target_encoding_and_records_original_jump() {
    let mut e = editor("origin text\n");
    e.feed_text("$");
    let origin = (e.current(), e.head());
    let context = arm(&mut e, 0, RequestKind::Goto, PositionEncoding::Utf8);
    let mut target = Buffer::from_text("a😀z\n");
    target.path = Some(PathBuf::from("/workspace/target.txt"));
    let target = e.docs.insert(Document::new(target));
    e.finish_lsp_jump(
        target,
        ServerPosition {
            line: LineIndex::new(0),
            column: ServerColumn::new(5),
        },
        context,
    );
    assert_eq!(e.current(), target);
    assert_eq!(e.head(), 5);
    e.jump_back();
    assert_eq!((e.current(), e.head()), origin);
}

#[test]
fn navigation_changed_during_load_does_not_switch_or_consume_new_request() {
    let mut e = editor("origin text\n");
    let old = arm(&mut e, 0, RequestKind::Goto, PositionEncoding::Utf8);
    let current = arm(&mut e, 1, RequestKind::Goto, PositionEncoding::Utf8);
    let target = e.docs.insert(Document::new(Buffer::from_text("target\n")));
    let before = (e.current(), e.head());
    e.finish_lsp_jump(
        target,
        ServerPosition {
            line: LineIndex::new(0),
            column: ServerColumn::new(2),
        },
        old,
    );
    assert_eq!((e.current(), e.head()), before);
    assert!(e.lsp_reply_fresh(&current));
}

#[test]
fn diagnostics_resolve_utf8_columns_in_place() {
    let mut e = editor("a😀z\n");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf8,
        vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(5),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(6),
            severity: Severity::Error,
            message: "z".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(
        (diagnostics[0].col.get(), diagnostics[0].end_col.get()),
        (5, 6)
    );
    assert_eq!(diagnostics[0].severity, Severity::Error);
}

#[test]
fn diagnostics_resolve_utf16_columns_against_the_rope() {
    let mut e = editor("a😀z\n");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    // UTF-16 units: a=1, emoji=2, z=1. Position 3 begins z;
    // position 4 is the boundary after z.
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf16,
        vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(3),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(4),
            severity: Severity::Warning,
            message: "mid-emoji".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(
        (diagnostics[0].col.get(), diagnostics[0].end_col.get()),
        (5, 6)
    );
    assert_eq!(diagnostics[0].severity_char(), 'W');
}

#[test]
fn diagnostics_beyond_the_document_clamp_instead_of_panicking() {
    let mut e = editor("single line");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf16,
        vec![Diag {
            line: LineIndex::new(9),
            col: ServerColumn::new(2),
            end_line: LineIndex::new(12),
            end_col: ServerColumn::new(4),
            severity: Severity::Information,
            message: "stale server range".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(diagnostics[0].line.get(), 0);
    assert_eq!(diagnostics[0].end_line.get(), 0);
    // "single line": utf16 == bytes, col 2 stays 2.
    assert_eq!(diagnostics[0].col.get(), 2);
}

#[test]
fn attached_server_diagnostics_survive_full_replay() {
    use crate::editor::trace::{
        drive::{self, Action},
        seed::Seed,
    };
    use strop_trace::replay::{Tape, Tick};
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    e.tape = std::rc::Rc::new(Tape::fixture(|operation, _| match operation {
        "lsp.open" => Ok(serde_json::json!(true)),
        _ => Err(std::io::Error::other("unexpected native observation")),
    }));
    e.tape.seed(&Seed::capture(&e).unwrap()).unwrap();
    e.recorded_action(
        Action::Start {
            directory_picker: false,
            open: None,
        },
        Tick::default(),
    )
    .unwrap();
    let ticket = *e.lsp_state.attach.pending.values().next().unwrap();
    let attach = AttachRecord {
        ticket,
        server: Some(ServerId::new(3)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::Attached,
        layers: Vec::new(),
    };
    e.recorded_action(Action::Event(AppEvent::LspAttach(attach)), Tick::default())
        .unwrap();
    let event = LspEvent::Diagnostics {
        context: strop_lsp::DiagnosticContext {
            server: ServerId::new(3),
            document: e.current(),
            revision: e.buf().revision(),
            encoding: PositionEncoding::Utf8,
            version: Some(WireVersion::new(1)),
        },
        doc: diag_key(Path::new("/workspace/origin.rs")),
        diags: vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(0),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(1),
            severity: Severity::Error,
            message: "source diagnostic".into(),
        }],
    };
    e.recorded_action(Action::Event(AppEvent::Lsp(event)), Tick::default())
        .unwrap();
    assert_eq!(e.diag_counts(e.current()), (1, 0));
    e.tape.finish().unwrap();
    let replayed = drive::replay(e.tape.fixture_nodes()).unwrap();
    assert_eq!(replayed.diag_counts(replayed.current()), (1, 0));
}

#[test]
fn stale_attach_completion_is_refused() {
    let mut e = editor("a\n");
    let current = WorkerId::new(2);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), current);
    e.handle_lsp_attach(AttachRecord {
        ticket: WorkerId::new(1),
        server: Some(ServerId::new(3)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::Attached,
        layers: Vec::new(),
    });
    assert!(e.lsp_servers.is_empty());
    assert_eq!(
        e.lsp_state.attach.pending.get(&attach_key(&e, "rust")),
        Some(&current)
    );
}

#[test]
fn sticky_refusal_reports_once_but_trust_refusals_repeat() {
    let mut e = editor("a\n");
    let first = WorkerId::new(1);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), first);
    e.handle_lsp_attach(AttachRecord {
        ticket: first,
        server: None,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::NotExecutable {
            command: "rust-analyzer".into(),
            reason: "not found on PATH".into(),
            hint: "rustup component add rust-analyzer".into(),
        },
        layers: Vec::new(),
    });
    assert!(e.message.contains("not found on PATH"));
    assert!(e.message.contains("rust-analyzer"));
    assert!(e.message.contains("rustup component add rust-analyzer"));
    let second = WorkerId::new(2);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), second);
    e.message = "later state".into();
    e.handle_lsp_attach(AttachRecord {
        ticket: second,
        server: None,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::NotExecutable {
            command: "rust-analyzer".into(),
            reason: "not found on PATH".into(),
            hint: "rustup component add rust-analyzer".into(),
        },
        layers: Vec::new(),
    });
    assert_eq!(e.message, "later state", "sticky refusals do not repeat");
    // Trust refusals are actionable: they re-report every attempt.
    let third = WorkerId::new(3);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), third);
    e.handle_lsp_attach(AttachRecord {
        ticket: third,
        server: None,
        language: "rust".into(),
        name: "custom-lsp".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::TrustRequired {
            command: "custom-lsp".into(),
        },
        layers: Vec::new(),
    });
    assert!(e.message.contains(":trust"));
}

#[test]
fn attach_skips_unknown_languages_without_discovery() {
    let mut e = editor("plain text\n");
    e.lsp_maybe_attach();
    assert!(e.lsp_state.attach.pending.is_empty());
    assert!(e.lsp_servers.is_empty());
}

#[test]
fn refused_attach_messages_are_reported() {
    let mut e = editor("a\n");
    let ticket = WorkerId::new(9);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), ticket);
    e.handle_lsp_attach(AttachRecord {
        ticket,
        server: None,
        language: "rust".into(),
        name: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::NoServer,
        layers: Vec::new(),
    });
    assert!(e.message.contains("no language server"));
    assert!(e
        .lsp_state
        .attach
        .refused
        .contains_key(&attach_key(&e, "rust")));
}

/// 0033 §2: a malformed layer's diagnostic reaches the modeline with
/// its exact path even though the fallback server attached — and a
/// healthy Ready does not erase it.
#[test]
fn layer_diagnostic_survives_a_healthy_attach_and_ready() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let ticket = WorkerId::new(4);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), ticket);
    e.handle_lsp_attach(AttachRecord {
        ticket,
        server: Some(ServerId::new(5)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::Attached,
        layers: vec![strop_lsp::languages::LayerDiagnostic {
            path: PathBuf::from("/home/u/.config/strop/languages.toml"),
            message: "TOML parse error — layer ignored".into(),
            remote: None,
        }],
    });
    // the fallback server attached and the warning is visible with the
    // actual path
    assert_eq!(e.lsp_servers.len(), 1);
    assert_eq!(e.lsp_state.attach.attached.len(), 1);
    assert!(
        e.message.contains("/home/u/.config/strop/languages.toml"),
        "{}",
        e.message
    );
    assert!(e.message.contains("layer ignored"));
    // a healthy Ready must not erase the configuration warning
    e.handle_lsp_event(LspEvent::Ready {
        server: ServerId::new(5),
        name: "rust-analyzer".into(),
    });
    assert!(e.message.contains("ready"));
    assert!(
        e.message.contains("/home/u/.config/strop/languages.toml"),
        "{}",
        e.message
    );
    // a later attach reporting the same layer does not duplicate it
    let again = WorkerId::new(6);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), again);
    e.handle_lsp_attach(AttachRecord {
        ticket: again,
        server: None,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::NoServer,
        layers: vec![strop_lsp::languages::LayerDiagnostic {
            path: PathBuf::from("/home/u/.config/strop/languages.toml"),
            message: "TOML parse error — layer ignored".into(),
            remote: None,
        }],
    });
    assert_eq!(e.lsp_state.attach.layer_diagnostics.len(), 1);
}

/// 0033 §2: several malformed layers report the first with a count,
/// and a refusal still carries them.
#[test]
fn multiple_layer_diagnostics_report_with_a_count() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let ticket = WorkerId::new(7);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), ticket);
    e.handle_lsp_attach(AttachRecord {
        ticket,
        server: None,
        language: "rust".into(),
        name: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::NoServer,
        layers: vec![
            strop_lsp::languages::LayerDiagnostic {
                path: PathBuf::from("/xdg/languages.toml"),
                message: "bad — layer ignored".into(),
                remote: None,
            },
            strop_lsp::languages::LayerDiagnostic {
                path: PathBuf::from("/proj/.strop/languages.toml"),
                message: "worse — layer ignored".into(),
                remote: None,
            },
        ],
    });
    assert!(e.message.contains("/xdg/languages.toml"), "{}", e.message);
    assert!(e.message.contains("+1 more"), "{}", e.message);
    assert_eq!(e.lsp_state.attach.layer_diagnostics.len(), 2);
}

/// 0033 §3: a spawn failure carries its typed reason to the modeline.
#[test]
fn spawn_failure_refusal_carries_its_reason() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let ticket = WorkerId::new(8);
    e.lsp_state
        .attach
        .pending
        .insert(attach_key(&e, "rust"), ticket);
    e.handle_lsp_attach(AttachRecord {
        ticket,
        server: None,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_lsp::FsTarget::Local,
        outcome: AttachDecision::SpawnFailed {
            reason: "cannot build the LSP runtime: boom".into(),
        },
        layers: Vec::new(),
    });
    assert!(e.message.contains("could not start"), "{}", e.message);
    assert!(e.message.contains("runtime"), "{}", e.message);
    assert!(e
        .lsp_state
        .attach
        .refused
        .contains_key(&attach_key(&e, "rust")));
}

/// 0033 §3: a terminal Failed event reaches the modeline with the
/// executable-bearing hint and tears the server down; a failure for an
/// already-removed server cannot resurrect a message.
#[test]
fn failed_server_event_names_the_command_and_removes_the_server() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let server = ServerId::new(6);
    e.lsp_servers.push(LspServer {
        id: server,
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: true,
    });
    e.handle_lsp_event(LspEvent::Failed {
        server,
        name: "pyright".into(),
        hint: "cannot run `pyright-langserver`: No such file or directory \
               (os error 2) — npm i -g pyright"
            .into(),
    });
    assert!(e.message.contains("pyright-langserver"), "{}", e.message);
    assert!(e.message.contains("npm i -g pyright"), "{}", e.message);
    assert!(e.lsp_servers.is_empty());
    // a duplicate failure for the removed server is refused, not shown
    e.message = "later state".into();
    e.handle_lsp_event(LspEvent::Failed {
        server,
        name: "pyright".into(),
        hint: "stale".into(),
    });
    assert_eq!(e.message, "later state");
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
