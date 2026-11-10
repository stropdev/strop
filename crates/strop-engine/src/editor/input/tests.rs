use super::*;

fn row_events(keys: &str) -> Vec<Vec<Key>> {
    crate::keymap::lookup::expand(keys)
        .iter()
        .map(|seq| {
            let mut out = Vec::new();
            for t in seq {
                match *t {
                    "space" => out.push(Key::Char(' ')),
                    "ctrl-c" => out.push(Key::CtrlC),
                    "ctrl-w" => out.push(Key::CtrlW),
                    "ctrl-o" => out.push(Key::CtrlO),
                    "ctrl-r" => out.push(Key::CtrlR),
                    "ctrl-t" => out.push(Key::CtrlT),
                    "ctrl-d" => out.push(Key::CtrlD),
                    "ctrl-u" => out.push(Key::CtrlU),
                    "ctrl-f" => out.push(Key::CtrlF),
                    "ctrl-b" => out.push(Key::CtrlB),
                    "ctrl-^" => out.push(Key::CtrlCaret),
                    "ctrl-v" => out.push(Key::CtrlV),
                    "tab" => out.push(Key::Tab),
                    "s-tab" => out.push(Key::Backtab),
                    "up" => out.push(Key::Up),
                    "down" => out.push(Key::Down),
                    "left" => out.push(Key::Left),
                    "right" => out.push(Key::Right),
                    "ctrl-x" => out.push(Key::CtrlX),
                    "ctrl-l" => out.push(Key::CtrlL),
                    "enter" => out.push(Key::Enter),
                    t if t.contains("<space>") => {
                        let i = t.find('<').unwrap();
                        for c in t[..i].chars() {
                            out.push(Key::Char(c));
                        }
                        out.push(Key::Char(' '));
                    }
                    t if t.contains('<') => {
                        let i = t.find('<').unwrap();
                        for c in t[..i].chars() {
                            out.push(Key::Char(c));
                        }
                        out.push(Key::Char('x'));
                    }
                    t if t.starts_with(':') || t == "/" || t == "?" => {
                        out.push(Key::Char(t.chars().next().unwrap()))
                    }
                    t => out.extend(t.chars().map(Key::Char)),
                }
            }
            out
        })
        .collect()
}

#[test]
fn zero_is_a_motion_when_no_count2_pending() {
    // vim: d0 deletes to column 0; d10w counts through the 0 (0015)
    let mut w = Walker::new();
    assert!(matches!(w.feed(Key::Char('d')), Action::Pending));
    match w.feed(Key::Char('0')) {
        Action::Grammar(cmd) => {
            assert_eq!(cmd.op, Some(strop_grammar::Op::Delete));
            assert!(matches!(
                cmd.target,
                strop_grammar::Target::Motion(strop_grammar::Motion::LineStart)
            ));
        }
        _ => panic!("d0 must complete as a grammar command"),
    }
    let mut w = Walker::new();
    w.feed(Key::Char('d'));
    w.feed(Key::Char('1'));
    assert!(matches!(w.feed(Key::Char('0')), Action::Pending));
    match w.feed(Key::Char('w')) {
        Action::Grammar(cmd) => assert_eq!(cmd.count, Some(10)),
        _ => panic!("d10w keeps its count"),
    }
}

#[test]
fn counts_cap_under_adversarial_input() {
    let mut w = Walker::new();
    for _ in 0..50 {
        w.feed(Key::Char('9'));
    }
    assert_eq!(w.state.count1, Some(Walker::MAX_COUNT));
}

#[test]
fn display_shows_the_full_state() {
    // a pending operator is never invisible (0015)
    let mut w = Walker::new();
    w.feed(Key::Char('"'));
    w.feed(Key::Char('a'));
    w.feed(Key::Char('2'));
    w.feed(Key::Char('d'));
    w.feed(Key::Char('3'));
    assert_eq!(w.display(), "\"a2d3");
}

/// 0016's core invariant: every live row's sequence walks the
/// machine to a typed action — never Invalid, and the machine
/// grounds afterwards (no state leaks into the next command).
#[test]
fn every_row_completes_and_grounds() {
    for b in crate::keymap::BINDINGS.iter().filter(|b| {
        b.live
            && !matches!(
                b.handler,
                crate::keymap::Handler::Soon | crate::keymap::Handler::Contextual
            )
    }) {
        // Soon rows are surface-only verbs — they dispatch in the
        // readonly layer, not on plain buffers
        for events in row_events(b.keys) {
            if events.is_empty() {
                continue;
            }
            let mut w = Walker::new();
            let mut last = Action::Pending;
            for key in &events {
                last = w.feed(*key);
                if matches!(last, Action::Pending) {
                    continue;
                }
                break;
            }
            assert!(
                !matches!(last, Action::Invalid(_)),
                "{} walked to Invalid",
                b.keys
            );
            // a completed walk grounds; a legitimately pending one
            // (operator waits for its motion) is ALWAYS visible
            if matches!(last, Action::Pending) {
                assert!(
                    !w.display().is_empty(),
                    "{} pends invisibly — the statusline must show it",
                    b.keys
                );
            } else {
                assert!(
                    w.display().is_empty(),
                    "{} left walker state {:?}",
                    b.keys,
                    w.display()
                );
            }
        }
    }
}

/// Adversarial streams: junk at ground never poisons the next
/// command's count (the 2zx class from the second review).
#[test]
fn junk_never_doubles_a_count() {
    for j in ['=', '\\', ',', ';'] {
        let mut w = Walker::new();
        let _ = w.feed(Key::Char('2'));
        let _ = w.feed(Key::Char(j));
        if let Action::Row { count, .. } = w.feed(Key::Char('x')) {
            assert_ne!(count, Some(4), "2{j}x must not double-count");
        }
    }
}

#[test]
fn counts_on_both_sides_of_register_selector_multiply() {
    let original = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
    let mut editor = crate::editor::Editor::new(strop_core::Buffer::from_text(original));

    editor.feed_text("2\"a3yy");

    assert_eq!(
        editor.register(Some('a')).text,
        "one\ntwo\nthree\nfour\nfive\nsix\n"
    );
    assert_eq!(editor.register(None), editor.register(Some('a')));
    assert_eq!(editor.buf().text().to_string(), original);
    assert_eq!(editor.head(), 0);
}
