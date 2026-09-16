//! Real terminal lifecycle and event loop. No alternate-screen writes escape
//! this boundary; the trace writer owns a different, private file.
use crate::editor::{self, events::AppEvent, Editor, Mode};
use crossterm::event::{self, Event};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::{self, Write};
use std::time::Duration;
use strop_trace::{record_with, EventKind};
mod input;

/// Terminal restoration runs exactly once across the normal drop and
/// the panic hook (0048 §B): popping the keyboard-enhancement stack
/// twice would eat the user's outer-session flags.
static RESTORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn restore_terminal() {
    if RESTORED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableFocusChange,
        LeaveAlternateScreen
    );
}

struct TerminalLease;
impl Drop for TerminalLease {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub fn run(mut editor: Editor) -> io::Result<()> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));
    enable_raw_mode()?;
    let _lease = TerminalLease;
    let mut output = io::stdout();
    crossterm::execute!(
        output,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    // Preserve disambiguated keys and release/repeat facts. The engine decides
    // which events belong to editor grammar and which belong to a child terminal.
    let _ = crossterm::execute!(
        output,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(output))?;
    let (sender, receiver) = editor::events::channel();
    let (input_sender, input_receiver) = editor::events::channel();
    editor.connect_events(sender);
    std::thread::spawn(move || loop {
        let application_event = match event::read() {
            Ok(Event::Key(key)) => {
                AppEvent::Input(strop_core::frontend_input::Input::Key(input::key(key)))
            }
            Ok(Event::Paste(text)) => {
                AppEvent::Input(strop_core::frontend_input::Input::Paste(text))
            }
            Ok(Event::FocusLost) => AppEvent::Focus(false),
            Ok(Event::FocusGained) => AppEvent::Focus(true),
            Ok(Event::Resize(columns, rows)) => {
                record_with(
                    EventKind::Resize,
                    || serde_json::json!({"columns":columns,"rows":rows}),
                );
                AppEvent::Resize { columns, rows }
            }
            Ok(_) => continue,
            Err(error) => {
                record_with(
                    EventKind::Error,
                    || serde_json::json!({"source":"terminal_read","message":error.to_string()}),
                );
                break;
            }
        };
        // FIFO input ownership is resolved by the engine, never this reader.
        if input_sender.send(application_event).is_err() {
            return;
        }
    });
    editor.trace_state();
    let mut redraw = true;
    let mut painted_flash = false;
    let mut painted_fade = false;
    let mut animation_due = std::time::Instant::now();
    while !editor.should_quit {
        let started = std::time::Instant::now();
        let mut processed = 0;
        for _ in 0..editor::events::EVENTS_PER_TURN {
            let event = match input_receiver.try_recv() {
                Ok(event) => Some(event),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    editor.should_quit = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => receiver.try_recv().ok(),
            };
            let Some(event) = event else {
                break;
            };
            editor.recorded_action(
                editor::trace::drive::Action::Event(event),
                editor.tape.sample_tick(),
            )?;
            editor.trace_state();
            redraw = true;
            processed += 1;
            for payload in std::mem::take(&mut editor.terminal_output) {
                write!(
                    terminal.backend_mut(),
                    "\x1b]52;c;{}\x07",
                    base64_encode(payload.as_bytes())
                )?;
                terminal.backend_mut().flush()?;
            }
            if editor.should_quit || started.elapsed() >= editor::events::TURN_BUDGET {
                break;
            }
        }
        if editor.should_quit {
            break;
        }
        let flashing = editor.flash_range().is_some();
        // 0064 §2: the cursor fade shares the flash's 16 ms animation
        // budget — one cadence, never an extra thread or await.
        let fading = editor.cursor_fade_progress().is_some();
        if redraw
            || ((flashing || painted_flash || fading || painted_fade)
                && std::time::Instant::now() >= animation_due)
        {
            use crossterm::cursor::SetCursorStyle;
            let terminal_cursor = (editor.input_owner() == editor::InputOwner::Terminal)
                .then(|| {
                    editor
                        .terminal_frame(editor.current(), true)
                        .map(|frame| frame.cursor)
                })
                .flatten();
            let shape = if let Some(cursor) = terminal_cursor {
                use strop_terminal::model::CursorShape;
                match (cursor.shape, cursor.blinking) {
                    (CursorShape::Bar, true) => SetCursorStyle::BlinkingBar,
                    (CursorShape::Bar, false) => SetCursorStyle::SteadyBar,
                    (CursorShape::Underline, true) => SetCursorStyle::BlinkingUnderScore,
                    (CursorShape::Underline, false) => SetCursorStyle::SteadyUnderScore,
                    (CursorShape::Block, true) => SetCursorStyle::BlinkingBlock,
                    _ => SetCursorStyle::SteadyBlock,
                }
            } else if editor.input_normal() {
                SetCursorStyle::SteadyBlock
            } else if editor.picker_open()
                || editor.pending_sigil().is_some()
                || editor.mode == Mode::Insert
            {
                SetCursorStyle::SteadyBar
            } else if matches!(
                editor.mode,
                Mode::Visual | Mode::VisualLine | Mode::VisualBlock
            ) {
                SetCursorStyle::SteadyUnderScore
            } else {
                SetCursorStyle::SteadyBlock
            };
            crossterm::execute!(terminal.backend_mut(), shape)?;
            if std::mem::take(&mut editor.needs_repaint) {
                terminal.clear()?;
            }
            terminal.draw(|frame| crate::render::frame_capture::draw(&mut editor, frame, true))?;
            redraw = false;
            painted_flash = flashing;
            painted_fade = fading;
            animation_due = std::time::Instant::now() + Duration::from_millis(16);
        }
        if processed == 0 {
            // A retained unpark token closes the queue-empty/park race. The
            // timeout also notices terminal-reader shutdown and flash expiry.
            std::thread::park_timeout(Duration::from_millis(16));
        }
    }
    editor.recorded_action(
        editor::trace::drive::Action::Finish,
        editor.tape.sample_tick(),
    )?;
    while editor.async_pending() {
        let event = receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(io::Error::other)?;
        editor.recorded_action(
            editor::trace::drive::Action::Event(event),
            editor.tape.sample_tick(),
        )?;
    }
    editor.tape.finish()?;
    if let Some(error) = editor.take_shutdown_error() {
        return Err(io::Error::other(error));
    }
    Ok(())
}

/// Minimal base64 for OSC52 (no dep for a twenty-line function).
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
