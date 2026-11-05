//! Container namespace routing and lifecycle regressions. Native opens are
//! replay-suppressed: assertions inspect the exact admitted target instead.
use super::*;
use strop_workspace::{ContainerId, Filesystem, ResourceLocation};

fn identity(container: &ContainerId) -> strop_containers::ContainerIdentity {
    strop_containers::ContainerIdentity {
        id: container.to_string(),
        name: "fixture".into(),
        image: "busybox:latest".into(),
        started_at: "2026-09-09T00:00:00Z".into(),
        user: String::new(),
        workdir: "/workspace".into(),
        env: Vec::new(),
    }
}

fn container_editor() -> (Editor, ContainerId, strop_core::id::DocumentId) {
    let container = ContainerId::canonical("c".repeat(64)).unwrap();
    let mut e = editor("host twin\n");
    e.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Ok(serde_json::Value::Bool(false))
    }));
    let origin = e
        .docs
        .try_insert(Document::container_file(
            Buffer::from_text("fn origin() {}\n"),
            container.clone(),
            PathBuf::from("/workspace/origin.rs"),
        ))
        .unwrap();
    e.switch_to(origin);
    e.containers
        .attached
        .insert(container.to_string(), identity(&container));
    (e, container, origin)
}

fn container_arm(
    e: &mut Editor,
    document: strop_core::id::DocumentId,
    container: &ContainerId,
    kind: RequestKind,
) -> ReplyContext {
    let revision = e.docs.get(document).unwrap().buf.revision();
    let server = ServerId::new(41);
    e.lsp_state.bindings.insert(
        document,
        state::Binding {
            server,
            revision,
            path: PathBuf::from("/workspace/origin.rs"),
            root: PathBuf::from("/workspace"),
            language: "rust".into(),
            target: Filesystem::Container(container.clone()),
        },
    );
    let stamp = RequestStamp {
        request: RequestId::new(4),
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

fn settle_picker_rank(e: &mut Editor) {
    let (picker, ticket, ranking) = {
        let glue = e.picker.as_mut().unwrap();
        let ticket = glue.rank_pending.clone().unwrap();
        let ranking = strop_picker::rank::rank(&glue.picker.filter_request(), || false)
            .unwrap()
            .unwrap();
        (glue.id, ticket, ranking)
    };
    e.handle_picker_ranking(crate::editor::picker::ranking::Event {
        picker,
        update: strop_picker::RankingEvent::Completed(strop_core::worker::Completion {
            ticket,
            outcome: strop_core::worker::Outcome::Success(ranking),
        }),
    });
}

fn location(container: &ContainerId, path: &str, line: usize) -> strop_lsp::ServerLocation {
    strop_lsp::ServerLocation {
        doc: ResourceLocation {
            filesystem: Filesystem::Container(container.clone()),
            path: PathBuf::from(path),
        },
        position: ServerPosition {
            line: LineIndex::new(line),
            column: ServerColumn::new(0),
        },
    }
}

fn symbol(location: strop_lsp::ServerLocation, name: &str) -> strop_lsp::ProtoSymbol {
    strop_lsp::ProtoSymbol {
        name: name.into(),
        container: String::new(),
        kind: "Function".into(),
        location,
    }
}

fn assert_pending_container_open(e: &Editor, container: &ContainerId, path: &str) {
    assert_eq!(
        e.io.open.len(),
        1,
        "one replay-suppressed open is registered"
    );
    let key = e.io.open.values().next().unwrap();
    assert_eq!(
        key.path,
        crate::files::FileTarget::Container {
            container: container.clone(),
            path: PathBuf::from(path),
        },
        "the analogous host path is never opened"
    );
}

#[test]
fn container_goto_requests_the_container_target_not_a_host_path() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(&mut e, origin, &container, RequestKind::Goto);
    e.handle_lsp_event(LspEvent::GotoLocation {
        context,
        location: location(&container, "/workspace/definition.rs", 2),
    });
    assert_pending_container_open(&e, &container, "/workspace/definition.rs");
}

#[test]
fn container_reference_rows_accept_with_their_namespace_and_context() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(
        &mut e,
        origin,
        &container,
        RequestKind::Locations(strop_lsp::LocKind::References),
    );
    e.handle_lsp_event(LspEvent::Locations {
        context,
        kind: strop_lsp::LocKind::References,
        items: vec![
            location(&container, "/workspace/first.rs", 0),
            location(&container, "/workspace/second.rs", 1),
        ],
    });
    let picker = &e.picker.as_ref().unwrap().picker;
    assert_eq!(picker.items.len(), 2, "no container reference is dropped");
    assert!(picker.items.iter().all(|item| matches!(
        &item.payload,
        strop_picker::Payload::Grep { location, .. }
            if location.filesystem == Filesystem::Container(container.clone())
    )));
    settle_picker_rank(&mut e);
    e.accept_current_picker();
    assert_pending_container_open(&e, &container, "/workspace/first.rs");
}

