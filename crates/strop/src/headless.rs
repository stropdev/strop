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
        "cursor": editor.cursor,
        "line": editor.buf().line_of(editor.cursor) + 1,
        "col": editor.buf().col_of(editor.cursor) + 1,
        "pending": editor.pending,
        "picker": editor.picker_open(),
        "picker_input": editor.picker.as_ref().map(|g| g.picker.input.clone()),
        "picker_items": editor.picker.as_ref().map(|g| g.picker.items.len()),
        "picker_streaming": editor.picker.as_ref().map(|g| g.picker.streaming),
        "register": editor.register(None).0,
        "dirty": editor.buf().dirty,
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
        if editor.should_quit {
            break; // nothing to drive; the editor is gone
        }
        if let Some(keys) = line.strip_prefix("keys ") {
            editor.feed_text(keys);
            editor.drain_picker();
        } else if line == "settle" {
            // let streaming sources deliver: drain until Done (bounded)
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                editor.drain_picker();
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
mod tests {
    #[test]
    fn picker_card_renders() {
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("fn main() {}\n"));
        e.open_picker(strop_picker::Kind::Buffers);
        let frame = crate::headless::frame_string(&mut e, 80, 20);
        assert!(frame.contains("buffers"), "{frame}");
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
