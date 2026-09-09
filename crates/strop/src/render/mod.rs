//! Rendering: the render tree's root. Panes and diff decoration live
//! in `buffer`/`diff` (0010 §3); `statusline` owns the modeline.
//! This module owns the palette, welcome card, and cursor.
//! Overlay precedence (0001 §5.8 subset): search/incsearch < operator
//! preview < cursor. One accent color (0001 §4).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::editor::{Editor, Mode};

mod blame_card;
mod buffer;
mod cmd_card;
pub(crate) mod diff;
mod help;
mod hover_card;
mod picker_card;
mod statusline;
#[cfg(test)]
mod terminal_tests;
mod text;
mod which_key;

// strop default palette (plan 0004 site, --accent amber)
pub const BASE: Color = Color::Rgb(0x16, 0x16, 0x1e);
pub const TEXT: Color = Color::Rgb(0xe8, 0xe4, 0xda);
pub const MUTED: Color = Color::Rgb(0x6b, 0x6f, 0x7e);
pub const ACCENT: Color = Color::Rgb(0xf0, 0xa3, 0x5e);
pub const PREVIEW_BG: Color = Color::Rgb(0x4a, 0x33, 0x1c); // accent, dimmed
pub const FLASH_BG: Color = Color::Rgb(0x6b, 0x47, 0x22); // accent, stronger
pub const SELECT_BG: Color = Color::Rgb(0x2a, 0x2c, 0x3a);

/// Diagnostic severity → color (LSP typed severity; one source for
/// the gutter sign and the cursor-line end-of-line note).
pub(crate) fn severity_color(sev: strop_lsp::Severity) -> Color {
    use strop_lsp::Severity;
    match sev {
        Severity::Error => Color::Rgb(0xe8, 0x67, 0x7a), // error red
        Severity::Warning => ACCENT,                     // warning amber
        Severity::Information => Color::Rgb(0x7f, 0xb4, 0xca), // info blue
        Severity::Hint => MUTED,                         // hint
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
        C::Heading | C::List => ACCENT,
        C::Link | C::Tag => Color::Rgb(0x7f, 0xb4, 0xca),
        C::Code => Color::Rgb(0xa9, 0xc4, 0x7c),
        C::Quote => MUTED,
    }
}

/// One style projection for panes and picker previews; overlays compose later.
pub(crate) fn syntax_style(span: &strop_syntax::Span) -> Style {
    let mut style = Style::default().fg(class_color(span.class));
    if span.emphasis.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if span.emphasis.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if span.emphasis.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if span.emphasis.strikethrough {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

pub fn render(editor: &mut Editor, frame: &mut Frame) {
    let area = frame.area();
    // pane geometry (heights feed the vertical viewport, widths the
    // horizontal origin) is decided per pane inside render_panes —
    // the full-area numbers were wrong in splits (0031 R6)
    editor.refresh_hunks();

    let pane_area = buffer::render_panes(editor, frame, area);
    statusline::render(editor, frame, area);
    cmd_card::render_cmd_card(editor, frame);
    if !cmd_card_active(editor) {
        place_cursor(editor, frame, pane_area);
    }
    render_welcome(editor, frame);
    picker_card::render_picker(editor, frame);
    blame_card::render_blame_card(editor, frame);
    hover_card::render_hover_card(editor, frame);
    which_key::render_which_key(editor, frame);
    // Every widget can display external text (paths, LSP messages, shell output).
    // Enforce the printable-cell invariant at the final emission boundary too.
    for cell in &mut frame.buffer_mut().content {
        if cell.symbol().chars().any(char::is_control) {
            cell.set_symbol("\u{fffd}");
        }
    }
}

/// Mode chip colors (0001 §4: mode = accent color change, not bars).
pub(crate) fn mode_color(mode: Mode) -> Color {
    match mode {
        Mode::Normal => ACCENT,
        Mode::Insert => Color::Rgb(0xa9, 0xc4, 0x7c), // green
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => Color::Rgb(0xc5, 0x8a, 0xe8), // violet
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

fn place_cursor(editor: &Editor, frame: &mut Frame, area: Rect) {
    // one projection shared with every painted caret: vertical top +
    // horizontal origin + fixed inset, checked, narrowed once
    if let Some(at) = buffer::caret_position(
        editor,
        area,
        editor.current(),
        editor.head(),
        editor.view_top(),
        editor.view().hscroll,
    ) {
        crate::editor::trace::frame::place_cursor(frame, at);
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

/// True when the floating command/search/pipe card owns the caret
/// (0031 R7: every sigil prompt — `:`, `/`, `?`, `|` — takes it).
pub(crate) fn cmd_card_active(editor: &Editor) -> bool {
    !editor.picker_open() && editor.pending_sigil().is_some()
}
