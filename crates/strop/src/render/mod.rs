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
mod clip;
mod cmd_card;
mod completion;
pub(crate) mod diff;
pub(crate) mod frame_capture;
#[cfg(test)]
mod frame_tests;
mod help;
mod hover_card;
#[path = "picker/mod.rs"]
mod picker;
mod statusline;
mod terminal;
#[cfg(test)]
mod terminal_tests;
mod which_key;

// strop palette (plan 0004 site, --accent amber): every value derives
// from the strop-core theme seed's CURRENT palette (0065 D3, `:theme`)
// — never restated here. Accessors, not consts: a theme switch takes
// effect on the next frame.
const fn seed(value: strop_core::theme::Rgb) -> Color {
    Color::Rgb(value.r, value.g, value.b)
}
pub fn base() -> Color {
    seed(strop_core::theme::current().base)
}
pub fn text() -> Color {
    seed(strop_core::theme::current().text)
}
pub fn muted() -> Color {
    seed(strop_core::theme::current().muted)
}
pub fn accent() -> Color {
    seed(strop_core::theme::current().accent)
}
pub fn preview_bg() -> Color {
    seed(strop_core::theme::current().preview_bg)
}
pub fn flash_bg() -> Color {
    seed(strop_core::theme::current().flash_bg)
}
pub fn select_bg() -> Color {
    seed(strop_core::theme::current().select_bg)
}
/// Matching-delimiter overlay (0051 §7 R09): quiet — a slate wash one
/// step above the selection, never the accent's urgency.
pub fn pair_bg() -> Color {
    seed(strop_core::theme::current().pair_bg)
}
/// Useful secondary context (0050 §4): between text and muted.
pub fn secondary() -> Color {
    seed(strop_core::theme::current().secondary)
}
/// Terminal-input chip (0065 S1): the child's type teal — glanceably not
/// Normal's amber, Insert's green or Visual's violet, because the keys go
/// to the child, not the grammar.
pub fn terminal_chip() -> Color {
    seed(strop_core::theme::current().class_type)
}

/// Diagnostic severity → color (LSP typed severity; one source for
/// the gutter sign and the cursor-line end-of-line note).
pub(crate) fn severity_color(sev: strop_lsp::Severity) -> Color {
    use strop_lsp::Severity;
    let theme = strop_core::theme::current();
    match sev {
        Severity::Error => seed(theme.diag_error),
        Severity::Warning => accent(), // warning amber
        Severity::Information => seed(theme.diag_info),
        Severity::Hint => muted(), // hint
    }
}

