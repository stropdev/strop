use super::*;

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
        target: strop_workspace::Filesystem::Local,
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
