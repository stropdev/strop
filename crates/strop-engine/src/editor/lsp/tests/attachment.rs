use super::*;

fn attach_at(
    e: &mut Editor,
    ticket: u64,
    server: u64,
    language: &str,
    name: &str,
    root: &std::path::Path,
    layers: Vec<strop_lsp::languages::LayerDiagnostic>,
) {
    let key = super::super::attach::AttachKey {
        target: strop_workspace::Filesystem::Local,
        language: language.into(),
        path: root.join("main.rs"),
    };
    e.lsp_state
        .attach
        .pending
        .insert(key, WorkerId::new(ticket));
    e.handle_lsp_attach(AttachRecord {
        ticket: WorkerId::new(ticket),
        server: Some(ServerId::new(server)),
        language: language.into(),
        name: name.into(),
        root: root.to_path_buf(),
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::Attached,
        layers,
    });
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
        target: strop_workspace::Filesystem::Local,
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
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::NotExecutable {
            command: "rust-analyzer".into(),
            reason: "not found on PATH".into(),
            hint: "rustup component add rust-analyzer".into(),
        },
        layers: Vec::new(),
    });
    assert_eq!(
        e.message,
        "lsp: rust-analyzer not found on PATH — rustup component add rust-analyzer"
    );
    let refusal = &e.lsp_state.attach.refused[&attach_key(&e, "rust")];
    let details = crate::editor::explain::explain_decision(refusal);
    assert!(details.contains("not found on PATH"));
    assert!(details.contains("rust-analyzer"));
    assert!(details.contains("rustup component add rust-analyzer"));
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
        target: strop_workspace::Filesystem::Local,
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
        target: strop_workspace::Filesystem::Local,
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
    e.lsp_attach_document(e.current());
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
        target: strop_workspace::Filesystem::Local,
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
        target: strop_workspace::Filesystem::Local,
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
        target: strop_workspace::Filesystem::Local,
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
        target: strop_workspace::Filesystem::Local,
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

/// 0033 §3: a spawn refusal compacts the modeline and retains its typed
/// reason in the explain buffer.
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
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::SpawnFailed {
            reason: "cannot build the LSP runtime: boom".into(),
        },
        layers: Vec::new(),
    });
    assert!(e.message.contains("did not start"), "{}", e.message);
    assert!(e.message.contains(":explain"), "{}", e.message);
    assert!(!e.message.contains("runtime"), "{}", e.message);
    let refusal = &e.lsp_state.attach.refused[&attach_key(&e, "rust")];
    assert!(
        crate::editor::explain::explain_decision(refusal).contains("runtime"),
        "the complete reason remains inspectable"
    );
    assert!(e
        .lsp_state
        .attach
        .refused
        .contains_key(&attach_key(&e, "rust")));
    e.message = "unrelated later message".into();
    e.open_explain();
    let explained = e.buf().text().to_string();
    assert!(
        explained.contains("cannot build the LSP runtime: boom"),
        "{explained}"
    );
    assert!(
        explained.contains("resource /workspace/origin.rs"),
        "{explained}"
    );
    assert!(explained.contains("on local"), "{explained}");
}

