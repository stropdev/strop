//! Real terminal lifecycle and event loop. No alternate-screen writes escape
//! this boundary; the trace writer owns a different, private file.
use crate::editor::{self, events::AppEvent, Editor, Mode};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::{self, Write};
use std::time::Duration;
use strop_trace::{record_with, EventKind};

struct TerminalLease;
impl Drop for TerminalLease {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
    }
}

pub fn run(mut editor: Editor) -> io::Result<()> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        previous_hook(info);
    }));
    enable_raw_mode()?;
    let _lease = TerminalLease;
    let mut output = io::stdout();
    crossterm::execute!(output, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(output))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    editor.connect_events(sender.clone());
    std::thread::spawn(move || loop {
        let application_event = match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Release => continue,
            Ok(Event::Key(key))
                if (key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c'))
                    || key.code == KeyCode::Char('\x03') =>
            {
                AppEvent::QuitIntent
            }
            Ok(Event::Key(key)) => match key_from_event(key) {
                Some(key) => AppEvent::Terminal(key),
                None => continue,
            },
            Ok(Event::Paste(text)) => AppEvent::Paste(text),
            Ok(Event::Resize(columns, rows)) => {
                record_with(
                    EventKind::Resize,
                    || serde_json::json!({"columns":columns,"rows":rows}),
                );
                AppEvent::Resize
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
        if sender.send(application_event).is_err() {
            break;
        }
    });
    editor.trace_state();
    while !editor.should_quit {
        use crossterm::cursor::SetCursorStyle;
        let shape = if editor.input_normal() {
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
        terminal.draw(|frame| editor::trace::frame::draw(&mut editor, frame))?;
        let event = if editor.flash_range().is_some() {
            match receiver.recv_timeout(Duration::from_millis(16)) {
                Ok(event) => Some(event),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            receiver.recv().ok()
        };
        let Some(event) = event else { continue };
        editor.handle_app_event(event);
        editor.lsp_sync_changed();
        editor.trace_state();
        if let Some(payload) = editor.osc52.take() {
            write!(
                terminal.backend_mut(),
                "\x1b]52;c;{}\x07",
                base64_encode(payload.as_bytes())
            )?;
            terminal.backend_mut().flush()?;
        }
    }
    crate::session::save(&editor);
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

/// crossterm event → editor Key (terminals that deliver raw control
/// bytes instead of Char(letter)+CONTROL included — Windows Terminal →
/// WSL among them).
fn key_from_event(ev: crossterm::event::KeyEvent) -> Option<editor::Key> {
    use editor::Key;
    Some(match ev.code {
        KeyCode::Esc => Key::Esc,
        KeyCode::Enter => Key::Enter,
        KeyCode::Char('d') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlD,
        KeyCode::Char('u') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlU,
        KeyCode::Char('f') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlF,
        KeyCode::Char('b') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlB,
        KeyCode::Char('6') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlCaret,
        KeyCode::Char('\x1e') => Key::CtrlCaret,
        KeyCode::Char('\x04') => Key::CtrlD,
        KeyCode::Char('o') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlO,
        KeyCode::Char('\x0f') => Key::CtrlO,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Tab => Key::Tab,
        KeyCode::Char('\x12') => Key::CtrlR,
        KeyCode::Char('\x17') => Key::CtrlW,
        KeyCode::Char('\x18') => Key::CtrlX,
        KeyCode::BackTab => Key::Backtab,
        // arrows were dropped by the catch-all once — pickers, cmd
        // line, buffers all speak hjkl through these
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Char('n') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Down,
        KeyCode::Char('p') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Up,
        KeyCode::Char('r') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlR,
        KeyCode::Char('x') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlX,
        KeyCode::Char('l') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlL,
        KeyCode::Char('\x0c') => Key::CtrlL,
        KeyCode::Char('v') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlV,
        KeyCode::Char('\x16') => Key::CtrlV,
        KeyCode::Char('w') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlW,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}
