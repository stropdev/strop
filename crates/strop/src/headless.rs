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
        .draw(|f| crate::editor::trace::frame::draw(editor, f))
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
    if editor.docs.is_empty() {
        return serde_json::json!({"should_quit":editor.should_quit,"documents":0,"message":editor.message}).to_string();
    }
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
pub fn run_script(
    editor: &mut Editor,
    script: &str,
    cols: u16,
    rows: u16,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let mut steps = script
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .peekable();
    if let Some(text) = steps.peek().and_then(|line| line.strip_prefix("buffer ")) {
        let text: String = serde_json::from_str(text).map_err(std::io::Error::other)?;
        let configuration = std::mem::take(&mut editor.config);
        *editor = Editor::new(strop_core::Buffer::from_text(&text));
        editor.config = configuration;
        steps.next();
    }
    let mut terminal = Terminal::new(TestBackend::new(cols, rows))?;
    editor.trace_state();
    editor.lsp_maybe_attach();
    draw(editor, &mut terminal)?;
    for line in steps {
        if editor.should_quit {
            break;
        }
        if let Some(keys) = line.strip_prefix("keys ") {
            for key in crate::editor::keys::parse(keys) {
                editor.feed(key);
                after_input(editor, &mut terminal)?;
                if editor.should_quit {
                    break;
                }
            }
        } else if let Some(key) = line.strip_prefix("key ") {
            let key = serde_json::from_str(key).map_err(std::io::Error::other)?;
            editor.feed(key);
            after_input(editor, &mut terminal)?;
        } else if let Some(text) = line.strip_prefix("paste ") {
            let text = serde_json::from_str(text).map_err(std::io::Error::other)?;
            editor.handle_app_event(crate::editor::events::AppEvent::Paste(text));
            after_input(editor, &mut terminal)?;
        } else if line == "quit-intent" {
            editor.handle_app_event(crate::editor::events::AppEvent::QuitIntent);
            after_input(editor, &mut terminal)?;
        } else if let Some(size) = line.strip_prefix("resize ") {
            let values: Result<Vec<u16>, _> = size.split_whitespace().map(str::parse).collect();
            let values = values.map_err(std::io::Error::other)?;
            let [columns, rows] = values.as_slice() else {
                return Err(std::io::Error::other("resize requires columns and rows"));
            };
            terminal.backend_mut().resize(*columns, *rows);
            strop_trace::record_with(
                strop_trace::EventKind::Resize,
                || serde_json::json!({"columns":columns,"rows":rows}),
            );
            draw(editor, &mut terminal)?;
        } else if line == "settle" || line.starts_with("wait ") {
            let milliseconds: u64 = if line == "settle" {
                2000
            } else {
                line[5..].trim().parse().map_err(std::io::Error::other)?
            };
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(milliseconds);
            loop {
                drain(editor);
                draw(editor, &mut terminal)?;
                if std::time::Instant::now() >= deadline
                    || (line == "settle"
                        && !editor
                            .picker
                            .as_ref()
                            .is_some_and(|glue| glue.picker.streaming))
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        } else if line == "frame" {
            drain(editor);
            draw(editor, &mut terminal)?;
            let buffer = terminal.backend().buffer();
            writeln!(
                out,
                "─── frame {}×{}",
                buffer.area.width, buffer.area.height
            )?;
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    write!(out, "{}", buffer[(x, y)].symbol())?;
                }
                writeln!(out)?;
            }
        } else if line == "state" {
            writeln!(out, "─── state {}", state_json(editor))?;
        } else {
            return Err(std::io::Error::other(format!(
                "unknown script command: {line}"
            )));
        }
    }
    Ok(())
}

fn drain(editor: &mut Editor) {
    editor.drain_shell();
    editor.drain_picker();
    editor.drain_git_jobs();
    editor.drain_lsp();
    editor.drain_clipboard();
    editor.trace_state();
}

fn draw(editor: &mut Editor, terminal: &mut Terminal<TestBackend>) -> std::io::Result<()> {
    if !editor.should_quit && !editor.docs.is_empty() {
        if std::mem::take(&mut editor.needs_repaint) {
            terminal.clear()?;
        }
        terminal.draw(|frame| crate::editor::trace::frame::draw(editor, frame))?;
    }
    Ok(())
}

fn after_input(editor: &mut Editor, terminal: &mut Terminal<TestBackend>) -> std::io::Result<()> {
    if !editor.should_quit && !editor.docs.is_empty() {
        editor.lsp_sync_changed();
        drain(editor);
    }
    draw(editor, terminal)
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
        crate::headless::run_script(&mut e, "keys :q!<cr>\nframe\n", 60, 10, &mut out).unwrap();
        assert!(e.should_quit);
    }
}
