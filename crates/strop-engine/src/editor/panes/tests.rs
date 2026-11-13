use super::*;
use strop_core::Buffer;

fn outline(e: &Editor) -> String {
    e.layout().outline().to_string()
}

#[test]
fn vsplit_shares_buffer_and_navigates() {
    // unique path: parallel tests sharing a fixture file race
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("vsplit-a.rs");
    std::fs::write(&a, "fn a() {}\nfn b() {}\n").unwrap();
    let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
    e.feed_text("j"); // line 2
    e.feed_text(":vs<cr>");
    assert_eq!(e.panes.len(), 2);
    assert_eq!(e.active_pane, 1);
    // the new pane shows the same buffer from its own view
    e.feed_text("gg");
    // Space w 1 returns to the first pane — it kept its cursor
    e.feed_text(" w1");
    assert_eq!(e.active_pane, 0);
    assert_eq!(e.buf().line_of(e.head()), 1, "pane 1 kept its own cursor");
    // :q closes the pane, buffer stays
    e.feed_text(":q<cr>");
    assert_eq!(e.panes.len(), 1);
    assert_eq!(e.docs.len(), 1);
}

#[test]
fn split_with_path_opens_other_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("split-a.rs");
    let b = dir.path().join("split-b.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
    e.feed_text(&format!(":vs {}<cr>", b.display()));
    e.wait_io().unwrap();
    assert_eq!(e.panes.len(), 2);
    e.feed_text(" w1");
    assert_eq!(e.buf().path.as_deref(), Some(a.as_path()));
}

#[test]
fn space_w_splits_focuses_and_numbers() {
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    e.feed_text(" wv");
    assert_eq!(e.panes.len(), 2);
    assert_eq!(outline(&e), r#"["row",[1,2]]"#);
    e.feed_text(" w-");
    assert_eq!(e.panes.len(), 3);
    assert_eq!(
        outline(&e),
        r#"["row",[1,["col",[2,3]]]]"#,
        "- splits pane 2 only"
    );
    e.feed_text(" w1");
    assert_eq!(e.active_pane(), 0);
    e.feed_text(" w3");
    assert_eq!(e.active_pane(), 2);

    e.feed_text(" w9");
    assert_eq!(e.active_pane(), 2);
    assert_eq!(e.message(), "no pane 9");
    e.feed_text(" wq");
    assert_eq!(e.panes.len(), 2);
}

/// 0091 §7 (0.56.1 acceptance): every `ctrl-w {key}` either
/// dispatches a window command or changes nothing and reports
/// "ctrl-w {key} is not bound". The second key never reaches Normal.
#[test]
fn every_ctrl_w_key_is_a_window_command_or_reports_unbound() {
    use strop_core::frontend_input::Key;
    const BOUND: &str = "hjklwWpsvcoqn=";
    let fresh = || {
        let mut e = Editor::new(Buffer::from_text("alpha\nbeta\ngamma\n"));
        e.feed_text(":vs<cr>");
        e.feed_text("j");
        e
    };
    for key in (b'!'..=b'~').map(char::from) {
        if key.is_ascii_digit() || key == '"' {
            continue; // counts and the register selector stay the walker's
        }
        let mut e = fresh();
        let before = (
            e.panes.len(),
            e.active_pane(),
            e.head(),
            e.buf().text().to_string(),
            e.mode,
        );
        e.feed(Key::CtrlW);
        e.feed(Key::Char(key));
        assert!(e.walker.is_ground(), "ctrl-w {key} leaves nothing pending");
        if BOUND.contains(key) {
            assert!(
                !e.message().contains("not bound") && !e.message().contains("not an editor"),
                "ctrl-w {key}: {}",
                e.message()
            );
        } else {
            assert_eq!(e.message(), format!("ctrl-w {key} is not bound"));
            let after = (
                e.panes.len(),
                e.active_pane(),
                e.head(),
                e.buf().text().to_string(),
                e.mode,
            );
            assert_eq!(after, before, "ctrl-w {key} changed something");
        }
    }
}

#[test]
fn ctrl_w_commands_drive_the_pane_machinery() {
    use strop_core::frontend_input::Key;
    let window = |e: &mut Editor, key: Key| {
        e.feed(Key::CtrlW);
        e.feed(key);
    };
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    window(&mut e, Key::Char('v'));
    assert_eq!(outline(&e), r#"["row",[1,2]]"#);
    assert_eq!(e.active_pane(), 1);
    window(&mut e, Key::Char('h'));
    assert_eq!(e.active_pane(), 0);
    // held Ctrl: ctrl-w ctrl-w, and ctrl-h as a legacy Backspace
    window(&mut e, Key::CtrlW);
    assert_eq!(e.active_pane(), 1);
    window(&mut e, Key::Backspace);
    assert_eq!(e.active_pane(), 0);
    window(&mut e, Key::Char('\x0c'));
    assert_eq!(e.active_pane(), 1, "a raw ctrl-l is l");
    window(&mut e, Key::Char('v'));
    assert_eq!(e.active_pane(), 2);
    window(&mut e, Key::Char('h'));
    window(&mut e, Key::Char('h'));
    assert_eq!(e.active_pane(), 0);
    window(&mut e, Key::Char('p'));
    assert_eq!(e.active_pane(), 1, "p returns to the last used window");
    window(&mut e, Key::Char('W'));
    assert_eq!(e.active_pane(), 0);
    window(&mut e, Key::Char('c'));
    assert_eq!(e.panes.len(), 2);
    window(&mut e, Key::Char('n'));
    assert_eq!(outline(&e), r#"["row",[["col",[1,2]],3]]"#);
    assert_eq!(e.active_pane(), 1, "n opens below the pane it split");
    assert_eq!(e.buf().text().to_string(), "", "n opens an empty buffer");
    window(&mut e, Key::Char('o'));
    assert_eq!((e.panes.len(), e.active_pane()), (1, 0));
    window(&mut e, Key::Char('c'));
    assert_eq!(e.panes.len(), 1);
    assert_eq!(e.message(), "cannot close the last window");
    // ctrl-c cancels the prefix; the next key is an ordinary one
    e.feed(Key::CtrlW);
    e.feed(Key::CtrlC);
    assert!(e.walker.is_ground());
    // the prefix is consumed in Visual mode too
    e.feed_text(":e!<cr>");
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    e.feed_text("v");
    window(&mut e, Key::Char('x'));
    assert_eq!(e.buf().text().to_string(), "alpha\nbeta\n");
}

mod tree;
