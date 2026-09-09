//! Remote endpoint, document and capability regressions.
use super::*;

fn remote_doc(e: &mut Editor, uri: &str, text: &str) -> strop_core::id::DocumentId {
    let file = strop_workspace::RemoteFile::parse(uri).expect("canonical remote uri");
    e.docs.insert(Document::remote(
        strop_core::Buffer::from_text(text),
        crate::editor::document::RemoteDocument {
            file,
            window: strop_remote::RemoteWindow::resolve(
                &strop_remote::ReadSelection::Full,
                strop_remote::RemoteSize::new(text.len() as u64),
            ),
            selection: strop_remote::ReadSelection::Full,
            connection: None,
            return_to: None,
            write: None,
        },
    ))
}

fn remote_arm(
    e: &mut Editor,
    document: strop_core::id::DocumentId,
    endpoint: &strop_workspace::RemoteEndpoint,
    path: &std::path::Path,
) -> ReplyContext {
    let revision = e.docs.get(document).unwrap().buf.revision();
    let server = ServerId::new(11);
    e.lsp_state.bindings.insert(
        document,
        state::Binding {
            server,
            revision,
            path: path.to_path_buf(),
            root: PathBuf::from("/w/proj"),
            target: strop_workspace::Filesystem::Remote(endpoint.clone()),
        },
    );
    let stamp = RequestStamp {
        request: RequestId::new(0),
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

/// The same remote path bytes must never collide with the local
/// diagnostics store: endpoints are part of the key.
#[test]
fn remote_diagnostics_never_alias_local_paths() {
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder.example").unwrap();
    let mut e = editor("local twin\n");
    e.buf_mut().path = Some(PathBuf::from("/w/proj/a.rs"));
    let local = e.current();
    let remote = remote_doc(&mut e, "ssh://builder.example/w/proj/a.rs", "remote twin\n");
    let _local_arm = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    let context = remote_arm(&mut e, remote, &endpoint, Path::new("/w/proj/a.rs"));
    e.handle_lsp_event(LspEvent::Diagnostics {
        context: strop_lsp::DiagnosticContext {
            server: context.stamp.server,
            document: context.stamp.document,
            revision: context.stamp.revision,
            encoding: PositionEncoding::Utf8,
            version: Some(WireVersion::new(1)),
        },
        doc: strop_workspace::ResourceLocation::remote(endpoint, PathBuf::from("/w/proj/a.rs")),
        diags: vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(0),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(4),
            severity: Severity::Error,
            message: "remote problem".into(),
        }],
    });
    assert_eq!(e.diag_counts(remote), (1, 0));
    assert_eq!(e.diag_counts(local), (0, 0));
}

/// A goto on a remote server routes to the endpoint's file identity —
/// through the already-open remote document, never the analogous
/// local path.
#[test]
fn remote_goto_routes_to_the_remote_target_not_local() {
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder.example").unwrap();
    let mut e = editor("origin\n");
    e.buf_mut().path = Some(PathBuf::from("/w/proj/origin.rs"));
    let origin = e.current();
    let target = remote_doc(&mut e, "ssh://builder.example/w/proj/other.rs", "a😀z\n");
    e.switch_to(origin);
    let context = remote_arm(&mut e, origin, &endpoint, Path::new("/w/proj/origin.rs"));
    e.handle_lsp_event(LspEvent::GotoLocation {
        context,
        location: strop_lsp::ServerLocation {
            doc: strop_workspace::ResourceLocation::remote(
                endpoint,
                PathBuf::from("/w/proj/other.rs"),
            ),
            position: ServerPosition {
                line: LineIndex::new(0),
                column: ServerColumn::new(1),
            },
        },
    });
    // The already-open remote document was reused: no local open was
    // requested, and the switch landed on the remote target.
    assert_eq!(e.current(), target);
    assert_eq!(e.head(), 1);
}

/// Server columns on a remote jump resolve against the remote
/// document's rope with the negotiated encoding — same ownership as
/// local navigation.
#[test]
fn remote_goto_resolves_utf16_columns_against_the_remote_rope() {
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder.example").unwrap();
    let mut e = editor("origin\n");
    e.buf_mut().path = Some(PathBuf::from("/w/proj/origin.rs"));
    let origin = e.current();
    let target = remote_doc(&mut e, "ssh://builder.example/w/proj/other.rs", "a😀z\n");
    e.switch_to(origin);
    let mut context = remote_arm(&mut e, origin, &endpoint, Path::new("/w/proj/origin.rs"));
    context.encoding = PositionEncoding::Utf16;
    e.finish_lsp_jump(
        target,
        ServerPosition {
            line: LineIndex::new(0),
            column: ServerColumn::new(3),
        },
        context,
    );
    assert_eq!(e.current(), target);
    assert_eq!(e.head(), 5, "utf16 column 3 is byte 5 (after the emoji)");
}

/// Closing the last document of a remote workspace retires its server
/// (0036): no orphan ssh client, no live placement.
#[test]
fn closing_the_last_remote_document_retires_its_server() {
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder.example").unwrap();
    let mut e = editor("keepalive\n");
    e.buf_mut().path = Some(PathBuf::from("/w/keep.txt"));
    let remote = remote_doc(&mut e, "ssh://builder.example/w/proj/a.rs", "remote\n");
    remote_doc(
        &mut e,
        "ssh://builder.example/another-project/a.rs",
        "other workspace\n",
    );
    let ticket = WorkerId::new(21);
    e.lsp_state.attach.pending.insert(
        super::super::attach::AttachKey {
            target: strop_workspace::Filesystem::Remote(endpoint.clone()),
            language: "rust".into(),
            path: "/w/proj/a.rs".into(),
        },
        ticket,
    );
    e.handle_lsp_attach(AttachRecord {
        ticket,
        server: Some(ServerId::new(31)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/w/proj"),
        target: strop_workspace::Filesystem::Remote(endpoint),
        outcome: AttachDecision::Attached,
        layers: Vec::new(),
    });
    assert_eq!(e.lsp_servers.len(), 1);
    // Another remote project on the same endpoint must not retain this server.
    e.lsp_close_document(remote);
    e.docs.remove(remote);
    e.lsp_retire_remote_servers();
    assert!(
        e.lsp_servers.is_empty(),
        "the last owning remote workspace retires its server"
    );
    assert!(e.lsp_state.attach.attached.is_empty());
}

/// Partial windows cannot be admitted as complete language-server documents.
#[test]
fn partial_remote_window_refuses_attach_visibly() {
    let mut e = editor("tail window\n");
    let file = strop_workspace::RemoteFile::parse("ssh://builder.example/w/proj/a.rs").unwrap();
    let id = e.docs.insert(Document::remote(
        strop_core::Buffer::from_text("tail\n"),
        crate::editor::document::RemoteDocument {
            file,
            selection: strop_remote::ReadSelection::Tail(strop_remote::ReadLimit::new(5).unwrap()),
            window: strop_remote::RemoteWindow::resolve(
                &strop_remote::ReadSelection::Tail(strop_remote::ReadLimit::new(5).unwrap()),
                strop_remote::RemoteSize::new(100),
            ),
            connection: None,
            return_to: None,
            write: None,
        },
    ));
    e.switch_to(id);
    e.lsp_start_services();
    assert!(e.lsp_state.attach.pending.is_empty());
    assert!(e.lsp_state.bindings.is_empty());
    assert!(!e.message.is_empty(), "capability refusal is visible");
}
