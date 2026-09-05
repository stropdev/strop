//! Rendering: the render tree's root. Panes and diff decoration live
//! in `buffer`/`diff` (0010 §3); this module owns the palette, the
//! statusline, the welcome card, and the cursor.
//! Overlay precedence (0001 §5.8 subset): search/incsearch < operator
//! preview < cursor. One accent color (0001 §4).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use strop_core::Range;

use crate::editor::{Editor, Mode};

mod blame_card;
mod buffer;
mod cmd_card;
mod diff;
mod help;
mod hover_card;
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

/// Diagnostic severity → color (LSP 1=error … 4=hint; one source for
/// the gutter sign and the cursor-line end-of-line note).
pub(crate) fn severity_color(sev: u8) -> Color {
    match sev {
        1 => Color::Rgb(0xe8, 0x67, 0x7a), // error red
        2 => ACCENT,                       // warning amber
        3 => Color::Rgb(0x7f, 0xb4, 0xca), // info blue
        _ => MUTED,                        // hint
    }
}

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
    editor.refresh_hunks();

    let pane_area = buffer::render_panes(editor, frame, area);
    render_statusline(editor, frame, area);
    cmd_card::render_cmd_card(editor, frame);
    if !cmd_card_active(editor) {
        place_cursor(editor, frame, pane_area);
    }
    render_welcome(editor, frame);
    picker_card::render_picker(editor, frame);
    blame_card::render_blame_card(editor, frame);
    hover_card::render_hover_card(editor, frame);
    which_key::render_which_key(editor, frame);
}

/// Mode chip colors (0001 §4: mode = accent color change, not bars).
pub(crate) fn mode_color(mode: Mode) -> Color {
    match mode {
        Mode::Normal => ACCENT,
        Mode::Insert => Color::Rgb(0xa9, 0xc4, 0x7c), // green
        Mode::Visual | Mode::VisualLine => Color::Rgb(0xc5, 0x8a, 0xe8), // violet
    }
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

fn render_statusline(editor: &Editor, frame: &mut Frame, area: Rect) {
    let y = area.height - 1;
    let mode = editor.mode.chip();
    let file = editor
        .buf()
        .path
        .as_deref()
        .or(editor.buf().name.as_deref())
        .unwrap_or("[scratch]");
    let dirty = if editor.buf().dirty { " ●" } else { "" };
    let line = editor.buf().line_of(editor.head()) + 1;
    let col = editor.buf().col_of(editor.head()) + 1;
    let branch = editor.git.as_ref().and_then(|g| g.head_branch());
    let hunks_dirty = !editor.hunks.is_empty();
    let readonly = editor.buf().readonly;
    let (errors, warnings) = editor.diag_counts(editor.current());
    let cursors = editor.sels().count();
    let total = editor.buf().len_lines().max(1);
    let pct = if total <= 1 { 100 } else { line * 100 / total };

    let spec = if let Some((_, spec)) = editor.preview() {
        format!("{spec}  ")
    } else if !editor.pending.is_empty() && !cmd_card_active(editor) {
        format!("{}  ", editor.pending.trim_end_matches('\r'))
    } else if !editor.walker.prefix.is_empty() || !editor.walker.state.empty() {
        // structural input mid-flight (3d…, g…, space…): the modeline
        // shows the walker's typed state
        format!("{}  ", editor.walker.display())
    } else if !editor.message.is_empty() {
        format!("{}  ", editor.message)
    } else {
        String::new()
    };

    // left: mode chip · branch (worktree-dirty marks it) · file · flags
    let mut left: Vec<Span> = vec![
        Span::styled("▌", Style::default().fg(mode_color(editor.mode))),
        Span::styled(
            format!(" {mode} "),
            Style::default()
                .fg(BASE)
                .bg(mode_color(editor.mode))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(b) = branch {
        let dirty_mark = if hunks_dirty { "*" } else { "" };
        left.push(Span::styled(
            format!(" {b}{dirty_mark}"),
            Style::default().fg(ACCENT),
        ));
    }
    left.push(Span::styled(
        format!(" {file}{dirty}"),
        Style::default().fg(MUTED),
    ));
    if readonly {
        left.push(Span::styled(
            " [RO]",
            Style::default().fg(Color::Rgb(0xe0, 0xaf, 0x68)),
        ));
    }
    if cursors > 1 {
        left.push(Span::styled(
            format!(" {cursors}×"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    }

    // right: diag chips · position · percent
    let mut right: Vec<Span> = Vec::new();
    if errors > 0 {
        right.push(Span::styled(
            format!(" ●{errors}"),
            Style::default().fg(severity_color(1)),
        ));
    }
    if warnings > 0 {
        right.push(Span::styled(
            format!(" ●{warnings}"),
            Style::default().fg(severity_color(2)),
        ));
    }
    right.push(Span::styled(
        format!(" {line}:{col}"),
        Style::default().fg(MUTED),
    ));
    right.push(Span::styled(
        format!(" {pct}% "),
        Style::default().fg(MUTED),
    ));

    let used: usize = left
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>()
        + spec.chars().count()
        + right
            .iter()
            .map(|s| s.content.chars().count())
            .sum::<usize>()
        + 1;
    let pad = (area.width as usize).saturating_sub(used);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(spec, Style::default().fg(ACCENT)));
    spans.extend(right);
    let row = Line::from(spans);
    let rect = Rect {
        y,
        height: 1,
        ..area
    };
    frame.render_widget(Paragraph::new(row).style(Style::default().bg(BASE)), rect);
}

fn place_cursor(editor: &Editor, frame: &mut Frame, area: Rect) {
    let line = editor.buf().line_of(editor.head());
    let row = line.saturating_sub(editor.view_top()) as u16;
    // composed once: sidebar + blame column + the surface's number
    // gutter (0011) — diff-wide gutters used to drift the caret
    let col = diff::left_inset(editor, editor.current()) as u16
        + editor.buf().col_of(editor.head()) as u16;
    if row < area.height - 1 && col < area.width {
        frame.set_cursor_position((col, row));
    }
}

/// First-launch card: brand + the three keys that matter. Only on an
/// empty scratch buffer — once you're editing, it never intrudes.
fn render_welcome(editor: &Editor, frame: &mut Frame) {
    if editor.buf().path.is_some() || editor.buf().len_bytes() > 0 || editor.picker_open() {
        return;
    }
    use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
    let area = frame.area();
    let (w, h) = (58u16, 9u16);
    if area.width < w + 4 || area.height < h + 4 {
        return;
    }
    let card = Rect {
        x: (area.width - w) / 2,
        y: (area.height - h) / 3,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .style(Style::default().bg(BASE));
    let inner = block.inner(card);
    frame.render_widget(block, card);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " strop",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " see the cut before you make it.",
            Style::default().fg(ACCENT),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " space ",
                Style::default()
                    .fg(ACCENT)
                    .bg(SELECT_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" everything · ", Style::default().fg(MUTED)),
            Span::styled(
                " ? ",
                Style::default()
                    .fg(ACCENT)
                    .bg(SELECT_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" keybindings · ", Style::default().fg(MUTED)),
            Span::styled(
                " :w ",
                Style::default()
                    .fg(ACCENT)
                    .bg(SELECT_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" save", Style::default().fg(MUTED)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  git signs paint the gutter · ci[ previews the cut",
            Style::default().fg(MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// True when the floating command/search card owns the caret.
pub(crate) fn cmd_card_active(editor: &Editor) -> bool {
    !editor.picker_open()
        && (editor.pending.starts_with(':') || editor.pending.contains('/'))
        && !editor.pending.is_empty()
}