/// Syntax class → color (one projection; `:theme` swaps the palette).
pub(crate) fn class_color(class: strop_syntax::Class) -> Color {
    use strop_syntax::Class as C;
    let theme = strop_core::theme::current();
    match class {
        C::Keyword => seed(theme.class_keyword),
        C::Function => seed(theme.class_function),
        C::Type => seed(theme.class_type),
        C::String => seed(theme.class_string),
        C::Comment => muted(),
        C::Number => seed(theme.class_number),
        C::Operator => seed(theme.class_operator),
        C::Punctuation => seed(theme.class_punctuation),
        C::Constant => accent(),
        C::Attribute => seed(theme.class_attribute),
        C::Variable => text(),
        C::Heading | C::List => accent(),
        C::Link | C::Tag => seed(theme.diag_info),
        C::Code => seed(theme.class_string),
        C::Quote => muted(),
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

mod field;

/// Toolkit rect → engine cell geometry (AR01: geometry enters the engine
/// only through this conversion; cells, never pixels).
pub(crate) fn to_cells(rect: Rect) -> strop_engine::editor::prepare::CellRect {
    strop_engine::editor::prepare::CellRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// Engine cell geometry → toolkit rect.
pub(crate) fn from_cells(cells: strop_engine::editor::prepare::CellRect) -> Rect {
    Rect {
        x: cells.x,
        y: cells.y,
        width: cells.width,
        height: cells.height,
    }
}

/// AR01: paint is a readonly presentation query. The admitted engine
/// update (`Editor::prepare_view`, engine-side) owns hunk refresh,
/// visible-analysis/preview admission and viewport adjustment; the frame
/// boundary (`frame_capture::draw`, or the test `paint` helper) runs it
/// before this function. Paint may update its own frontend caches (the
/// ratatui cell buffer), never engine authority.
pub fn render(editor: &Editor, frame: &mut Frame) {
    let area = frame.area();
    if editor.panes().is_empty() {
        return;
    }
    // A frontend that skipped preparation paints a stale-revision view:
    // loud in tests, and never an engine mutation in any build.
    debug_assert_eq!(
        editor.view_epoch().geometry,
        strop_engine::editor::prepare::ViewGeometry {
            columns: area.width,
            rows: area.height,
        },
        "paint without prepare_view for this geometry"
    );

    // pane geometry (heights feed the vertical viewport, widths the
    // horizontal origin) is decided per pane inside render_panes —
    // the full-area numbers were wrong in splits (0031 R6)
    let pane_area = buffer::render_panes(editor, frame, area);
    statusline::render(editor, frame, area);
    cmd_card::render_cmd_card(editor, frame);
    let caret = if !cmd_card_active(editor) && !editor.terminal_input_active() {
        place_cursor(editor, frame, pane_area)
    } else {
        None
    };
    render_welcome(editor, frame);
    picker::render_picker(editor, frame);
    blame_card::render_blame_card(editor, frame);
    hover_card::render_hover_card(editor, frame);
    completion::render(editor, frame, pane_area, caret);
    which_key::render_which_key(editor, frame);
    // Every widget can display external text (paths, LSP messages, shell output).
    // Enforce the printable-cell invariant at the final emission boundary too.
    for cell in &mut frame.buffer_mut().content {
        if cell.symbol().chars().any(char::is_control) {
            cell.set_symbol("\u{fffd}");
        }
    }
}

/// Test-frontend frame boundary: the admitted preparation for this
/// frame's geometry, then the readonly paint — the same two phases the
/// real frontends run through `frame_capture::draw`.
#[cfg(test)]
pub(crate) fn paint(editor: &mut Editor, frame: &mut Frame) {
    let area = frame.area();
    editor.prepare_view(strop_engine::editor::prepare::ViewGeometry {
        columns: area.width,
        rows: area.height,
    });
    render(editor, frame);
}

/// Mode chip colors (0001 §4: mode = accent color change, not bars).
pub(crate) fn mode_color(mode: Mode) -> Color {
    match mode {
        Mode::Normal => accent(),
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

fn place_cursor(editor: &Editor, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
    // one projection shared with every painted caret: vertical top +
    // horizontal origin + fixed inset, checked, narrowed once
    let at = buffer::caret_position(
        editor,
        area,
        editor.current(),
        editor.head(),
        editor.view_top(),
        editor.view().hscroll,
    )?;
    // 0064 §2: while the fade-in runs, the Normal-mode block is
    // software-painted toward its final appearance and the native
    // cursor stays hidden; when the window closes the final frame is
    // exactly the unfaded cursor below. Presentation only.
    if matches!(editor.mode(), Mode::Normal)
        && !editor.picker_open()
        && editor.pending_sigil().is_none()
    {
        if let Some(progress) = editor.cursor_fade_progress() {
            let cell = &mut frame.buffer_mut()[at];
            let fg = cell.fg;
            cell.set_bg(fade_mix(base(), text(), progress));
            cell.set_fg(fade_mix(fg, base(), progress));
            return Some(at);
        }
    }
    crate::render::frame_capture::place_cursor(frame, at);
    Some(at)
}

/// Linear RGB ramp for the cursor fade (0064 §2). Terminal-palette
/// (non-RGB) colors have no interpoland and switch over halfway.
fn fade_mix(from: Color, to: Color, pct: u8) -> Color {
    let (Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb)) = (from, to) else {
        return if pct >= 50 { to } else { from };
    };
    let mix =
        |a: u8, b: u8| (i32::from(a) + (i32::from(b) - i32::from(a)) * i32::from(pct) / 100) as u8;
    Color::Rgb(mix(fr, tr), mix(fg, tg), mix(fb, tb))
}

/// First-launch card: brand + the three keys that matter. Only on an
/// empty scratch buffer — once you're editing, it never intrudes.
fn render_welcome(editor: &Editor, frame: &mut Frame) {
    if !matches!(editor.cur().source, crate::editor::DocumentSource::Scratch)
        || editor.buf().path.is_some()
        || editor.buf().len_bytes() > 0
        || editor.picker_open()
    {
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
        .border_style(Style::default().fg(muted()))
        .style(Style::default().bg(base()));
    let inner = block.inner(card);
    frame.render_widget(block, card);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " strop",
            Style::default().fg(text()).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " see the cut before you make it.",
            Style::default().fg(accent()),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " space ",
                Style::default()
                    .fg(accent())
                    .bg(select_bg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" everything · ", Style::default().fg(muted())),
            Span::styled(
                " ? ",
                Style::default()
                    .fg(accent())
                    .bg(select_bg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" keybindings · ", Style::default().fg(muted())),
            Span::styled(
                " :w ",
                Style::default()
                    .fg(accent())
                    .bg(select_bg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" save", Style::default().fg(muted())),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  git signs paint the gutter · ci[ previews the cut",
            Style::default().fg(muted()),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// True when the floating command/search/pipe card owns the caret
/// (0031 R7: every sigil prompt — `:`, `/`, `?`, `|` — takes it).
pub(crate) fn cmd_card_active(editor: &Editor) -> bool {
    !editor.picker_open() && editor.pending_sigil().is_some()
}
