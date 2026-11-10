use super::*;
use std::time::{Duration, Instant};

use crate::editor::{Document, DocumentSource, Mode};
use strop_core::frontend_input::{Input, Key, KeyCode, KeyEvent, Modifiers};
use strop_core::Buffer;
use strop_terminal::model::{Color, Style, Update};

mod context_controls;
mod launch_identity;

fn retained_terminal(editor: &mut Editor) -> DocumentId {
    editor.terminal_fixture(
        &[("safe", Style::default())],
        Phase::Exited {
            code: Some(0),
            signal: None,
        },
    )
}

/// Drive the real service loop until `condition` holds; failure to
/// converge is a test failure with the editor state attached, never a hang.
fn drain_until(editor: &mut Editor, condition: impl Fn(&Editor) -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        editor.drain_terminals();
        if condition(editor) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        condition(editor),
        "terminal never reached {what}: {}",
        editor.message
    );
}

fn live_text(editor: &Editor, document: DocumentId) -> String {
    editor
        .terminal_frame(document)
        .map(|frame| frame.projection.to_string())
        .unwrap_or_default()
}

fn ctrl(code: KeyCode) -> Input {
    Input::Key(KeyEvent {
        code,
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..KeyEvent::press(code)
    })
}

/// The one escape chord: `Ctrl-\ Ctrl-N`.
fn to_normal(editor: &mut Editor) {
    editor.feed_terminal(ctrl(KeyCode::Char('\\')));
    editor.feed_terminal(ctrl(KeyCode::Char('n')));
    assert!(!editor.terminal_input_active());
    assert_eq!(editor.mode, Mode::Normal);
}

fn publish(editor: &mut Editor, id: DocumentId, rows: &[&str], revision: u64, origin: u64) {
    let session = editor.terminal_document(id).unwrap().session;
    let rows: Vec<(&str, Style)> = rows.iter().map(|row| (*row, Style::default())).collect();
    let frame = fixture_frame(session, editor.presentation.theme, &rows, revision, origin);
    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Running,
        frame: Some(frame),
        effects: Vec::new(),
        acknowledged_input: 0,
        warning: None,
    });
}

fn line(editor: &Editor) -> usize {
    editor.buf().line_of(editor.head())
}

/// 0058 WK12: the modal terminal's lifecycle rides the namespace's
/// worker end to end — spawn with geometry, fed input, bounded output,
/// an ordered resize the child's own `stty` attests, and a truthful
/// exit — with emulation and presentation still engine-owned.
#[test]
fn terminal_journey_runs_through_the_namespace_worker() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.feed_text(":terminal\r");
    let document = editor.current();
    assert!(matches!(
        editor.doc(document).source,
        DocumentSource::Terminal(_)
    ));
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Running)),
        "the worker-admitted shell",
    );
    editor.feed_terminal(Input::Text("printf 'HI:%s\\n' x\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("HI:x"),
        "the fed command's output",
    );
    // The ordered resize boundary: after it, the child itself reports
    // the new winsize, and the projection keeps parsing truthfully.
    editor.prepare_terminal_geometry(document, 100, 30);
    drain_until(
        &mut editor,
        |editor| {
            editor
                .terminal_frame(document)
                .is_some_and(|frame| frame.geometry.columns == 100 && frame.geometry.rows == 30)
        },
        "the applied resize",
    );
    editor.feed_terminal(Input::Text("stty size\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("30 100"),
        "the child's attestation of the new geometry",
    );
    editor.feed_terminal(Input::Text("exit 3\n".into()));
    drain_until(
        &mut editor,
        |editor| {
            matches!(
                editor.terminal_phase(document),
                Some(Phase::Exited {
                    code: Some(3),
                    signal: None
                })
            )
        },
        "the truthful exit status",
    );
}

/// 0058 WK12 regression: a tiny-write flood (`yes` writes 2 bytes per
/// syscall) coalesces at the worker's pump instead of drowning the
/// bounded stream in per-write chunks — the tail always lands.
#[test]
fn terminal_flood_coalesces_without_losing_the_tail() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.feed_text(":terminal yes | head -c 16384; printf 'FLOOD-DONE\\n'; exit 0\r");
    let document = editor.current();
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("FLOOD-DONE"),
        "the flood completing",
    );
    drain_until(
        &mut editor,
        |editor| {
            matches!(
                editor.terminal_phase(document),
                Some(Phase::Exited { code: Some(0), .. })
            )
        },
        "exit",
    );
}

