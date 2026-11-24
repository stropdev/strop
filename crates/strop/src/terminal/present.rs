//! One frame onto the physical terminal as one atomic update.
//!
//! A frame is bracketed in a synchronized update (DEC mode 2026), the
//! hardware cursor stays hidden until the frame places it, and the bytes
//! leave through a frame-sized buffer in one write. Without this, a
//! scroll repaints nearly every row through ~1 KiB stdout chunks with the
//! cursor visible, and the terminal shows it parked at the end of a
//! half-written row between chunks. Terminals without mode 2026 ignore
//! the bracket; the hidden cursor and the single write still hold.
use crossterm::cursor::{Hide, SetCursorStyle};
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};
use std::io::{self, BufWriter, Write};
use std::mem::{discriminant, Discriminant};

/// Large enough for a full repaint of a big terminal with true colour
/// on every cell run; a frame that still exceeds it splits, but the
/// cursor stays hidden across the split.
const FRAME_BYTES: usize = 1 << 20;

/// The physical terminal's output: frame-sized buffering over `out`.
pub(super) fn output<W: Write>(out: W) -> BufWriter<W> {
    BufWriter::with_capacity(FRAME_BYTES, out)
}

/// Paints one frame. `shape` is emitted only when it differs from the
/// last one sent (`sent`); `repaint` clears the screen first.
pub(super) fn present<W: Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
    shape: SetCursorStyle,
    sent: &mut Option<Discriminant<SetCursorStyle>>,
    repaint: bool,
    paint: impl FnOnce(&mut Frame),
) -> io::Result<()> {
    crossterm::queue!(terminal.backend_mut(), BeginSynchronizedUpdate, Hide)?;
    if *sent != Some(discriminant(&shape)) {
        crossterm::queue!(terminal.backend_mut(), shape)?;
        *sent = Some(discriminant(&shape));
    }
    if repaint {
        terminal.clear()?;
    }
    // ratatui ends the frame with the cursor shown at its position (or
    // left hidden) and flushes
    terminal.draw(paint)?;
    crossterm::queue!(terminal.backend_mut(), EndSynchronizedUpdate)?;
    terminal.backend_mut().flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records each write call the terminal would receive.
    #[derive(Clone, Default)]
    struct Writes(Rc<RefCell<Vec<Vec<u8>>>>);
    impl Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn rows(frame: &mut Frame, label: &str) {
        let area = frame.area();
        for y in 0..area.height {
            let text = format!("{label} row {y} {}", "x".repeat(usize::from(area.width)));
            frame.render_widget(Line::raw(text), Rect::new(0, y, area.width, 1));
        }
        frame.set_cursor_position((3, 2));
    }

    fn terminal(writes: &Writes) -> Terminal<CrosstermBackend<BufWriter<Writes>>> {
        let backend = CrosstermBackend::new(output(writes.clone()));
        Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 200, 60)),
            },
        )
        .unwrap()
    }

    #[test]
    fn a_full_repaint_reaches_the_terminal_with_the_cursor_hidden_until_placed() {
        let writes = Writes::default();
        let mut terminal = terminal(&writes);
        let mut sent = None;
        present(
            &mut terminal,
            SetCursorStyle::SteadyBlock,
            &mut sent,
            false,
            |f| rows(f, "first"),
        )
        .unwrap();
        writes.0.borrow_mut().clear();
        // a scroll: every row changes
        present(
            &mut terminal,
            SetCursorStyle::SteadyBlock,
            &mut sent,
            false,
            |f| rows(f, "second"),
        )
        .unwrap();
        let chunks = writes.0.borrow();
        let bytes = chunks.concat();
        let cells = find(&bytes, b"second row 0").unwrap();
        let last_cells = find(&bytes, b"second row 59").unwrap();
        let begin = find(&bytes, b"\x1b[?2026h").unwrap();
        let hide = find(&bytes, b"\x1b[?25l").unwrap();
        let show = find(&bytes, b"\x1b[?25h").unwrap();
        assert!(
            begin < cells && hide < cells,
            "cursor hidden before any cell"
        );
        assert!(show > last_cells, "cursor shown only after every cell");
        assert!(
            bytes.ends_with(b"\x1b[?2026l"),
            "the update closes the frame"
        );
        let body = chunks
            .iter()
            .filter(|chunk| find(chunk, b"second row").is_some())
            .count();
        assert_eq!(body, 1, "the frame body leaves in one write");
        assert!(
            find(&bytes, b" q").is_none(),
            "an unchanged cursor shape is not re-sent"
        );
    }

    #[test]
    fn a_changed_cursor_shape_is_sent_inside_the_update() {
        let writes = Writes::default();
        let mut terminal = terminal(&writes);
        let mut sent = Some(discriminant(&SetCursorStyle::SteadyBlock));
        present(
            &mut terminal,
            SetCursorStyle::SteadyBar,
            &mut sent,
            false,
            |f| rows(f, "insert"),
        )
        .unwrap();
        let bytes = writes.0.borrow().concat();
        let bar = find(&bytes, b"\x1b[6 q").unwrap();
        assert!(find(&bytes, b"\x1b[?2026h").unwrap() < bar);
        assert_eq!(sent, Some(discriminant(&SetCursorStyle::SteadyBar)));
    }
}
