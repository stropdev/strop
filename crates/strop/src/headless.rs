//! Headless driver (0006 tier 2 prototype): scripted keys in, cell-grid
//! frames + state JSON out. Deterministic — no PTY, no timing.

use std::io::Write;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::editor::Editor;

pub fn frame_string(editor: &mut Editor, cols: u16, rows: u16) -> String {
    let backend = TestBackend::new(cols, rows);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|f| crate::render::render(editor, f))
        .expect("draw");
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..rows {
        for x in 0..cols {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

pub fn state_json(editor: &Editor) -> String {
    serde_json::json!({
        "mode": editor.mode.chip(),
        "cursor": editor.head(),
        "line": editor.buf().line_of(editor.head()) + 1,
        "col": editor.buf().col_of(editor.head()) + 1,
        "pending": editor.pending,
        "message": editor.message,
        "extra_cursors": editor.extra_selections().iter().map(|s| s.head).collect::<Vec<_>>(),
        "panes": editor.panes.len(),
        "active_pane": editor.active_pane,
        "picker": editor.picker_open(),
        "picker_input": editor.picker.as_ref().map(|g| g.picker.input.text.clone()),
        "picker_items": editor.picker.as_ref().map(|g| g.picker.items.len()),
        "picker_streaming": editor.picker.as_ref().map(|g| g.picker.streaming),
        "register": editor.register(None).0,
        "dirty": editor.buf().dirty,
    })
    .to_string()
}

/// Script format: one step per line. `keys <text>` feeds keys (token
/// forms: <esc> <cr> <bs> <space> <tab> <s-tab> <up> <down> <left>
/// <right> <c-r> <c-x> <c-d> <c-w> <c-o>); `wait N` ms drains jobs;
/// `settle` waits out streaming pickers; `frame` dumps the screen;
/// `state` dumps JSON. `#` comments. Blank lines ignored.
pub fn run_script(editor: &mut Editor, script: &str, cols: u16, rows: u16, out: &mut dyn Write) {
    for line in script.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if editor.should_quit {
            break; // nothing to drive; the editor is gone
        }
        if let Some(keys) = line.strip_prefix("keys ") {
            editor.feed_text(keys);
            editor.drain_picker();
            editor.drain_git_jobs();
        } else if let Some(ms) = line.strip_prefix("wait ") {
            // drain events for N ms (LSP servers index on their own clock)
            let n: u64 = ms.trim().parse().unwrap_or(500);
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(n);
            while std::time::Instant::now() < deadline {
                editor.drain_shell();
                editor.drain_picker();
                editor.drain_git_jobs();
                editor.drain_lsp();
                editor.drain_clipboard();
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        } else if line == "settle" {
            // let streaming sources deliver: drain until Done (bounded)
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                editor.drain_shell();
                editor.drain_picker();
                editor.drain_git_jobs();
                editor.drain_lsp();
                editor.drain_clipboard();
                let streaming = editor.picker.as_ref().is_some_and(|g| g.picker.streaming);
                if !streaming || std::time::Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        } else if line == "frame" {
            editor.drain_picker();
            let _ = writeln!(out, "─── frame {}×{}", cols, rows);
            let _ = write!(out, "{}", frame_string(editor, cols, rows));
        } else if line == "state" {
            let _ = writeln!(out, "─── state {}", state_json(editor));
        }
    }
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
                    origin: LineOrigin::Context,
                    old_lineno: Some(1),
                    new_lineno: Some(1),
                    text: "fn a() {}".into(),
                },
                DiffLine {
                    origin: LineOrigin::Deletion,
                    old_lineno: Some(2),
                    new_lineno: None,
                    text: "fn old() {}".into(),
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    old_lineno: None,
                    new_lineno: Some(2),
                    text: "fn new() {}".into(),
                },
                DiffLine {
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
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("x\n"));
        e.open_diff_surface("delta", "f.rs", vec![hunk()], None);
        let frame = crate::headless::frame_string(&mut e, 80, 20);
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
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("a\nb\nc\n"));
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
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("x\n"));
        let mut out = Vec::new();
        crate::headless::run_script(&mut e, "keys :q!<cr>\nframe\n", 60, 10, &mut out);
        assert!(e.should_quit);
    }
}