/// A one-shot command retains its final frame in the terminal document
/// with its exit status; `i` has no child to enter.
#[test]
fn terminal_command_exit_retains_the_final_frame() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.feed_text(":terminal printf 'DONE\\n'; exit 5\r");
    let document = editor.current();
    drain_until(
        &mut editor,
        |editor| {
            matches!(
                editor.terminal_phase(document),
                Some(Phase::Exited {
                    code: Some(5),
                    signal: None
                })
            )
        },
        "the command's exit",
    );
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("DONE"),
        "the final published frame",
    );
    assert!(!editor.terminal_input_active());
    assert!(editor.buf().text().to_string().contains("DONE"));
    editor.feed(Key::Char('i'));
    assert!(!editor.terminal_input_active());
    assert!(editor.message.contains("exited"), "{}", editor.message);
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
fn terminal_buffer_cannot_be_made_writable_saved_or_implicitly_relaunched() {
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

/// 0090 §4: in TERMINAL mode every key is the child's — Ctrl-W, F2 and
/// Esc included. `Ctrl-\ Ctrl-N` is the only way out.
#[test]
fn only_ctrl_backslash_ctrl_n_leaves_terminal_mode() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(&[("safe", Style::default())], Phase::Running);
    assert!(editor.enter_terminal_input());
    for input in [
        ctrl(KeyCode::Char('w')),
        Input::Key(KeyEvent::press(KeyCode::Function(2))),
        Input::Key(KeyEvent::press(KeyCode::Escape)),
    ] {
        editor.message.clear();
        editor.feed_terminal(input);
        assert!(editor.terminal_input_active());
        // the service-less fixture reports the send instead of moving
        assert_eq!(editor.message, "terminal service unavailable");
    }
    to_normal(&mut editor);
    assert!(editor.terminals.prefix.is_none());
    // No entry message: the echo row would cover the prompt (0089 T12).
    assert_eq!(editor.message, "");
}

/// 0065 S2 (invariant): focus loss cancels a pending escape prefix — a
/// later Ctrl-N goes to the child, never silently completes the escape.
#[test]
fn focus_loss_cancels_a_pending_terminal_escape_prefix() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(&[("safe", Style::default())], Phase::Running);
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    editor.feed_terminal(ctrl(KeyCode::Char('\\')));
    assert!(editor.terminals.prefix.is_some());
    editor.terminal_focus_changed(false);
    assert!(editor.terminals.prefix.is_none());
    editor.terminal_focus_changed(true);
    editor.feed_terminal(ctrl(KeyCode::Char('n')));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.message, "terminal service unavailable");
}

/// 0090 §4: Neovim-style names everywhere a buffer is named — the
/// buffer, its label, the buffer list (with vim's `!` job badge and the
/// live phase).
#[test]
fn terminal_buffers_carry_term_names_in_the_buffer_list() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let id = retained_terminal(&mut editor);
    let expected = format!(
        "term://{}//1:sh",
        strop_workspace::directory::display_path(directory.path())
    );
    assert_eq!(editor.buf().name.as_deref(), Some(expected.as_str()));
    assert_eq!(editor.doc(id).label(directory.path()), expected);
    editor.open_picker(strop_picker::Kind::Buffers);
    let glue = editor.picker.as_ref().unwrap();
    let terminal_item = glue
        .picker
        .items
        .iter()
        .find(|item| item.text.starts_with("term://"))
        .expect("terminal listed in buffers");
    assert_eq!(terminal_item.badge.as_deref(), Some("!"));
    assert!(
        terminal_item.text.ends_with("//1:sh · exited 0"),
        "{}",
        terminal_item.text
    );
}