/// 0033 §3: a terminal Failed event retains the executable-bearing hint for
/// :explain and tears the server down; a failure for an already-removed
/// server cannot resurrect a message or replace the retained reason.
#[test]
fn failed_server_event_names_the_command_and_removes_the_server() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let server = ServerId::new(6);
    e.lsp_servers.push(LspServer {
        id: server,
        name: "pyright".into(),
        language: "python".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
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
    assert!(e.message.contains("failed"), "{}", e.message);
    assert!(e.message.contains(":explain"), "{}", e.message);
    assert!(!e.message.contains("npm i -g pyright"), "{}", e.message);
    assert!(e.lsp_servers.is_empty());
    assert_eq!(e.lsp_state.failures.len(), 1);
    assert!(e.lsp_state.failures[0]
        .reason
        .contains("pyright-langserver"));
    assert!(e.lsp_state.failures[0].reason.contains("npm i -g pyright"));
    // a duplicate failure for the removed server is refused, not shown
    e.message = "later state".into();
    e.handle_lsp_event(LspEvent::Failed {
        server,
        name: "pyright".into(),
        hint: "stale".into(),
    });
    assert_eq!(e.message, "later state");
    assert_eq!(e.lsp_state.failures.len(), 1);
    e.open_explain();
    let explained = e.buf().text().to_string();
    assert!(explained.contains("pyright-langserver"), "{explained}");
    assert!(explained.contains("npm i -g pyright"), "{explained}");
    assert!(explained.contains("server 6"), "{explained}");
    assert!(explained.contains("workspace /workspace"), "{explained}");
    assert!(explained.contains("on local"), "{explained}");
}

#[test]
fn recovery_clears_only_its_owned_failure_scope() {
    let mut e = editor("a\n");
    let one = PathBuf::from("/workspace/one");
    let two = PathBuf::from("/workspace/two");
    let diagnostic = strop_lsp::languages::LayerDiagnostic {
        path: PathBuf::from("/xdg/languages.toml"),
        message: "bad layer — ignored".into(),
        remote: None,
    };
    attach_at(&mut e, 30, 30, "rust", "rust-one", &one, vec![diagnostic]);
    attach_at(&mut e, 31, 31, "rust", "rust-two", &two, Vec::new());
    assert!(e
        .lsp_statuses()
        .all(|status| status.phase == LspPhase::Starting));

    for (server, name) in [
        (ServerId::new(30), "rust-one"),
        (ServerId::new(31), "rust-two"),
    ] {
        e.handle_lsp_event(LspEvent::Ready {
            server,
            name: name.into(),
        });
    }
    assert!(e
        .lsp_statuses()
        .all(|status| status.phase == LspPhase::Ready));
    for (server, name, reason) in [
        (ServerId::new(30), "rust-one", "first complete failure"),
        (ServerId::new(31), "rust-two", "second complete failure"),
    ] {
        e.handle_lsp_event(LspEvent::Failed {
            server,
            name: name.into(),
            hint: reason.into(),
        });
    }
    assert_eq!(e.lsp_state.failures.len(), 2);
    e.message = "unrelated later input".into();
    assert!(e
        .lsp_state
        .failures
        .iter()
        .any(|failure| failure.reason == "first complete failure"));

    // A new attachment for one root starts recovery, but only Ready
    // retires its retained failure. The unrelated root stays untouched.
    attach_at(&mut e, 32, 32, "rust", "rust-one-new", &one, Vec::new());
    assert_eq!(e.lsp_state.failures.len(), 2);
    assert!(e
        .lsp_state
        .failures
        .iter()
        .any(|failure| failure.root == one));
    assert!(e
        .lsp_state
        .failures
        .iter()
        .any(|failure| failure.root == two));
    assert_eq!(e.lsp_state.attach.layer_diagnostics.len(), 1);
    e.handle_lsp_event(LspEvent::Ready {
        server: ServerId::new(32),
        name: "rust-one-new".into(),
    });
    assert_eq!(e.lsp_state.failures.len(), 1);
    assert_eq!(e.lsp_state.failures[0].root, two);
    assert!(e.lsp_statuses().any(|status| {
        status.phase == LspPhase::Ready
            && status.resource == one.as_path()
            && status.name == "rust-one-new"
    }));
    assert!(e.lsp_statuses().any(|status| {
        status.phase == LspPhase::Failed
            && status.resource == two.as_path()
            && status.name == "rust-two"
    }));

    // An owned Ready for the other scope clears its older server failure,
    // independently of the readiness cleanup exercised above.
    e.lsp_servers.push(LspServer {
        id: ServerId::new(33),
        name: "rust-two-new".into(),
        language: "rust".into(),
        root: two.clone(),
        target: strop_workspace::Filesystem::Local,
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: false,
    });
    e.handle_lsp_event(LspEvent::Ready {
        server: ServerId::new(33),
        name: "rust-two-new".into(),
    });
    assert!(e.lsp_state.failures.is_empty());
    assert!(e
        .lsp_statuses()
        .all(|status| status.phase == LspPhase::Ready));

    let before = e.lsp_state.failures.len();
    e.handle_lsp_event(LspEvent::Ready {
        server: ServerId::new(999),
        name: "ghost".into(),
    });
    e.handle_lsp_event(LspEvent::Failed {
        server: ServerId::new(999),
        name: "ghost".into(),
        hint: "stale".into(),
    });
    assert_eq!(e.lsp_state.failures.len(), before);
    assert_eq!(e.lsp_state.attach.layer_diagnostics.len(), 1);
}

#[test]
fn successful_retry_replaces_its_refusal_with_starting_then_ready() {
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    let key = attach_key(&e, "rust");
    e.lsp_state
        .attach
        .pending
        .insert(key.clone(), WorkerId::new(50));
    e.handle_lsp_attach(AttachRecord {
        ticket: WorkerId::new(50),
        server: None,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::RemoteIo {
            reason: "connection lost".into(),
        },
        layers: Vec::new(),
    });
    assert!(e
        .lsp_statuses()
        .any(|status| status.phase == LspPhase::Refused));

    e.lsp_state
        .attach
        .pending
        .insert(key.clone(), WorkerId::new(51));
    e.handle_lsp_attach(AttachRecord {
        ticket: WorkerId::new(51),
        server: Some(ServerId::new(51)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::Attached,
        layers: Vec::new(),
    });
    assert!(!e.lsp_state.attach.refused.contains_key(&key));
    assert!(e
        .lsp_statuses()
        .all(|status| status.phase == LspPhase::Starting));
    e.handle_lsp_event(LspEvent::Ready {
        server: ServerId::new(51),
        name: "rust-analyzer".into(),
    });
    assert!(e
        .lsp_statuses()
        .all(|status| status.phase == LspPhase::Ready));
}
