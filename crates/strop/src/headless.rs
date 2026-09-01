//! Headless driver (0006 tier 2 prototype): scripted keys in, cell-grid
//! frames + state JSON out. Deterministic — no PTY, no timing.

use std::io::Write;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::Editor;

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
        "cursor": editor.cursor,
        "line": editor.buf.line_of(editor.cursor) + 1,
        "col": editor.buf.col_of(editor.cursor) + 1,
        "pending": editor.pending,
        "register": editor.register,
        "dirty": editor.buf.dirty,
    })
    .to_string()
}

/// Script format: one step per line. `keys <text>` feeds keys (token
/// forms: <esc> <cr> <bs>); `frame` dumps the screen; `state` dumps JSON.
/// `#` comments. Blank lines ignored.
pub fn run_script(editor: &mut Editor, script: &str, cols: u16, rows: u16, out: &mut dyn Write) {
    for line in script.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(keys) = line.strip_prefix("keys ") {
            editor.feed_text(keys);
        } else if line == "frame" {
            writeln!(out, "─── frame {}×{}", cols, rows).unwrap();
            write!(out, "{}", frame_string(editor, cols, rows)).unwrap();
        } else if line == "state" {
            writeln!(out, "─── state {}", state_json(editor)).unwrap();
        }
    }
}
