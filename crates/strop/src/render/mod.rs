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

use crate::editor::{Editor, Mode};

mod picker_card;
mod which_key;

// strop default palette (plan 0004 site, --accent amber)
pub const BASE: Color = Color::Rgb(0x16, 0x16, 0x1e);
pub const TEXT: Color = Color::Rgb(0xe8, 0xe4, 0xda);
pub const MUTED: Color = Color::Rgb(0x6b, 0x6f, 0x7e);
pub const ACCENT: Color = Color::Rgb(0xf0, 0xa3, 0x5e);
pub const PREVIEW_BG: Color = Color::Rgb(0x4a, 0x33, 0x1c); // accent, dimmed
pub const FLASH_BG: Color = Color::Rgb(0x6b, 0x47, 0x22); // accent, stronger
pub const SELECT_BG: Color = Color::Rgb(0x2a, 0x2c, 0x3a);

const GUTTER: u16 = 5; // 4-digit numbers + one empty column (0001 §4)

/// Syntax class → color (strop palette; theme engine swaps these later).
pub(crate) fn class_color(class: strop_syntax::Class) -> Color {
    use strop_syntax::Class as C;
    match class {
        C::Keyword => Color::Rgb(0xc5, 0x8a, 0xe8),
        C::Function => Color::Rgb(0x7f, 0xb4, 0xca),
        C::Type => Color::Rgb(0x94, 0xd2, 0xbd),
        C::String => Color::Rgb(0xa9, 0xc4, 0x7c),
        C::Comment => MUTED,
        C::Number => Color::Rgb(0xe8, 0x97, 0x7a),
        C::Operator => Color::Rgb(0x9a, 0xa0, 0xae),
        C::Punctuation => Color::Rgb(0x56, 0x5b, 0x6e),
        C::Constant => ACCENT,
        C::Attribute => Color::Rgb(0xd0, 0xa4, 0x5e),
        C::Variable => TEXT,
    }
}

pub fn render(editor: &mut Editor, frame: &mut Frame) {
    let area = frame.area();
    let text_rows = area.height.saturating_sub(1) as usize;
    editor.scroll_to_cursor(text_rows);

    render_text(editor, frame, area, text_rows);
    render_statusline(editor, frame, area);
    place_cursor(editor, frame, area);
    picker_card::render_picker(editor, frame);
    which_key::render_which_key(editor, frame);
}

/// Pull a color toward the base for the picker's dimmed backdrop.
pub(crate) fn dim_color(c: Color) -> Color {
    fn mix(c: (u8, u8, u8), base: (u8, u8, u8), t: u8) -> Color {
        let m =
            |a: u8, b: u8| (a as u16 * (100 - t) as u16 / 100 + b as u16 * t as u16 / 100) as u8;
        Color::Rgb(m(c.0, base.0), m(c.1, base.1), m(c.2, base.2))
    }
    const BASE_RGB: (u8, u8, u8) = (0x16, 0x16, 0x1e);
    match c {
        Color::Rgb(r, g, b) => mix((r, g, b), BASE_RGB, 55),
        other => other,
    }
}

fn in_range(r: Range, pos: usize) -> bool {
    pos >= r.start && pos < r.end
}

fn render_text(editor: &mut Editor, frame: &mut Frame, area: Rect, text_rows: usize) {
    let preview = editor.preview().map(|r| r.range);
    let flash = editor.flash_range();
    let selection = editor.visual_range();
    let search_hits: Vec<usize> = editor
        .search_pattern()
        .map(|p| grammar::search_all(editor.buf(), p))
        .unwrap_or_default();
    let find = editor.find_candidates();

    let cur_line = editor.buf().line_of(editor.cursor);
    let mut lines: Vec<Line> = Vec::with_capacity(text_rows);

    // tree-sitter spans for the visible window (base layer, 0001 §5.8)
    let first_byte = editor.buf().line_start(editor.view_top);
    let last_line = (editor.view_top + text_rows).min(editor.buf().len_lines());
    let last_byte = editor.buf().line_end(last_line.saturating_sub(1));
    let syn_spans: Vec<strop_syntax::Span> = match &mut editor.highlighter {
        Some(h) => h.highlight(&editor.buffers[editor.current].rope, first_byte, last_byte),
        None => Vec::new(),
    };

    for row in 0..text_rows {
        let line_idx = editor.view_top + row;
        if line_idx >= editor.buf().len_lines() {
            lines.push(Line::from(Span::styled("~", Style::default().fg(MUTED))));
            continue;
        }
        let start = editor.buf().line_start(line_idx);
        let text = editor.buf().line_text(line_idx);

        // gutter: muted numbers, current line in accent (0001 §4)
        let num_style = if line_idx == cur_line {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(MUTED)
        };
        let mut spans = vec![Span::styled(format!("{:>4} ", line_idx + 1), num_style)];

        let mut syn_idx = syn_spans.partition_point(|s| s.end <= start);
        for (i, ch) in text.chars().enumerate() {
            let pos = start + i; // prototype is ASCII-honest (0001 §5.9 later)
            while syn_idx < syn_spans.len() && syn_spans[syn_idx].end <= pos {
                syn_idx += 1;
            }
            let mut style = Style::default().fg(TEXT);
            if syn_idx < syn_spans.len() && syn_spans[syn_idx].start <= pos {
                let class = syn_spans[syn_idx].class;
                style = style.fg(class_color(class));
                if class == strop_syntax::Class::Comment {
                    style = style.add_modifier(Modifier::ITALIC);
                }
            }
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
                let on_line = editor.buf().line_of(pos) == cur_line;
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
    let file = editor.buf().path.as_deref().unwrap_or("[scratch]");
    let dirty = if editor.buf().dirty { " ●" } else { "" };
    let line = editor.buf().line_of(editor.cursor) + 1;
    let col = editor.buf().col_of(editor.cursor) + 1;

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
    let line = editor.buf().line_of(editor.cursor);
    let row = line.saturating_sub(editor.view_top) as u16;
    let col = GUTTER + editor.buf().col_of(editor.cursor) as u16;
    if row < area.height - 1 && col < area.width {
        frame.set_cursor_position((col, row));
    }
    let _ = Mode::Normal; // cursor shape per mode lands with config (0005)
}