#[test]
fn container_document_symbols_route_to_the_container_document() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(&mut e, origin, &container, RequestKind::DocumentSymbols);
    e.handle_lsp_event(LspEvent::Symbols {
        context,
        symbols: vec![symbol(
            location(&container, "/workspace/origin.rs", 0),
            "origin",
        )],
    });
    let picker = &e.picker.as_ref().unwrap().picker;
    assert_eq!(picker.items.len(), 1, "the container symbol is retained");
    settle_picker_rank(&mut e);
    e.accept_current_picker();
    assert_eq!(e.current(), origin);
    assert!(
        e.io.open.is_empty(),
        "the current container document needs no reopen or host alias"
    );
}

#[test]
fn workspace_symbols_dedup_by_namespace_and_open_container_hits() {
    let (mut e, container, _) = container_editor();
    e.set_picker(crate::editor::PickerGlue::diagnostics(
        strop_picker::Picker::new(strop_picker::Kind::WorkspaceSymbols, Vec::new(), false),
    ));
    let generation = e.picker.as_ref().unwrap().wsymbols_generation;
    e.handle_lsp_event(LspEvent::WorkspaceSymbols {
        server: ServerId::new(41),
        generation,
        symbols: vec![
            symbol(
                strop_lsp::ServerLocation {
                    doc: ResourceLocation::local(PathBuf::from("/workspace/shared.rs")),
                    position: at_origin(),
                },
                "shared",
            ),
            symbol(location(&container, "/workspace/shared.rs", 0), "shared"),
        ],
    });
    let picker = &e.picker.as_ref().unwrap().picker;
    assert_eq!(
        picker.items.len(),
        2,
        "same-bytes local and container symbols are distinct"
    );
    let payload = picker
        .items
        .iter()
        .find(|item| {
            matches!(
                &item.payload,
                strop_picker::Payload::Grep { location, .. }
                    if location.filesystem == Filesystem::Container(container.clone())
            )
        })
        .unwrap();
    assert!(payload.text.contains("container:"));
    let payload = payload.payload.clone();
    e.accept_picker(payload, None);
    assert_pending_container_open(&e, &container, "/workspace/shared.rs");
}

#[test]
fn container_diagnostics_remain_navigable_without_host_aliasing() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(&mut e, origin, &container, RequestKind::Goto);
    e.handle_lsp_event(LspEvent::Diagnostics {
        context: strop_lsp::DiagnosticContext {
            server: context.stamp.server,
            document: origin,
            revision: context.stamp.revision,
            encoding: PositionEncoding::Utf8,
            version: Some(WireVersion::new(1)),
        },
        doc: ResourceLocation {
            filesystem: Filesystem::Container(container.clone()),
            path: PathBuf::from("/workspace/origin.rs"),
        },
        diags: vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(0),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(2),
            severity: Severity::Error,
            message: "container problem".into(),
        }],
    });
    e.open_diagnostics_picker();
    let picker = &e.picker.as_ref().unwrap().picker;
    assert_eq!(
        picker.items.len(),
        1,
        "the container diagnostic is retained"
    );
    assert!(picker.items[0].text.contains("container:"));
    settle_picker_rank(&mut e);
    e.accept_current_picker();
    assert_pending_container_open(&e, &container, "/workspace/origin.rs");
}

#[test]
fn container_jump_context_and_outside_root_use_container_paths() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(&mut e, origin, &container, RequestKind::Goto);
    let target = e
        .docs
        .try_insert(Document::container_file(
            Buffer::from_text("pub fn external() {}\n"),
            container.clone(),
            PathBuf::from("/external/lib.rs"),
        ))
        .unwrap();
    e.docs.get_mut(target).unwrap().buf.clear_readonly();
    e.switch_to(origin);
    e.finish_lsp_jump(target, at_origin(), context);
    let carried = e.lsp_state.jump_contexts.get(&target).unwrap();
    assert_eq!(carried.root, PathBuf::from("/workspace"));
    assert_eq!(carried.target, Filesystem::Container(container));
    assert_eq!(
        e.docs.get(target).unwrap().buf.readonly_reason,
        Some(strop_core::ReadonlyReason::OutsideWorkspace),
        "the host cwd never decides whether a container path is outside its root"
    );
}

