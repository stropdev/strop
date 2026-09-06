//! Output-side trace: frame geometry and cell/style fingerprint always; cell
//! glyphs only with explicit full-content capture.
use crate::editor::Editor;
use ratatui::Frame;
use serde_json::json;
use std::hash::{Hash, Hasher};
use std::time::Instant;
use strop_trace::{capture_content, enabled, record, EventKind};

thread_local! {
    static CURSOR: std::cell::Cell<Option<(u16,u16)>> = const { std::cell::Cell::new(None) };
}

pub fn place_cursor(frame: &mut Frame, position: (u16, u16)) {
    if enabled() {
        CURSOR.with(|cursor| cursor.set(Some(position)));
    }
    frame.set_cursor_position(position);
}

pub fn draw(editor: &mut Editor, frame: &mut Frame) {
    let started = enabled().then(Instant::now);
    if started.is_some() {
        CURSOR.with(|cursor| cursor.set(None));
    }
    crate::render::render(editor, frame);
    let Some(started) = started else { return };
    let area = frame.area();
    let cursor = CURSOR.with(std::cell::Cell::get);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let grid = frame.buffer_mut();
    for cell in &grid.content {
        cell.symbol().hash(&mut hash);
        cell.fg.hash(&mut hash);
        cell.bg.hash(&mut hash);
        cell.modifier.bits().hash(&mut hash);
    }
    let rows = capture_content().then(|| {
        (area.y..area.bottom())
            .map(|y| {
                let cells: Vec<_> = (area.x..area.right())
                    .map(|x| {
                        let cell = &grid[(x, y)];
                        json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),
                    "bg":format!("{:?}",cell.bg),"modifiers":cell.modifier.bits()})
                    })
                    .collect();
                cells
            })
            .collect::<Vec<_>>()
    });
    record(
        EventKind::Render,
        &json!({
            "duration_us":started.elapsed().as_micros(),"columns":area.width,"rows":area.height,
            "cursor":cursor.map(|(column,row)|json!({"column":column,"row":row})),
            "cell_hash":format!("{:016x}",hash.finish()),"cells":rows,
            "active_pane":editor.active_pane,
        }),
    );
}
