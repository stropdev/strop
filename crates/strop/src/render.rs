//! Rendering: gutter, buffer text, overlay layers, statusline.
//! Overlay precedence (0001 §5.8 subset): search/incsearch < operator
//! preview < cursor. One accent color (0001 §4).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use strop_core::Range;
use strop_grammar as grammar;

use crate::app::{Editor, Mode};

// strop default palette (plan 0004 site, --accent amber)
pub const BASE: Color = Color::Rgb(0x16, 0x16, 0x1e);
pub const TEXT: Color = Color::Rgb(0xe8, 0xe4, 0xda);
pub const MUTED: Color = Color::Rgb(0x6b, 0x6f, 0x7e);
pub const ACCENT: Color = Color::Rgb(0xf0, 0xa3, 0x5e);
pub const PREVIEW_BG: Color = Color::Rgb(0x4a, 0x33, 0x1c); // accent, dimmed
pub const FLASH_BG: Color = Color::Rgb(0x6b, 0x47, 0x22); // accent, stronger
pub const SELECT_BG: Color = Color::Rgb(0x2a, 0x2c, 0x3a);

const GUTTER: u16 = 5; // 4-digit numbers + one empty column (0001 §4)

pub fn render(editor: &mut Editor, frame: &mut Frame) {
    let area = frame.area();
    let text_rows = area.height.saturating_sub(1) as usize;
    editor.scroll_to_cursor(text_rows);

    render_text(editor, frame, area, text_rows);
    render_statusline(editor, frame, area);
    place_cursor(editor, frame, area);
}

fn in_range(r: Range, pos: usize) -> bool {
    pos >= r.start && pos < r.end
}

fn render_text(editor: &Editor, frame: &mut Frame, area: Rect, text_rows: usize) {
    let preview = editor.preview().map(|r| r.range);
    let flash = editor.flash_range();
    let selection = editor.visual_range();
    let search_hits: Vec<usize> = editor
        .search_pattern()
        .map(|p| grammar::search_all(&editor.buf, p))
        .unwrap_or_default();
    let find = editor.find_candidates();

    let cur_line = editor.buf.line_of(editor.cursor);
    let mut lines: Vec<Line> = Vec::with_capacity(text_rows);

    for row in 0..text_rows {
        let line_idx = editor.view_top + row;
        if line_idx >= editor.buf.len_lines() {
            lines.push(Line::from(Span::styled("~", Style::default().fg(MUTED))));
            continue;
        }
        let start = editor.buf.line_start(line_idx);
        let text = editor.buf.line_text(line_idx);

        // gutter: muted numbers, current line in accent (0001 §4)
        let num_style = if line_idx == cur_line {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(MUTED)
        };
        let mut spans = vec![Span::styled(format!("{:>4} ", line_idx + 1), num_style)];

        for (i, ch) in text.chars().enumerate() {
            let pos = start + i; // prototype is ASCII-honest (0001 §5.9 later)
            let mut style = Style::default().fg(TEXT);
            if selection.is_some_and(|r| in_range(r, pos)) {
                style = style.bg(SELECT_BG);
            }
            if search_hits
                .iter()
                .any(|&h| pos >= h && pos < h + editor.search_pattern().map_or(0, str::len))
            {
                style = style.fg(ACCENT).add_modifier(Modifier::BOLD);
            }
            if let Some((_, backward)) = find {
                // leap-style: candidates bold-accent on the pending side
                let on_line = editor.buf.line_of(pos) == cur_line;
                let ahead = if backward {
                    pos < editor.cursor
                } else {
                    pos > editor.cursor
                };
                if on_line && ahead && !ch.is_whitespace() {
                    style = style.fg(ACCENT).add_modifier(Modifier::BOLD);
                }
            }
            if let Some(r) = preview {
                if in_range(r, pos) {
                    style = style.fg(ACCENT).bg(PREVIEW_BG);
                }
            }
            if let Some(r) = flash {
                if in_range(r, pos) {
                    style = style.bg(FLASH_BG);
                }
            }
            spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(spans));
    }

    let block = Paragraph::new(lines).style(Style::default().bg(BASE));
    frame.render_widget(
        block,
        Rect {
            height: text_rows as u16,
            ..area
        },
    );
}

fn render_statusline(editor: &Editor, frame: &mut Frame, area: Rect) {
    let y = area.height - 1;
    let mode = editor.mode.chip();
    let file = editor.buf.path.as_deref().unwrap_or("[scratch]");
    let dirty = if editor.buf.dirty { " ●" } else { "" };
    let line = editor.buf.line_of(editor.cursor) + 1;
    let col = editor.buf.col_of(editor.cursor) + 1;

    let spec = if let Some(p) = editor.preview() {
        format!("{}  ", p.spec)
    } else if !editor.pending.is_empty() {
        format!("{}  ", editor.pending.trim_end_matches('\r'))
    } else if !editor.message.is_empty() {
        format!("{}  ", editor.message)
    } else {
        String::new()
    };
    let pos = format!("{line}:{col} ");

    // One Line, one Paragraph — two overlapping Paragraphs repaint each
    // other's cells (the mode chip went base-on-base and vanished).
    let chip = format!(" {mode} ");
    let name = format!(" {file}{dirty}");
    let used = chip.len() + name.len() + spec.len() + pos.len();
    let pad = (area.width as usize).saturating_sub(used);
    let row = Line::from(vec![
        Span::styled(
            chip,
            Style::default()
                .fg(BASE)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(name, Style::default().fg(MUTED)),
        Span::raw(" ".repeat(pad)),
        Span::styled(spec, Style::default().fg(ACCENT)),
        Span::styled(pos, Style::default().fg(MUTED)),
    ]);
    let rect = Rect {
        y,
        height: 1,
        ..area
    };
    frame.render_widget(Paragraph::new(row).style(Style::default().bg(BASE)), rect);
}

fn place_cursor(editor: &Editor, frame: &mut Frame, area: Rect) {
    let line = editor.buf.line_of(editor.cursor);
    let row = line.saturating_sub(editor.view_top) as u16;
    let col = GUTTER + editor.buf.col_of(editor.cursor) as u16;
    if row < area.height - 1 && col < area.width {
        frame.set_cursor_position((col, row));
    }
    let _ = Mode::Normal; // cursor shape per mode lands with config (0005)
}