#[test]
fn private_terminal_command_paste_is_classified_before_action_capture() {
    use crate::editor::trace::drive::Action;
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

/// 0090 §4, the baseline spine: `i` enters TERMINAL, keys go to the
/// child, `Ctrl-\ Ctrl-N` lands in NORMAL on the child's cursor, the
/// buffer stays live, and following is a property of the cursor being
/// on the last line — `G` resumes it, `k` stops it.
#[test]
fn normal_mode_is_a_live_buffer_that_follows_from_the_last_line() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let id = editor.terminal_fixture(
        &[("first", Style::default()), ("second", Style::default())],
        Phase::Running,
    );
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.input_owner(), crate::editor::InputOwner::Terminal);
    assert_eq!(editor.message, "");
    // A raw key routes to the terminal path, never the grammar.
    let head = editor.head();
    editor.feed_terminal(Input::Key(KeyEvent::press(KeyCode::Char('j'))));
    assert_eq!(editor.head(), head);
    assert!(editor.terminal_input_active());
    assert_eq!(editor.message, "terminal service unavailable");
    editor.message.clear();
    to_normal(&mut editor);
    // The fixture's child cursor is row 0, column 0: NORMAL lands there.
    assert_eq!(line(&editor), 0);
    editor.feed_text("yy");
    assert_eq!(editor.register(None).text, "first \n");
    // Output under NORMAL lands in the buffer at once; a cursor away from
    // the last line stays on its content.
    publish(&mut editor, id, &["first", "second", "third"], 2, 0);
    assert!(editor.buf().text().to_string().contains("third"));
    assert_eq!(line(&editor), 0);
    // G: last line, following. New rows carry the cursor along.
    editor.feed_text("G");
    assert_eq!(line(&editor), 2);
    publish(
        &mut editor,
        id,
        &["first", "second", "third", "fourth"],
        3,
        0,
    );
    assert_eq!(line(&editor), 3);
    publish(
        &mut editor,
        id,
        &["first", "second", "third", "fourth", "fifth"],
        4,
        0,
    );
    assert_eq!(line(&editor), 4);
    // k stops following: the cursor keeps its line through later output.
    editor.feed_text("k");
    assert_eq!(line(&editor), 3);
    publish(
        &mut editor,
        id,
        &["first", "second", "third", "fourth", "fifth", "sixth"],
        5,
        0,
    );
    assert_eq!(line(&editor), 3);
    assert!(editor.buf().text().to_string().contains("sixth"));
    // i returns to TERMINAL at the child's cursor.
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.head(), 0);
}

/// Scrollback drops remap the cursor and marks by stream byte: the same
/// content stays addressed, with no explicit refresh anywhere.
#[test]
fn live_updates_remap_cursor_and_marks_across_scrollback_drops() {
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
    publish(&mut editor, id, &["kept-one", "kept-two", "fresh"], 2, 9);
    assert!(editor.buf().text().to_string().starts_with("kept-one"));
    assert_eq!(editor.head(), 0);
    assert_eq!(line(&editor), 0);
    editor.feed_text("G'a");
    assert_eq!(editor.head(), 0);
}

/// 0090 §4: switching away and back restores the cursor and viewport
/// in NORMAL mode — never 1:1 (0089 B2/T2/T3) — and a view that was at
/// the bottom keeps following output that arrived meanwhile.
#[test]
fn re_entry_restores_the_view_and_lands_in_normal() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let origin = editor.current();
    editor.view_rows = 5;
    let rows: Vec<String> = (0..30).map(|index| format!("row {index:02}")).collect();
    let refs: Vec<(&str, Style)> = rows
        .iter()
        .map(|row| (row.as_str(), Style::default()))
        .collect();
    let id = editor.terminal_fixture(&refs, Phase::Running);
    editor.feed(Key::Char('i'));
    to_normal(&mut editor);
    editor.feed_text(":13\r");
    editor.scroll_to_cursor(5);
    assert_eq!(line(&editor), 12);
    let top = editor.view_top();
    assert!(top > 0);
    editor.feed(Key::CtrlCaret);
    assert_eq!(editor.current(), origin);
    editor.feed(Key::CtrlCaret);
    assert_eq!(editor.current(), id);
    assert!(!editor.terminal_input_active());
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(line(&editor), 12);
    assert_eq!(editor.view_top(), top);
    // From TERMINAL mode: away, output arrives, back — NORMAL at the new
    // last line, still following.
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    editor.switch_to(origin);
    let mut more = rows.clone();
    more.push("row 30".into());
    let refs: Vec<&str> = more.iter().map(String::as_str).collect();
    publish(&mut editor, id, &refs, 2, 0);
    editor.switch_to(id);
    assert!(!editor.terminal_input_active());
    assert_eq!(line(&editor), 30);
}