#[test]
fn closing_the_last_container_workspace_document_retires_its_server() {
    let (mut e, container, origin) = container_editor();
    let _other_root = e
        .docs
        .try_insert(Document::container_file(
            Buffer::from_text("fn elsewhere() {}\n"),
            container.clone(),
            PathBuf::from("/elsewhere/keep.rs"),
        ))
        .unwrap();
    let server = ServerId::new(51);
    e.lsp_state.attach.attached.push(super::attach::Attachment {
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        server,
        target: Filesystem::Container(container.clone()),
    });
    e.lsp_servers.push(LspServer {
        id: server,
        name: "rust-analyzer".into(),
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: Filesystem::Container(container),
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: true,
    });
    e.lsp_close_document(origin);
    e.docs.remove(origin);
    e.lsp_retire_workspace_servers();
    assert!(e.lsp_servers.is_empty());
    assert!(e.lsp_state.attach.attached.is_empty());
}

#[test]
fn container_failure_does_not_clear_or_retire_a_local_server() {
    let (mut e, container, origin) = container_editor();
    let context = container_arm(&mut e, origin, &container, RequestKind::Goto);
    let local = ServerId::new(61);
    e.lsp_servers.push(LspServer {
        id: context.stamp.server,
        name: "rust-analyzer".into(),
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: Filesystem::Container(container.clone()),
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: true,
    });
    e.lsp_servers.push(LspServer {
        id: local,
        name: "rust-analyzer".into(),
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: Filesystem::Local,
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: true,
    });
    e.handle_lsp_event(LspEvent::Failed {
        server: context.stamp.server,
        name: "rust-analyzer".into(),
        hint: "container server exited".into(),
    });
    assert!(e.lsp_servers.iter().any(|server| server.id == local));
    assert_eq!(e.lsp_state.failures.len(), 1);
    assert_eq!(
        e.lsp_state.failures[0].target,
        Filesystem::Container(container)
    );
    e.handle_lsp_event(LspEvent::Ready {
        server: local,
        name: "rust-analyzer".into(),
    });
    assert_eq!(
        e.lsp_state.failures.len(),
        1,
        "local readiness cannot clear the container failure"
    );
}

#[test]
fn container_attach_waits_for_worker_admission_without_sticky_refusal() {
    let (mut e, _, origin) = container_editor();
    e.lsp_state.attach.enabled = true;
    e.lsp_attach_document(origin);
    assert!(e.message.contains(":container-worker"));
    assert!(e.lsp_state.attach.pending.is_empty());
    assert!(e.lsp_state.attach.refused.is_empty());
}

#[test]
fn worker_readiness_retries_the_same_container_incarnation() {
    let (mut e, container, origin) = container_editor();
    e.lsp_state.attach.enabled = true;
    let inspected = identity(&container);
    e.containers
        .workers
        .install_test(&inspected, strop_worker_client::Worker::local());
    let request = WorkerId::new(90);
    let key = crate::editor::containers::ContainerKey {
        request,
        document: origin,
        revision: e.docs.get(origin).unwrap().buf.revision(),
        focus: e.focus_epoch,
        job: crate::editor::containers::ContainerJob::EnableWorker {
            id: container.to_string(),
        },
    };
    let ticket = strop_core::worker::Ticket { request, key };
    e.containers.pending = Some(ticket.clone());
    e.handle_container_event(strop_core::worker::Completion {
        ticket,
        outcome: strop_core::worker::Outcome::Success(
            crate::editor::containers::ContainerResult::WorkerReady {
                id: container.to_string(),
                target: "test-container".into(),
            },
        ),
    });
    assert!(e.lsp_state.attach.pending.keys().any(|key| {
        key.target == Filesystem::Container(container.clone())
            && key.path.as_path() == std::path::Path::new("/workspace/origin.rs")
    }));
}

#[test]
fn missing_container_server_names_worker_admission_not_an_unwired_feature() {
    let (mut e, _, _) = container_editor();
    e.lsp_request(RequestKind::Goto);
    assert!(e.message.contains(":container-worker"), "{}", e.message);
    assert!(e.message.contains("no language server"), "{}", e.message);
    assert!(!e.message.contains("not wired"), "{}", e.message);
}
