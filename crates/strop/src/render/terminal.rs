use crate::editor::Editor;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Paragraph,
    Frame,
};
use strop_core::id::DocumentId;
use strop_terminal::model::{Color as TerminalColor, Palette, Rgb, Style as TerminalStyle};

fn rgb(value: Rgb) -> Color {
    Color::Rgb(value.red, value.green, value.blue)
}
fn color(value: TerminalColor, default: Rgb, palette: &Palette) -> Color {
    match value {
        TerminalColor::Default => rgb(default),
        TerminalColor::Rgb(value) => rgb(value),
        TerminalColor::Indexed(index) => {
            debug_assert_eq!(palette.colors.len(), 256);
            rgb(palette
                .colors
                .get(usize::from(index))
                .copied()
                .unwrap_or(default))
        }
    }
}
pub(super) fn style(value: TerminalStyle, palette: &Palette) -> Style {
    let mut foreground = color(value.foreground, palette.foreground, palette);
    let mut background = color(value.background, palette.background, palette);
    if value.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    let mut style = Style::default().fg(foreground).bg(background);
    for (enabled, modifier) in [
        (value.bold, Modifier::BOLD),
        (value.faint, Modifier::DIM),
        (value.italic, Modifier::ITALIC),
        (value.underline, Modifier::UNDERLINED),
        (value.invisible, Modifier::HIDDEN),
        (value.strikethrough, Modifier::CROSSED_OUT),
    ] {
        if enabled {
            style = style.add_modifier(modifier);
        }
    }
    style
}

pub(super) fn render(
    editor: &Editor,
    output: &mut Frame,
    area: Rect,
    document: DocumentId,
    active: bool,
) {
    let Some(frame) = editor.terminal_frame(document, true) else {
        output.render_widget(
            Paragraph::new("terminal starting")
                .style(Style::default().fg(super::MUTED).bg(super::BASE)),
            area,
        );
        return;
    };
    output.render_widget(
        Paragraph::new("").style(Style::default().bg(rgb(frame.palette.background))),
        area,
    );
    let rows = area.height.min(frame.geometry.rows);
    let columns = area.width.min(frame.geometry.columns);
    for row in 0..rows {
        let Some(projected) = frame.rows.get(frame.history_rows + usize::from(row)) else {
            continue;
        };
        for column in 0..columns {
            let Some(cell) = projected.row.cells.get(usize::from(column)) else {
                continue;
            };
            let at = (area.x + column, area.y + row);
            let target = &mut output.buffer_mut()[at];
            target.set_style(style(cell.style, &frame.palette));
            if cell.width == 0 || (cell.width == 2 && column + 1 >= columns) {
                target.set_symbol(" ");
            } else if let Some(symbol) = projected.row.symbol(usize::from(column)) {
                target.set_symbol(symbol);
            }
        }
    }
    if active && frame.cursor.visible && frame.cursor.column < columns && frame.cursor.row < rows {
        super::frame_capture::place_cursor(
            output,
            (area.x + frame.cursor.column, area.y + frame.cursor.row),
        );
    }
}
