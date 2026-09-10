//! Headless driver (0006 tier 2 prototype): scripted keys in, cell-grid
//! frames + state JSON out. Deterministic — no PTY, no timing.
//!
//! R11: every external input and delivery enters through the tape's
//! recording wrapper (`recorded_action`), so a `--log-content` capture of
//! a headless run is a complete forensic recording and `--replay`
//! reproduces it without the script, the clock or the host.

pub(crate) mod directives;
mod driver;
pub use driver::run_script;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::editor::Editor;

pub fn render_frame(
    editor: &mut Editor,
    cols: u16,
    rows: u16,
    record_action: bool,
) -> std::io::Result<Terminal<TestBackend>> {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows))?;
    terminal.draw(|frame| crate::render::frame_capture::draw(editor, frame, record_action))?;
    editor.tape.healthy()?;
    Ok(terminal)
}

/// The injected frame renderer (0046): cell-grid production is the
/// binary's; the engine's replay consumes recorded frames through this.
pub fn frame_draw(
    editor: &mut Editor,
    cols: u16,
    rows: u16,
    record_action: bool,
) -> std::io::Result<()> {
    render_frame(editor, cols, rows, record_action)?;
    Ok(())
}

pub fn frame_string(editor: &mut Editor, cols: u16, rows: u16) -> std::io::Result<String> {
    let terminal = render_frame(editor, cols, rows, true)?;
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..rows {
        for symbol in row_symbols(buf, y) {
            out.push_str(symbol);
        }
        out.push('\n');
    }
    Ok(out)
}

/// TestBackend retains covered cells beneath wide glyphs. Those cells are
/// not visible terminal text and must not leak into textual frame dumps.
pub(super) fn row_symbols(
    buffer: &ratatui::buffer::Buffer,
    row: u16,
) -> impl Iterator<Item = &str> {
    let mut column = buffer.area.x;
    let right = buffer.area.right();
    std::iter::from_fn(move || {
        if column >= right {
            return None;
        }
        let symbol = buffer[(column, row)].symbol();
        let covered = ratatui::text::Span::raw(symbol).width().max(1);
        column += covered.min(usize::from(right - column)) as u16;
        Some(symbol)
    })
}

#[cfg(test)]
mod diff_surface_tests {
    use strop_git::{DiffLine, Hunk, LineOrigin};

    fn hunk() -> Hunk {
        Hunk::build(
            1,
            2,
            1,
            3,
            vec![
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Context,
                    old_lineno: Some(1),
                    new_lineno: Some(1),
                    text: "fn a() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Deletion,
                    old_lineno: Some(2),
                    new_lineno: None,
                    text: "fn old() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Addition,
                    old_lineno: None,
                    new_lineno: Some(2),
                    text: "fn new() {}".into(),
                },
                DiffLine {
                    has_newline: true,
                    origin: LineOrigin::Addition,
                    old_lineno: None,
                    new_lineno: Some(3),
                    text: "fn extra() {}".into(),
                },
            ],
        )
    }

    /// Golden shape of a diff surface frame (0010 §4): stats row, hunk
    /// header, both sides' numbers, no raw-patch noise, no prefixes.
    #[test]
    fn diff_surface_frame_shape() {
        let mut e =
            crate::editor::Editor::new_in(strop_core::Buffer::from_text("x\n"), "/recorded".into());
        e.fixture_git_context();
        e.open_delta(
            "delta",
            crate::editor::PreparedDiff::new("f.rs".into(), vec![hunk()]),
            None,
            None,
        );
        let frame = crate::headless::frame_string(&mut e, 80, 20).unwrap();
        assert!(frame.contains(" f.rs +2 -1"), "stats row: {frame}");
        assert!(frame.contains(" @@ -1,2 +1,3 @@"), "hunk header: {frame}");
        // both numbers on context, one side blank on add/del rows
        assert!(
            frame.contains("  1   1 fn a() {}"),
            "context gutter: {frame}"
        );
        assert!(
            frame.contains("▎      2 fn new() {}"),
            "addition gutter: {frame}"
        );
        assert!(
            frame.contains("▎  2     fn old() {}"),
            "deletion gutter: {frame}"
        );
        assert!(
            !frame.contains("+fn new"),
            "no + prefix in content: {frame}"
        );
        assert!(
            !frame.contains("diff --git"),
            "no raw patch header: {frame}"
        );
    }

    /// `q` hands the cursor back to the buffer the surface opened from.
    #[test]
    fn surface_close_restores_cursor() {
        let mut e = crate::editor::Editor::new_in(
            strop_core::Buffer::from_text("a\nb\nc\n"),
            "/recorded".into(),
        );
        e.fixture_git_context();
        e.feed_text("jj"); // line 3
        e.open_delta(
            "hunk",
            crate::editor::PreparedDiff::new("hunk".into(), vec![hunk()]),
            None,
            None,
        );
        assert_eq!(
            e.buf().line_of(e.head()),
            0,
            "surface starts at its own top"
        );
        e.feed_text("q");
        assert_eq!(e.current(), e.first_doc());
        assert_eq!(e.buf().line_of(e.head()), 2, "cursor returned to line 3");
    }
}

#[cfg(test)]
mod quit_tests {
    #[test]
    fn quit_then_frame_does_not_panic() {
        let mut e =
            crate::editor::Editor::new_in(strop_core::Buffer::from_text("x\n"), "/recorded".into());
        let mut out = Vec::new();
        crate::headless::run_script(&mut e, "keys :q!<cr>\nframe\n", 60, 10, &mut out, None)
            .unwrap();
        assert!(e.should_quit);
    }
}
