use super::*;
use crate::editor::{Document, DocumentSource, Key, Mode};
use strop_core::Buffer;
use strop_terminal::model::{Color, Style, Update};

fn retained_terminal(editor: &mut Editor) -> DocumentId {
    editor.terminal_fixture(
        &[("safe", Style::default())],
        Phase::Exited {
            code: Some(0),
            signal: None,
        },
    )
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
    let id = editor
        .docs
        .try_insert(Document::directory(
            Buffer::from_text(&source.text()),
            source,
        ))
        .unwrap();
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

/// 0065 S2 (invariant): focus loss cancels a pending escape prefix — a
/// later Ctrl-N goes to the child, never silently completes the escape.
#[test]
fn focus_loss_cancels_a_pending_terminal_escape_prefix() {
    use strop_core::frontend_input::{Input, KeyCode, KeyEvent, Modifiers};
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(&[("safe", Style::default())], Phase::Running);
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    let ctrl = |code: KeyCode| {
        Input::Key(KeyEvent {
            code,
            modifiers: Modifiers {
                control: true,
                ..Default::default()
            },
            ..KeyEvent::press(code)
        })
    };
    editor.feed_terminal(ctrl(KeyCode::Char('\\')));
    assert!(editor.terminals.prefix.is_some());
    editor.terminal_focus_changed(false);
    assert!(editor.terminals.prefix.is_none());
    editor.terminal_focus_changed(true);
    editor.feed_terminal(ctrl(KeyCode::Char('n')));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.message, "terminal service unavailable");
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

/// 0065 S0: the baseline modal journey as one deterministic spine —
/// enter input, child-bound keys, both escapes to inspection, motions,
/// yank, pinned output, explicit refresh, return to input.
#[test]
fn modal_terminal_baseline_journey() {
    use strop_core::frontend_input::{Input, KeyCode, KeyEvent, Modifiers};
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let id = editor.terminal_fixture(
        &[
            ("first", Style::default()),
            ("second", Style::default()),
            ("third", Style::default()),
        ],
        Phase::Running,
    );
    // i enters terminal-input: owner, view state and the honest message agree.
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.input_owner(), crate::editor::InputOwner::Terminal);
    assert!(
        editor.message.contains("Ctrl-\\ Ctrl-N"),
        "{}",
        editor.message
    );
    // A raw key routes to the terminal path, never the grammar: the
    // cursor stays put and the (service-less) send reports its own
    // failure instead of moving like a Normal-mode j.
    let head = editor.head();
    editor.feed_terminal(Input::Key(KeyEvent::press(KeyCode::Char('j'))));
    assert_eq!(editor.head(), head);
    assert!(editor.terminal_input_active());
    assert_eq!(editor.message, "terminal service unavailable");
    // Ctrl-\ Ctrl-N leaves to pinned inspection.
    let ctrl = |code: KeyCode| {
        Input::Key(KeyEvent {
            code,
            modifiers: Modifiers {
                control: true,
                ..Default::default()
            },
            ..KeyEvent::press(code)
        })
    };
    editor.feed_terminal(ctrl(KeyCode::Char('\\')));
    editor.feed_terminal(ctrl(KeyCode::Char('n')));
    assert!(!editor.terminal_input_active());
    assert_eq!(editor.mode, Mode::Normal);
    assert!(editor.message.contains("snapshot"), "{}", editor.message);
    // Motions and yank operate on the projected text.
    editor.feed_text("j");
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    editor.feed_text("yy");
    assert_eq!(editor.register(None).text, "second\n");
    // Output arriving under inspection stays pinned; the statusline
    // signal flips, the buffer does not.
    let session = editor.terminal_document(id).unwrap().session;
    let updated = fixture_frame(
        session,
        &[
            ("first", Style::default()),
            ("second", Style::default()),
            ("THIRD!", Style::default()),
        ],
        2,
        0,
    );
    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Running,
        frame: Some(updated),
        effects: Vec::new(),
        acknowledged_input: 0,
        warning: None,
    });
    assert!(editor.terminal_has_new_output(id));
    assert!(!editor.buf().text().to_string().contains("THIRD!"));
    // The explicit refresh reinstalls the latest frame; positions map by
    // stream byte, so the same-width revision keeps the line-1 cursor.
    editor.feed_text(":terminal-refresh\r");
    assert!(editor.buf().text().to_string().contains("THIRD!"));
    assert!(!editor.terminal_has_new_output(id));
    assert!(editor.message.contains("refreshed"), "{}", editor.message);
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    // i returns to live input, following the child's cursor.
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
}

