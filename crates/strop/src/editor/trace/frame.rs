//! Record the actual draw boundary and compare canonical cells during replay.
use crate::editor::Editor;
use ratatui::Frame;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
use std::time::Instant;
use strop_trace::{enabled, record, EventKind};

thread_local! {
    static CURSOR: std::cell::Cell<Option<(u16, u16)>> = const { std::cell::Cell::new(None) };
}

pub fn place_cursor(frame: &mut Frame, position: (u16, u16)) {
    CURSOR.with(|cursor| cursor.set(Some(position)));
    frame.set_cursor_position(position);
}

/// Row runs retain every cell property without repeating the blank viewport.
pub fn frame_observation(
    grid: &ratatui::buffer::Buffer,
    area: ratatui::layout::Rect,
) -> Vec<Vec<Value>> {
    (area.y..area.bottom())
        .map(|y| {
            let mut runs = Vec::new();
            let mut x = area.x;
            while x < area.right() {
                let cell = &grid[(x, y)];
                let mut end = x + 1;
                while end < area.right() && grid[(end, y)] == *cell {
                    end += 1;
                }
                runs.push(json!([
                    end - x,
                    cell.symbol(),
                    color_code(cell.fg),
                    color_code(cell.bg),
                    color_code(cell.underline_color),
                    cell.modifier.bits(),
                    cell.skip
                ]));
                x = end;
            }
            runs
        })
        .collect()
}

fn color_code(color: ratatui::style::Color) -> u32 {
    use ratatui::style::Color;
    match color {
        Color::Reset => 0,
        Color::Black => 1,
        Color::Red => 2,
        Color::Green => 3,
        Color::Yellow => 4,
        Color::Blue => 5,
        Color::Magenta => 6,
        Color::Cyan => 7,
        Color::Gray => 8,
        Color::DarkGray => 9,
        Color::LightRed => 10,
        Color::LightGreen => 11,
        Color::LightYellow => 12,
        Color::LightBlue => 13,
        Color::LightMagenta => 14,
        Color::LightCyan => 15,
        Color::White => 16,
        Color::Indexed(index) => 256 + u32::from(index),
        Color::Rgb(red, green, blue) => {
            (1 << 24) | (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
        }
    }
}

pub fn draw(editor: &mut Editor, frame: &mut Frame, record_action: bool) {
    let started = enabled().then(Instant::now);
    let area = frame.area();
    if record_action && !editor.tape.is_replay() {
        // Render itself can admit hunk/preview work: its action comes first.
        let action = super::drive::Action::Frame {
            columns: area.width,
            rows: area.height,
        };
        if let Err(error) = editor.tape.action(editor.tape.sample_tick(), &action) {
            editor.message = error.to_string();
        }
    }
    CURSOR.with(|cursor| cursor.set(None));
    crate::render::render(editor, frame);
    let cursor = CURSOR.with(std::cell::Cell::get);
    let grid = frame.buffer_mut();
    if let Some(started) = started {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for cell in &grid.content {
            cell.symbol().hash(&mut hash);
            cell.fg.hash(&mut hash);
            cell.bg.hash(&mut hash);
            cell.modifier.bits().hash(&mut hash);
            cell.skip.hash(&mut hash);
        }
        record(
            EventKind::Render,
            &json!({
                "duration_us":started.elapsed().as_micros(),"columns":area.width,"rows":area.height,
                "cursor":cursor.map(|(column,row)|json!({"column":column,"row":row})),
                "cell_hash":format!("{:016x}",hash.finish()),"active_pane":editor.active_pane,
            }),
        );
    }
    if editor.tape.observes() {
        let cells = frame_observation(grid, area);
        if let Err(error) = editor.tape.check(&json!({
            "columns":area.width,"rows":area.height,"cursor":cursor,"cells":cells,
        })) {
            editor.message = error.to_string();
        }
    }
}
