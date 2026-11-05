use super::*;

#[test]
fn terminal_fixtures_retain_full_launch_identity_and_ignore_shell_metadata() {
    let mut editor = Editor::new_in(Buffer::from_text("origin"), "/host".into());
    let local = strop_workspace::ResourceLocation::local("/host/launch".into());
    let local_id = editor.terminal_fixture_at(
        local.clone(),
        &[("local", Style::default())],
        Phase::Running,
    );
    assert_eq!(editor.terminal_launch_location(local_id), Some(&local));
    assert_eq!(
        editor.terminal_status(local_id).unwrap()["namespace"],
        "local"
    );
    assert_eq!(
        editor.terminal_status(local_id).unwrap()["launch_location"],
        "/host/launch"
    );

    let remote = strop_workspace::ResourceLocation::remote(
        strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
        "/srv/launch".into(),
    );
    let remote_id = editor.terminal_fixture_at(
        remote.clone(),
        &[("remote", Style::default())],
        Phase::Running,
    );
    assert_eq!(editor.terminal_launch_location(remote_id), Some(&remote));
    let remote_label = format!(
        "terminal #{} · launch ssh://fixture/srv/launch",
        editor.terminal_document(remote_id).unwrap().session.get()
    );
    assert_eq!(
        editor
            .doc(remote_id)
            .label(std::path::Path::new("/unrelated")),
        remote_label
    );
    let state = editor.terminal_status(remote_id).unwrap();
    assert_eq!(state["namespace"], "ssh://fixture");
    assert_eq!(state["launch"], serde_json::to_value(&remote).unwrap());
    assert_eq!(state["launch_location"], "ssh://fixture/srv/launch");
    assert_eq!(state["launch_directory"], "/srv/launch");
    assert_eq!(editor.current_workspace_root(), remote);

    let session = editor.terminal_document(remote_id).unwrap().session;
    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Running,
        frame: None,
        effects: vec![
            strop_terminal::model::Effect::Title("shell-owned title".into()),
            strop_terminal::model::Effect::ReportedDirectory("/tmp/spoofed".into()),
        ],
        acknowledged_input: 0,
        warning: None,
    });
    assert_eq!(editor.terminal_launch_location(remote_id), Some(&remote));
    assert_eq!(editor.terminal_title(remote_id), Some("shell-owned title"));
    assert_eq!(
        editor
            .doc(remote_id)
            .label(std::path::Path::new("/unrelated")),
        remote_label
    );

    editor.open_picker(strop_picker::Kind::Buffers);
    let rows = &editor.picker.as_ref().unwrap().picker.items;
    let local_prefix = format!(
        "terminal #{}",
        editor.terminal_document(local_id).unwrap().session.get()
    );
    let remote_prefix = format!(
        "terminal #{}",
        editor.terminal_document(remote_id).unwrap().session.get()
    );
    let local_row = rows
        .iter()
        .find(|item| item.text.starts_with(&local_prefix))
        .unwrap();
    let remote_row = rows
        .iter()
        .find(|item| item.text.starts_with(&remote_prefix))
        .unwrap();
    assert!(
        local_row.text.contains("launch /host/launch"),
        "{local_row:?}"
    );
    assert!(
        remote_row.text.contains("launch ssh://fixture/srv/launch"),
        "{remote_row:?}"
    );
    assert!(!remote_row.text.contains("spoofed"), "{remote_row:?}");
    assert!(!remote_row.text.contains("shell-owned"), "{remote_row:?}");
}

#[test]
fn terminal_start_trace_retains_location_and_terminal_local_stays_local() {
    let mut editor = Editor::new_in(Buffer::from_text("origin"), "/host".into());
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Ok(serde_json::Value::Bool(true))
    }));
    let remote = strop_workspace::ResourceLocation::remote(
        strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
        "/srv/launch".into(),
    );
    let source = crate::editor::Directory::new(strop_workspace::DirectorySnapshot {
        location: remote,
        entries: Arc::from([]),
        state: strop_workspace::ListingState::Complete,
    });
    let directory = editor
        .docs
        .try_insert(Document::directory(
            Buffer::from_text(&source.text()),
            source,
        ))
        .unwrap();
    editor.switch_to(directory);
    editor.set_terminal_capture(true);
    editor.feed_text(":terminal-local printf TRACE-IDENTITY\r");
    let terminal = editor.current();
    assert_eq!(
        editor.terminal_launch_location(terminal),
        Some(&strop_workspace::ResourceLocation::local("/host".into()))
    );
    let nodes = editor.tape.fixture_nodes();
    let start = nodes.iter().find_map(|node| match node {
        strop_trace::replay::Node::Call {
            operation,
            arguments,
            ..
        } if operation == "terminal.start" => Some(arguments),
        _ => None,
    });
    let start = start.expect("terminal.start recorded");
    assert_eq!(start[1]["filesystem"], "Local");
    assert_eq!(
        start[1]["path"]["bytes"],
        serde_json::json!([47, 104, 111, 115, 116])
    );
}

#[test]
fn refused_container_terminal_never_falls_back_to_the_host() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let origin = editor.current();
    editor.launch_terminal_at(
        strop_workspace::ResourceLocation {
            filesystem: strop_workspace::Filesystem::Container(
                strop_workspace::ContainerId::canonical("a".repeat(64)).unwrap(),
            ),
            path: "/srv".into(),
        },
        "touch should-not-execute",
    );
    assert_eq!(editor.current(), origin);
    assert!(editor.terminals.entries.is_empty());
    assert!(editor.message.contains("refused"), "{}", editor.message);
    assert!(!directory.path().join("should-not-execute").exists());
}
