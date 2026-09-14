use super::*;
use crate::editor::{Document, DocumentSource, Key};
use strop_core::Buffer;
use strop_terminal::model::{
    Cell, Color, Cursor, CursorShape, Palette, ProjectedRow, Rgb, Row, Style,
};

fn retained_terminal(editor: &mut Editor) -> DocumentId {
    let session = SessionId::from_request(editor.worker_ids.allocate().unwrap());
    let geometry = Geometry {
        columns: 4,
        rows: 1,
        revision: 1,
    };
    let row = Arc::new(Row {
        text: "safe".into(),
        cells: (1..=4)
            .map(|end| Cell {
                end,
                width: 1,
                style: Style {
                    foreground: Color::Default,
                    ..Style::default()
                },
            })
            .collect(),
        wrapped: false,
    });
    let frame = Arc::new(Frame {
        session,
        revision: 1,
        geometry,
        alternate: false,
        cursor: Cursor {
            column: 0,
            row: 0,
            visible: true,
            blinking: false,
            shape: CursorShape::Block,
        },
        palette: Arc::new(Palette {
            foreground: Rgb {
                red: 200,
                green: 200,
                blue: 200,
            },
            background: Rgb {
                red: 0,
                green: 0,
                blue: 0,
            },
            colors: vec![
                Rgb {
                    red: 0,
                    green: 0,
                    blue: 0
                };
                256
            ],
        }),
        history_rows: 0,
        available_history_rows: 0,
        history_limited: false,
        origin: 0,
        rows: std::iter::once(ProjectedRow {
            absolute_start: 0,
            row,
        })
        .collect(),
        projection: ropey::Rope::from_str("safe\n"),
    });
    frame.validate().unwrap();
    let mut document = Document::output(Buffer::from_snapshot(frame.projection.clone()));
    document.source = DocumentSource::Terminal(Box::new(TerminalDocument {
        session,
        frame: Some(frame.clone()),
    }));
    let id = editor.docs.insert(document);
    editor.terminals.entries.insert(
        session,
        Entry {
            document: id,
            phase: Phase::Exited {
                code: Some(0),
                signal: None,
            },
            live: Some(frame),
            service: None,
            directory: editor.cwd.clone(),
            title: None,
            geometry,
            paste: None,
            keyboard: 0,
        },
    );
    editor.switch_to(id);
    id
}

#[test]
fn unsupported_remote_terminal_never_retargets_the_document_to_local_execution() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let location = strop_workspace::ResourceLocation::remote(
        strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
        "/workspace".into(),
    );
    let source = crate::editor::Directory::new(strop_workspace::DirectorySnapshot {
        location: location.clone(),
        entries: Arc::from([]),
        state: strop_workspace::ListingState::Complete,
    });
    let id = editor.docs.insert(Document::directory(
        Buffer::from_text(&source.text()),
        source,
    ));
    editor.switch_to(id);
    editor.feed_text(":terminal touch should-not-execute\r");
    assert_eq!(editor.current(), id);
    assert_eq!(editor.directory().unwrap().location, location);
    assert!(editor.terminals.entries.is_empty());
    assert!(editor.message.contains(":terminal-local"));
    assert!(!directory.path().join("should-not-execute").exists());
}

#[test]
fn terminal_snapshot_cannot_be_made_writable_saved_or_implicitly_relaunched() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let id = retained_terminal(&mut editor);
    editor.feed_text(":set noro\rdd:w! exported.txt\r");
    assert!(editor.buf().readonly);
    assert_eq!(editor.buf().text().to_string(), "safe\n");
    assert!(!directory.path().join("exported.txt").exists());
    assert!(matches!(editor.doc(id).source, DocumentSource::Terminal(_)));
    editor.feed(Key::Char('i'));
    assert!(!editor.terminal_input_active());
    assert_eq!(editor.terminals.entries.len(), 1);
    assert!(matches!(
        editor.terminal_phase(id),
        Some(Phase::Exited { code: Some(0), .. })
    ));
}

#[test]
fn terminal_window_prefix_moves_panes_without_reaching_the_child() {
    use strop_core::frontend_input::{Input, KeyCode, KeyEvent};

    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("left"), directory.path().to_owned());
    let id = retained_terminal(&mut editor);
    assert!(editor.enter_terminal_input());
    editor.split_pub('v');
    assert_eq!(editor.panes.len(), 2);
    let terminal_pane = editor.active_pane;

    let ctrl = |code: KeyCode| {
        Input::Key(KeyEvent {
            code,
            modifiers: strop_core::frontend_input::Modifiers {
                control: true,
                ..Default::default()
            },
            ..KeyEvent::press(code)
        })
    };
    // Ctrl-W opens the prefix; the follow-up moves focus off the terminal
    // pane without a single byte reaching the (service-less) child — the
    // message stays clear, proving no failed send was attempted.
    editor.feed_terminal(ctrl(KeyCode::Char('w')));
    editor.message.clear();
    editor.feed_terminal(Input::Key(KeyEvent::press(KeyCode::Char('l'))));
    assert_ne!(editor.active_pane, terminal_pane);
    assert!(editor.message.is_empty(), "{}", editor.message);
    assert!(editor.terminals.prefix.is_none());

    // the same prefix grammar escapes into pinned-snapshot inspection
    let mut other = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    retained_terminal(&mut other);
    other.feed_terminal(ctrl(KeyCode::Char('w')));
    other.feed_terminal(Input::Key(KeyEvent::press(KeyCode::Char('N'))));
    assert_eq!(other.mode, crate::editor::Mode::Normal);
    assert!(other.message.contains("snapshot"), "{}", other.message);
    // the exited session was never relaunched by any of it
    assert!(matches!(
        editor.terminal_phase(id),
        Some(Phase::Exited { code: Some(0), .. })
    ));
}

#[test]
fn buffers_list_shows_terminals_with_their_live_phase() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    retained_terminal(&mut editor);
    editor.open_picker(strop_picker::Kind::Buffers);
    let glue = editor.picker.as_ref().unwrap();
    let terminal_item = glue
        .picker
        .items
        .iter()
        .find(|item| item.text.starts_with("terminal #"))
        .expect("terminal listed in buffers");
    assert_eq!(terminal_item.badge.as_deref(), Some("!"));
    assert!(
        terminal_item.text.contains("exited 0"),
        "{}",
        terminal_item.text
    );
}

#[test]
fn private_terminal_command_paste_is_classified_before_action_capture() {
    use crate::editor::trace::drive::Action;
    use strop_core::frontend_input::Input;
    let mut editor = Editor::new_in(Buffer::from_text(""), "/workspace".into());
    let tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other("unexpected native observation"))
    }));
    editor.tape = tape.clone();
    editor.feed_text(":termi");
    editor
        .recorded_action(
            Action::Event(crate::editor::events::AppEvent::Input(Input::Paste(
                "nal printf PRIVATE-COMMAND".into(),
            ))),
            tape.now(),
        )
        .unwrap();
    assert_eq!(editor.pending.text(), ":terminal printf PRIVATE-COMMAND");
    assert!(tape.content_omitted());
    assert!(!serde_json::to_string(&tape.fixture_nodes())
        .unwrap()
        .contains("PRIVATE-COMMAND"));
}
