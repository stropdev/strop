//! Headless driver (0006 tier 2 prototype): scripted keys in, cell-grid
//! frames + state JSON out. Deterministic — no PTY, no timing.
//!
//! R11: every external input and delivery enters through the tape's
//! recording wrapper (`recorded_action`), so a `--log-content` capture of
//! a headless run is a complete forensic recording and `--replay`
//! reproduces it without the script, the clock or the host.

mod driver;
pub use driver::run_script;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::editor::Editor;

pub fn render_frame(
    editor: &mut Editor,
    cols: u16,
    rows: u16,
    record_action: bool,
) -> std::io::Result<Terminal<TestBackend>> {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows))?;
    terminal.draw(|frame| crate::editor::trace::frame::draw(editor, frame, record_action))?;
    editor.tape.healthy()?;
    Ok(terminal)
}

pub fn frame_string(editor: &mut Editor, cols: u16, rows: u16) -> std::io::Result<String> {
    let terminal = render_frame(editor, cols, rows, true)?;
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..rows {
        for x in 0..cols {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    Ok(out)
}

pub fn state_json(editor: &Editor) -> String {
    if editor.docs.is_empty() {
        return serde_json::json!({"should_quit":editor.should_quit,"documents":0,"message":editor.message}).to_string();
    }
    serde_json::json!({
        "mode": editor.mode.chip(),
        "cursor": editor.head(),
        "line": editor.buf().line_of(editor.head()) + 1,
        "col": editor.buf().col_of(editor.head()) + 1,
        "pending": editor.pending.text(),
        "message": editor.message,
        "extra_cursors": editor.extra_selections().iter().map(|s| s.head).collect::<Vec<_>>(),
        "panes": editor.panes.len(),
        "active_pane": editor.active_pane,
        "picker": editor.picker_open(),
        "picker_input": editor.picker.as_ref().map(|g| g.picker.input.text.clone()),
        "picker_items": editor.picker.as_ref().map(|g| g.picker.items.len()),
        "picker_streaming": editor.picker.as_ref().map(|g| g.picker.streaming),
        "register": editor.register(None).text,
        "dirty": editor.buf().dirty,
    })
    .to_string()
}

#[cfg(test)]
mod diff_surface_tests {
    use strop_git::{DiffLine, Hunk, LineOrigin};

    fn hunk() -> Hunk {
        Hunk::build(
            1,
            2,
            1,
            3,
            vec![
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Context,
                    old_lineno: Some(1),
                    new_lineno: Some(1),
                    text: "fn a() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Deletion,
                    old_lineno: Some(2),
                    new_lineno: None,
                    text: "fn old() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Addition,
                    old_lineno: None,
                    new_lineno: Some(2),
                    text: "fn new() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Addition,
                    old_lineno: None,
                    new_lineno: Some(3),
                    text: "fn extra() {}".into(),
                },
            ],
        )
    }

    /// Golden shape of a diff surface frame (0010 §4): stats row, hunk
    /// header, both sides' numbers, no raw-patch noise, no prefixes.
    #[test]
    fn diff_surface_frame_shape() {
        let mut e =
            crate::editor::Editor::new_in(strop_core::Buffer::from_text("x\n"), "/recorded".into());
        e.open_diff_surface("delta", "f.rs", vec![hunk()], None);
        let frame = crate::headless::frame_string(&mut e, 80, 20).unwrap();
        assert!(frame.contains(" f.rs +2 -1"), "stats row: {frame}");
        assert!(frame.contains(" @@ -1,2 +1,3 @@"), "hunk header: {frame}");
        // both numbers on context, one side blank on add/del rows
        assert!(
            frame.contains("  1   1 fn a() {}"),
            "context gutter: {frame}"
        );
        assert!(
            frame.contains("▎      2 fn new() {}"),
            "addition gutter: {frame}"
        );
        assert!(
            frame.contains("▎  2     fn old() {}"),
            "deletion gutter: {frame}"
        );
        assert!(
            !frame.contains("+fn new"),
            "no + prefix in content: {frame}"
        );
        assert!(
            !frame.contains("diff --git"),
            "no raw patch header: {frame}"
        );
    }

    /// `q` hands the cursor back to the buffer the surface opened from.
    #[test]
    fn surface_close_restores_cursor() {
        let mut e = crate::editor::Editor::new_in(
            strop_core::Buffer::from_text("a\nb\nc\n"),
            "/recorded".into(),
        );
        e.feed_text("jj"); // line 3
        e.open_diff_surface("hunk", "hunk", vec![hunk()], None);
        assert_eq!(
            e.buf().line_of(e.head()),
            0,
            "surface starts at its own top"
        );
        e.feed_text("q");
        assert_eq!(e.current(), e.first_doc());
        assert_eq!(e.buf().line_of(e.head()), 2, "cursor returned to line 3");
    }
}

#[cfg(test)]
mod quit_tests {
    #[test]
    fn quit_then_frame_does_not_panic() {
        let mut e =
            crate::editor::Editor::new_in(strop_core::Buffer::from_text("x\n"), "/recorded".into());
        let mut out = Vec::new();
        crate::headless::run_script(&mut e, "keys :q!<cr>\nframe\n", 60, 10, &mut out).unwrap();
        assert!(e.should_quit);
    }
}