/// A live terminal buffer moves its revision under a deferred motion.
/// The motion is resolved again on the new text and the keys typed
/// behind it are kept — `i` still lands in TERMINAL mode — instead of
/// being dropped as a stale delivery (0090 §4).
#[test]
fn output_under_a_pending_motion_keeps_the_typed_keys() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let rows: Vec<String> = (0..200)
        .map(|index| format!("row {index:03} {}", "x".repeat(60)))
        .collect();
    let refs: Vec<(&str, Style)> = rows
        .iter()
        .map(|row| (row.as_str(), Style::default()))
        .collect();
    let id = editor.terminal_fixture(&refs, Phase::Running);
    editor.resolution.enabled = true;
    assert!(
        editor.buf().len_bytes() > 4096,
        "large enough to resolve asynchronously"
    );
    editor.feed(Key::Char('i'));
    to_normal(&mut editor);
    editor.feed_text("gg");
    editor.feed(Key::Char('j'));
    assert!(
        editor.resolution.blocked(),
        "the motion resolves off the input thread"
    );
    editor.feed(Key::Char('i'));
    assert!(!editor.terminal_input_active(), "i waits behind the motion");
    // Output lands while the motion is in flight.
    let mut more: Vec<&str> = rows.iter().map(String::as_str).collect();
    more.push("row 200 fresh");
    publish(&mut editor, id, &more, 2, 0);
    while editor.resolution.pending() {
        if !editor.resolution.blocked() && !editor.resolution.queue.is_empty() {
            editor.resume_deferred_input();
        } else {
            let event = editor
                .resolution
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .expect("grammar worker settles");
            editor.handle_resolution(event);
        }
    }
    assert!(editor.terminal_input_active(), "{}", editor.message);
    assert!(!editor.message.contains("cancelled"), "{}", editor.message);
}

/// 0090 §4 (0089 B7/T7): `:qa` with running sessions is one notice, no
/// picker; `:qa!` stops them and quits.
#[test]
fn qa_with_running_terminals_is_one_notice_and_qa_bang_stops_them() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(&[("one", Style::default())], Phase::Running);
    editor.terminal_fixture(&[("two", Style::default())], Phase::Running);
    editor.feed_text(":qa\r");
    assert!(editor.picker.is_none());
    assert!(!editor.should_quit());
    assert_eq!(editor.message, "2 terminals running — :qa! stops them");
    editor.feed_text(":qa!\r");
    assert!(editor.should_quit());
    assert!(editor
        .terminals
        .entries
        .values()
        .all(|entry| entry.phase == Phase::Closing));
}

/// 0090 §4: `:q` hides a running terminal, `:bd` refuses, `:bd!` kills.
#[test]
fn q_hides_a_running_terminal_and_bd_bang_kills_it() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    let origin = editor.current();
    let id = editor.terminal_fixture(&[("live", Style::default())], Phase::Running);
    editor.feed_text(":q\r");
    assert_eq!(editor.current(), origin);
    assert!(editor.terminal_phase(id).unwrap().live());
    editor.switch_to(id);
    editor.feed_text(":bd\r");
    assert_eq!(editor.current(), id);
    assert!(editor.message.contains(":bd!"), "{}", editor.message);
    editor.feed_text(":bd!\r");
    assert_eq!(editor.current(), origin);
    assert!(editor.docs.get(id).is_none());
    assert!(editor.terminals.entries.is_empty());
    // Last buffer: :q refuses, :q! stops and quits.
    let id = editor.terminal_fixture(&[("live", Style::default())], Phase::Running);
    editor.switch_to(origin);
    editor.feed_text(":bd!\r");
    assert_eq!(editor.current(), id);
    editor.feed_text(":q\r");
    assert!(!editor.should_quit());
    assert!(editor.message.contains(":q!"), "{}", editor.message);
    editor.feed_text(":q!\r");
    assert!(editor.should_quit());
}

#[test]
fn terminal_insert_keys_and_edit_refusals_explain_mode_switching() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(&[("safe", Style::default())], Phase::Running);
    for key in ['i', 'I', 'a', 'A'] {
        editor.feed(Key::Char(key));
        assert!(editor.terminal_input_active(), "{key} enters input");
        to_normal(&mut editor);
    }
    editor.feed_text("vc");
    assert_eq!(editor.mode, Mode::Normal);
    assert!(editor.message.contains("TERMINAL"), "{}", editor.message);
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
    editor.enter_terminal_input();
    to_normal(&mut editor);
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