/// 0065 S4: the refresh remap carries cursor and saved anchors across a
/// scrollback-shifted frame — the same content stays addressed.
#[test]
fn terminal_refresh_remaps_anchors_across_scrollback_drops() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let id = editor.terminal_fixture(
        &[
            ("dropped", Style::default()),
            ("kept-one", Style::default()),
            ("kept-two", Style::default()),
        ],
        Phase::Running,
    );
    // Cursor and mark a sit on "kept-one" (line 1, stream byte 9: the
    // padded "dropped " row plus its newline occupy 0..9).
    editor.feed_text("j0ma");
    assert_eq!(editor.head(), 9);
    // The child scrolled one row out: the new frame's stream origin moved
    // past "dropped \n".
    let session = editor.terminal_document(id).unwrap().session;
    let updated = fixture_frame(
        session,
        &[
            ("kept-one", Style::default()),
            ("kept-two", Style::default()),
            ("fresh", Style::default()),
        ],
        2,
        9,
    );
    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Running,
        frame: Some(updated),
        effects: Vec::new(),
        acknowledged_input: 0,
        warning: None,
    });
    assert!(editor.terminal_has_new_output(id));
    assert!(editor.buf().text().to_string().starts_with("dropped"));
    editor.feed_text(":terminal-refresh\r");
    assert!(editor.buf().text().to_string().starts_with("kept-one"));
    // Cursor remapped to the same content: byte 9 of the old stream is
    // byte 0 of the refreshed projection.
    assert_eq!(editor.head(), 0);
    assert_eq!(editor.buf().line_of(editor.head()), 0);
    // The mark followed the same correspondence.
    editor.feed_text("G'a");
    assert_eq!(editor.head(), 0);
    assert_eq!(editor.buf().line_of(editor.head()), 0);
}

/// 0065 S4: the full motion surface over the projection — j/k, gg/G,
/// Ctrl-D/U, / and ? with n/N, and visual yank — asserted, not assumed.
#[test]
fn terminal_projection_supports_the_full_motion_surface() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.view_rows = 20;
    let mut rows: Vec<(String, Style)> = (0..30)
        .map(|index| (format!("filler {index:02}"), Style::default()))
        .collect();
    rows[5] = ("needle one".into(), Style::default());
    rows[20] = ("needle two".into(), Style::default());
    let refs: Vec<(&str, Style)> = rows.iter().map(|(t, s)| (t.as_str(), *s)).collect();
    editor.terminal_fixture(&refs, Phase::Running);
    // Ctrl-W N (vim's t_CTRL-W inspection) — the journey used Ctrl-\ Ctrl-N.
    editor.enter_terminal_input();
    editor.feed_terminal(strop_core::frontend_input::Input::Key(
        strop_core::frontend_input::KeyEvent {
            code: strop_core::frontend_input::KeyCode::Char('w'),
            modifiers: strop_core::frontend_input::Modifiers {
                control: true,
                ..Default::default()
            },
            ..strop_core::frontend_input::KeyEvent::press(
                strop_core::frontend_input::KeyCode::Char('w'),
            )
        },
    ));
    editor.feed_terminal(strop_core::frontend_input::Input::Key(
        strop_core::frontend_input::KeyEvent::press(strop_core::frontend_input::KeyCode::Char('N')),
    ));
    assert!(!editor.terminal_input_active());
    assert!(editor.message.contains("snapshot"));
    let line = |editor: &Editor| editor.buf().line_of(editor.head());
    editor.feed_text("G");
    assert_eq!(line(&editor), 29);
    editor.feed_text("gg");
    assert_eq!(line(&editor), 0);
    editor.feed_text("j");
    assert_eq!(line(&editor), 1);
    editor.feed_text("k");
    assert_eq!(line(&editor), 0);
    editor.feed(Key::CtrlD);
    assert_eq!(line(&editor), 10, "ctrl-d is a half page of 20 rows");
    editor.feed(Key::CtrlU);
    assert_eq!(line(&editor), 0);
    editor.feed_text("/needle\r");
    assert_eq!(line(&editor), 5);
    editor.feed_text("n");
    assert_eq!(line(&editor), 20);
    editor.feed_text("N");
    assert_eq!(line(&editor), 5);
    editor.feed_text("?needle\r");
    assert_eq!(line(&editor), 20, "? wraps to the previous hit");
    editor.feed_text("ggVGy");
    assert!(editor.register(None).text.contains("needle one"));
    assert!(editor.register(None).text.contains("filler 29"));
}

/// 0065 S4: wide/CJK cells keep their cell↔byte correspondence — motions
/// never land mid-cluster and styling resolves every byte of a wide
/// cluster to the same cell.
#[test]
fn wide_cells_keep_byte_mapping_and_motion_boundaries() {
    let wide = Style {
        foreground: Color::Indexed(2),
        ..Style::default()
    };
    let mut editor = Editor::new(Buffer::from_text("origin"));
    let id = editor.terminal_fixture(
        &[("ab東京cd", wide), ("plain", Style::default())],
        Phase::Exited {
            code: Some(0),
            signal: None,
        },
    );
    // Bytes: a=0 b=1 東=2..5 京=5..8 c=8 d=9. Motions land on cluster
    // starts only — never byte 3 or 4 inside 東.
    editor.feed_text("ll");
    assert_eq!(editor.head(), 2);
    editor.feed_text("l");
    assert_eq!(editor.head(), 5);
    editor.feed_text("l");
    assert_eq!(editor.head(), 8);
    // Every byte of a wide cluster resolves to that cluster's cell style.
    let at = |byte| {
        editor
            .terminal_style_at(id, byte)
            .map(|(style, _)| style.foreground)
    };
    assert_eq!(at(2), Some(Color::Indexed(2)));
    assert_eq!(at(3), Some(Color::Indexed(2)));
    assert_eq!(at(4), Some(Color::Indexed(2)));
    // A visual yank across the cluster boundary takes whole clusters.
    editor.feed_text("0vll");
    editor.feed_text("y");
    assert_eq!(editor.register(None).text, "ab東");
}
