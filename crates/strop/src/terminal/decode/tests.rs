use super::*;

fn decode(reads: &[&[u8]]) -> Vec<Decoded> {
    let mut decoder = Decoder::default();
    let mut out = Vec::new();
    for read in reads {
        decoder.feed(read, false, &mut out);
    }
    out
}

fn keys(reads: &[&[u8]]) -> Vec<(KeyCode, Modifiers)> {
    decode(reads)
        .into_iter()
        .map(|event| match event {
            Decoded::Key(key) => (key.code, key.modifiers),
            other => panic!("not a key: {other:?}"),
        })
        .collect()
}

const NONE: Modifiers = Modifiers {
    shift: false,
    control: false,
    alt: false,
    super_key: false,
    hyper: false,
    meta: false,
};
const ALT: Modifiers = Modifiers { alt: true, ..NONE };
const CTRL: Modifiers = Modifiers {
    control: true,
    ..NONE
};
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..NONE
};

#[test]
fn two_escapes_in_one_read_are_two_escape_keys() {
    // B4: crossterm reported one Esc for `\x1b\x1b`.
    assert_eq!(
        keys(&[b"\x1b\x1b"]),
        [(KeyCode::Escape, NONE), (KeyCode::Escape, NONE)]
    );
    assert_eq!(
        keys(&[b"\x1b\x1b\x1b"]),
        [(KeyCode::Escape, NONE); 3],
        "every Escape counts"
    );
}

#[test]
fn escape_then_a_key_is_alt_and_a_lone_escape_is_escape() {
    assert_eq!(keys(&[b"\x1bx"]), [(KeyCode::Char('x'), ALT)]);
    assert_eq!(
        keys(&[b"\x1bX"]),
        [(
            KeyCode::Char('X'),
            Modifiers {
                alt: true,
                shift: true,
                ..NONE
            }
        )]
    );
    assert_eq!(keys(&[b"\x1b\r"]), [(KeyCode::Enter, ALT)]);
    assert_eq!(keys(&[b"\x1b"]), [(KeyCode::Escape, NONE)]);
    // Esc, then Alt-x: the second ESC starts the alt chord.
    assert_eq!(
        keys(&[b"\x1b\x1bx"]),
        [(KeyCode::Escape, NONE), (KeyCode::Char('x'), ALT)]
    );
}

#[test]
fn a_full_read_waits_for_the_rest_of_a_sequence() {
    let mut decoder = Decoder::default();
    let mut out = Vec::new();
    decoder.feed(b"a\x1b", true, &mut out);
    assert_eq!(out.len(), 1, "the trailing ESC may begin a sequence");
    decoder.feed(b"[A", false, &mut out);
    assert_eq!(out[1], plain(KeyCode::Up));
    // CSI and UTF-8 split across reads always wait.
    assert_eq!(keys(&[b"\x1b[1;5", b"C"]), [(KeyCode::Right, CTRL)]);
    assert_eq!(keys(&[b"\xe7\x95", b"\x8c"]), [(KeyCode::Char('界'), NONE)]);
}

#[test]
fn control_bytes_and_text() {
    assert_eq!(
        keys(&[b"\x01\r\t\x7f\x00\x1e"]),
        [
            (KeyCode::Char('a'), CTRL),
            (KeyCode::Enter, NONE),
            (KeyCode::Tab, NONE),
            (KeyCode::Backspace, NONE),
            (KeyCode::Char(' '), CTRL),
            (KeyCode::Char('6'), CTRL),
        ]
    );
    assert_eq!(
        keys(&["aé".as_bytes()]),
        [(KeyCode::Char('a'), NONE), (KeyCode::Char('é'), NONE)]
    );
}

#[test]
fn legacy_cursor_function_and_editing_keys() {
    assert_eq!(
        keys(&[b"\x1b[A\x1bOB\x1b[1;3D\x1b[3~\x1b[5;5~\x1b[15~\x1bOQ\x1b[Z\x1b[[A"]),
        [
            (KeyCode::Up, NONE),
            (KeyCode::Down, NONE),
            (KeyCode::Left, ALT),
            (KeyCode::Delete, NONE),
            (KeyCode::PageUp, CTRL),
            (KeyCode::Function(5), NONE),
            (KeyCode::Function(2), NONE),
            (KeyCode::BackTab, SHIFT),
            (KeyCode::Function(1), NONE),
        ]
    );
}

#[test]
fn kitty_keys_keep_kind_alternates_and_keypad() {
    // `:` reported as Shift+semicolon with the shifted alternate.
    assert_eq!(keys(&[b"\x1b[59:58;2u"]), [(KeyCode::Char(':'), NONE)]);
    assert_eq!(keys(&[b"\x1b[27u"]), [(KeyCode::Escape, NONE)]);
    assert_eq!(keys(&[b"\x1b[27u\x1b[27u"]), [(KeyCode::Escape, NONE); 2]);
    let [Decoded::Key(release)] = &decode(&[b"\x1b[97;5:3u"])[..] else {
        panic!("one key")
    };
    assert_eq!(
        (release.code, release.modifiers, release.kind),
        (KeyCode::Char('a'), CTRL, KeyKind::Release)
    );
    let [Decoded::Key(keypad)] = &decode(&[b"\x1b[57414u"])[..] else {
        panic!("one key")
    };
    assert_eq!((keypad.code, keypad.state.keypad), (KeyCode::Enter, true));
    assert_eq!(
        keys(&[b"\x1b[57441;2u"]),
        [(KeyCode::Modifier(ModifierKey::LeftShift), SHIFT)]
    );
    assert_eq!(keys(&[b"\x1b[9;2u"]), [(KeyCode::BackTab, SHIFT)]);
}

#[test]
fn paste_focus_and_reports() {
    assert_eq!(
        decode(&[b"\x1b[200~one\x1b\ntwo", b"\x1b[201~x"]),
        [
            Decoded::Paste("one\x1b\ntwo".into()),
            plain(KeyCode::Char('x'))
        ]
    );
    assert_eq!(
        decode(&[b"\x1b[I\x1b[O"]),
        [Decoded::Focus(true), Decoded::Focus(false)]
    );
    // Terminal reports carry no input and never leak as keys.
    assert_eq!(keys(&[b"\x1b[?15u\x1b[12;40R\x1b[?64;1c"]), []);
}
