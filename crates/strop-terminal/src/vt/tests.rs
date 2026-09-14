use super::*;
use strop_core::frontend_input::Input;

fn terminal(columns: u16, rows: u16) -> Vt {
    Vt::new(
        SessionId::from_request(strop_core::worker::WorkerId::new(1)),
        Geometry {
            columns,
            rows,
            revision: 1,
        },
    )
    .unwrap()
}

#[test]
fn soft_wrap_and_grapheme_cells_share_the_readonly_text_projection() {
    let mut vt = terminal(5, 3);
    vt.feed("\x1b[?2027hA👩\u{200d}💻BCx".as_bytes()).unwrap();
    let (frame, _) = vt.snapshot().unwrap();
    let first = &frame.rows[0].row;
    assert!(first.wrapped);
    assert_eq!(first.symbol(1), Some("👩\u{200d}💻"));
    assert_eq!(first.cells[1].width, 2);
    assert_eq!(first.cells[2].width, 0);
    assert_eq!(first.byte_at(1), first.byte_at(2));
    assert!(frame.projection.to_string().starts_with("A👩\u{200d}💻BCx"));
    assert_eq!(frame.cursor.row, 1);
    assert_eq!(frame.cursor.column, 1);
    assert_eq!(frame.cursor_byte(), "A👩\u{200d}💻BCx".len());
}

#[test]
fn continuing_output_cannot_mutate_an_inspection_snapshot() {
    let mut vt = terminal(12, 3);
    vt.feed(b"one\r\ntwo\r\nthree\r\n").unwrap();
    let (before, _) = vt.snapshot().unwrap();
    let before_text = before.projection.to_string();
    vt.feed(b"four\r\nfive\r\nlatest\r\n").unwrap();
    let (after, _) = vt.snapshot().unwrap();
    let after_text = after.projection.to_string();
    assert_eq!(before.projection.to_string(), before_text);
    assert!(!before_text.contains("latest"));
    assert!(after_text.contains("one"));
    assert!(after_text.contains("latest"));
    assert!(after.history_rows > before.history_rows);
}

#[test]
fn erased_cells_retain_their_terminal_background_color() {
    let mut vt = terminal(10, 2);
    vt.feed(b"\x1b[41m\x1b[2J").unwrap();
    let (frame, _) = vt.snapshot().unwrap();
    assert_eq!(frame.rows[0].row.symbol(0), Some(" "));
    assert_eq!(
        frame.rows[0].row.cells[0].style.background,
        Color::Indexed(1)
    );
}

#[test]
fn clipboard_reads_are_denied_and_multiline_paste_requires_explicit_consent() {
    let mut vt = terminal(20, 3);
    let query = vt.feed(b"\x1b]52;c;?\x07").unwrap();
    assert!(query.reply.is_empty());
    let paste = Input::Paste("first\nsecond".into());
    assert!(matches!(
        vt.input(&paste, false),
        Err(Error::PasteNeedsConfirmation)
    ));
    let accepted = vt.input(&paste, true).unwrap();
    assert!(String::from_utf8(accepted.reply)
        .unwrap()
        .contains("second"));
    vt.feed(b"\x1b[?2004h").unwrap();
    let bracketed = vt.input(&paste, false).unwrap().reply;
    assert!(bracketed.starts_with(b"\x1b[200~"));
    assert!(bracketed.ends_with(b"\x1b[201~"));
}

#[test]
fn nul_retains_modifiers_and_negotiated_release_semantics() {
    use strop_core::frontend_input::{KeyCode, KeyEvent, KeyKind};
    let mut vt = terminal(20, 3);
    let mut key = KeyEvent::press(KeyCode::Null);
    key.modifiers.alt = true;
    assert_eq!(vt.input(&Input::Key(key), false).unwrap().reply, b"\x1b\0");
    vt.feed(b"\x1b[>3u").unwrap();
    assert_eq!(
        vt.input(&Input::Key(key), false).unwrap().reply,
        b"\x1b[32;7u"
    );
    key.kind = KeyKind::Release;
    assert_eq!(
        vt.input(&Input::Key(key), false).unwrap().reply,
        b"\x1b[32;7:3u"
    );
}

#[test]
fn oversized_paste_refusal_leaves_the_terminal_encoder_usable() {
    let mut vt = terminal(20, 3);
    vt.feed(b"\x1b[?2004h").unwrap();
    let oversized = Input::Paste("x".repeat(MAX_INPUT_BYTES));
    assert!(matches!(
        vt.input(&oversized, false),
        Err(Error::Capacity(_))
    ));
    assert_eq!(
        vt.input(&Input::Paste("safe".into()), false).unwrap().reply,
        b"\x1b[200~safe\x1b[201~"
    );
}

#[test]
fn replay_admission_checks_real_grapheme_and_projection_correspondence() {
    let mut vt = terminal(5, 3);
    vt.feed("\x1b[?2027hA👩\u{200d}💻BCx".as_bytes()).unwrap();
    let (frame, _) = vt.snapshot().unwrap();
    frame.validate().unwrap();
    let update = Update {
        session: frame.session,
        phase: Phase::Running,
        frame: Some(frame.clone()),
        effects: Vec::new(),
        acknowledged_input: 0,
        warning: None,
    };
    let wire = serde_json::to_value(&update).unwrap();
    let decoded: Update = serde_json::from_value(wire.clone()).unwrap();
    let decoded = decoded.frame.unwrap();
    assert_eq!(decoded.projection, frame.projection);
    assert_eq!(decoded.cell_at_byte(1).unwrap().width, 2);
    let mut corrupt = wire;
    corrupt["frame"]["geometry"]["columns"] = serde_json::json!(4);
    assert!(serde_json::from_value::<Update>(corrupt).is_err());
}

#[test]
fn focus_reports_follow_the_child_negotiated_mode() {
    let mut vt = terminal(20, 3);
    assert!(vt.focus(false).unwrap().reply.is_empty());
    vt.feed(b"\x1b[?1004h").unwrap();
    assert_eq!(vt.focus(false).unwrap().reply, b"\x1b[O");
    assert_eq!(vt.focus(true).unwrap().reply, b"\x1b[I");
    vt.feed(b"\x1b[?1004l").unwrap();
    assert!(vt.focus(true).unwrap().reply.is_empty());
}

#[test]
fn keyboard_advertisement_matches_the_captured_frontend_profile() {
    let mut vt = terminal(20, 3);
    vt.keyboard_capabilities(0).unwrap();
    assert_eq!(vt.feed(b"\x1b[?u\x1b[c").unwrap().reply, b"\x1b[?62;22c");
    vt.feed(b"\x1b[>31u").unwrap();
    assert!(vt.feed(b"\x1b[?u").unwrap().reply.is_empty());
    let mut alt =
        strop_core::frontend_input::KeyEvent::press(strop_core::frontend_input::KeyCode::Char('x'));
    alt.modifiers.alt = true;
    assert_eq!(vt.input(&Input::Key(alt), false).unwrap().reply, b"\x1bx");
    vt.keyboard_capabilities(SUPPORTED_KEYBOARD_FLAGS).unwrap();
    assert_eq!(vt.feed(b"\x1b[?u").unwrap().reply, b"\x1b[?27u");
}
