//! Real terminal lifecycle and event loop. No alternate-screen writes escape
//! this boundary; the trace writer owns a different, private file.
use crate::editor::{self, events::AppEvent, Editor, Mode};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::{self, Write};
use std::time::Duration;
use strop_trace::{record_with, EventKind};

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
    crossterm::execute!(output, EnterAlternateScreen, EnableBracketedPaste)?;
    // Ask capable terminals for unambiguous Escape encoding (0048 §B).
    // Fire-and-forget: terminals without the protocol ignore the push
    // (and crossterm's native-Windows shim reports Unsupported) — the
    // legacy Alt normalization below stays the fallback everywhere.
    let _ = crossterm::execute!(
        output,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(output))?;
    let (sender, receiver) = editor::events::channel();
    let (input_sender, input_receiver) = editor::events::channel();
    editor.connect_events(sender);
    std::thread::spawn(move || loop {
        let application_events = match event::read() {
            Ok(Event::Key(key)) => expand_key_event(key),
            Ok(Event::Paste(text)) => [Some(AppEvent::Paste(text)), None],
            Ok(Event::Resize(columns, rows)) => {
                record_with(
                    EventKind::Resize,
                    || serde_json::json!({"columns":columns,"rows":rows}),
                );
                [Some(AppEvent::Resize { columns, rows }), None]
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
        // FIFO, in order: an expanded Escape never overtakes the key it
        // preceded, and neither overtakes earlier edits (0048 §A).
        for application_event in application_events.into_iter().flatten() {
            if input_sender.send(application_event).is_err() {
                return;
            }
        }
    });
    editor.trace_state();
    let mut redraw = true;
    let mut painted_flash = false;
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
        if redraw || ((flashing || painted_flash) && std::time::Instant::now() >= animation_due) {
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
            terminal.draw(|frame| crate::render::frame_capture::draw(&mut editor, frame, true))?;
            redraw = false;
            painted_flash = flashing;
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
    if let Some(error) = editor.io.session_error.take() {
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

/// The Escape-preserving normalization (0048 §A). Legacy terminals
/// encode `Esc` followed quickly by a key and a real Alt chord as the
/// same bytes, and crossterm decodes both as Alt+key — strop has no
/// independent Alt bindings, so Alt is an Escape prefix consistently:
/// strip only the Alt bit and deliver Escape, then the base key, in
/// order. An unmapped base key still yields the Escape (vim's esckeys
/// semantics). Release events are filtered BEFORE any expansion.
fn expand_key_event(key: crossterm::event::KeyEvent) -> [Option<AppEvent>; 2] {
    if key.kind == KeyEventKind::Release {
        return [None, None];
    }
    let is_ctrl_c = (key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Char('c'))
        || key.code == KeyCode::Char('\x03');
    if !key.modifiers.contains(KeyModifiers::ALT) {
        if is_ctrl_c {
            return [Some(AppEvent::QuitIntent), None];
        }
        return match key_from_event(key) {
            Some(key) => [Some(AppEvent::Terminal(key)), None],
            None => [None, None],
        };
    }
    // Alt-modified: Escape first, then the base key/action with only
    // the Alt bit removed (Ctrl/Shift survive the strip).
    let base = crossterm::event::KeyEvent {
        modifiers: key.modifiers - KeyModifiers::ALT,
        ..key
    };
    let esc = AppEvent::Terminal(editor::Key::Esc);
    if is_ctrl_c {
        // Alt+Ctrl+C: the leading Escape, then the quit intent exactly
        // once (0048 §A).
        return [Some(esc), Some(AppEvent::QuitIntent)];
    }
    match key_from_event(base) {
        Some(key) => [Some(esc), Some(AppEvent::Terminal(key))],
        None => [Some(esc), None],
    }
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
        KeyCode::Char(' ') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlSpace,
        KeyCode::Null | KeyCode::Char('\0') => Key::CtrlSpace,
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
        // CSI-u aliases under keyboard enhancement (0048 §B): the
        // encoded forms must keep their legacy meanings, never fall
        // through to literal letters.
        KeyCode::Char('i') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Tab,
        KeyCode::Char('m') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Enter,
        KeyCode::Char('[') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Esc,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    /// The expanded events as comparable shapes.
    fn expanded(events: [Option<AppEvent>; 2]) -> Vec<AppEvent> {
        events.into_iter().flatten().collect()
    }

    fn terminal_key(event: &AppEvent) -> Option<editor::Key> {
        match event {
            AppEvent::Terminal(key) => Some(*key),
            _ => None,
        }
    }

    #[test]
    fn alt_char_expands_to_escape_then_the_base_key() {
        // 0048 §A — the coalesced legacy bytes: Esc must survive and the
        // following key keeps its identity, in order.
        let events = expanded(expand_key_event(key(KeyCode::Char('h'), KeyModifiers::ALT)));
        let keys: Vec<_> = events.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Esc, editor::Key::Char('h')]);
    }

    #[test]
    fn alt_expansion_preserves_shift_and_non_ascii_identity() {
        let shifted = expanded(expand_key_event(key(
            KeyCode::Char('H'),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        )));
        let keys: Vec<_> = shifted.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Esc, editor::Key::Char('H')]);
        let non_ascii = expanded(expand_key_event(key(KeyCode::Char('ö'), KeyModifiers::ALT)));
        let keys: Vec<_> = non_ascii.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Esc, editor::Key::Char('ö')]);
    }

    #[test]
    fn alt_ctrl_d_keeps_the_control_action() {
        let events = expanded(expand_key_event(key(
            KeyCode::Char('d'),
            KeyModifiers::ALT | KeyModifiers::CONTROL,
        )));
        let keys: Vec<_> = events.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Esc, editor::Key::CtrlD]);
    }

    #[test]
    fn release_never_expands_and_repeat_retains() {
        let release = KeyEvent {
            kind: KeyEventKind::Release,
            ..key(KeyCode::Char('h'), KeyModifiers::ALT)
        };
        assert!(expanded(expand_key_event(release)).is_empty());
        let repeat = KeyEvent {
            kind: KeyEventKind::Repeat,
            ..key(KeyCode::Char('h'), KeyModifiers::ALT)
        };
        let events = expanded(expand_key_event(repeat));
        assert_eq!(events.iter().filter_map(terminal_key).count(), 2);
    }

    #[test]
    fn ctrl_c_quit_intent_survives_and_alt_ctrl_c_expands_once() {
        let plain = expanded(expand_key_event(key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(matches!(plain.as_slice(), [AppEvent::QuitIntent]));
        let alt = expanded(expand_key_event(key(
            KeyCode::Char('c'),
            KeyModifiers::ALT | KeyModifiers::CONTROL,
        )));
        assert_eq!(alt.len(), 2, "Escape then the quit intent, exactly once");
        assert!(matches!(terminal_key(&alt[0]), Some(editor::Key::Esc)));
        assert!(matches!(alt[1], AppEvent::QuitIntent));
    }

    #[test]
    fn csi_u_control_aliases_keep_legacy_meanings() {
        // 0048 §B: under keyboard enhancement these arrive encoded;
        // they must not fall through to literal letters.
        for (code, expected) in [
            (KeyCode::Char('i'), editor::Key::Tab),
            (KeyCode::Char('m'), editor::Key::Enter),
            (KeyCode::Char('['), editor::Key::Esc),
        ] {
            let events = expanded(expand_key_event(key(code, KeyModifiers::CONTROL)));
            let keys: Vec<_> = events.iter().filter_map(terminal_key).collect();
            assert_eq!(keys, [expected], "{code:?}");
        }
    }

    #[test]
    fn alt_with_an_unmapped_base_still_yields_escape() {
        let events = expanded(expand_key_event(key(KeyCode::F(7), KeyModifiers::ALT)));
        let keys: Vec<_> = events.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Esc]);
    }

    #[test]
    fn plain_keys_are_untouched() {
        let events = expanded(expand_key_event(key(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));
        let keys: Vec<_> = events.iter().filter_map(terminal_key).collect();
        assert_eq!(keys, [editor::Key::Char('x')]);
    }

    #[test]
    fn coalesced_escape_leaves_insert_and_runs_the_motion() {
        // 0048 §5.1 consumer regression: the production normalizer's
        // output through real event dispatch — text unchanged, NORMAL
        // mode, cursor byte 8 after Esc+h.
        let mut editor = crate::editor::Editor::new(strop_core::Buffer::from_text(""));
        editor.feed_text("iwritewrite");
        assert_eq!(editor.mode, crate::editor::Mode::Insert);
        let events = expanded(expand_key_event(key(KeyCode::Char('h'), KeyModifiers::ALT)));
        for event in events {
            editor.handle_app_event(event);
        }
        assert_eq!(editor.buf().text().to_string(), "writewrite");
        assert_eq!(editor.mode, crate::editor::Mode::Normal);
        assert_eq!(editor.head(), 8, "Escape landed, then h moved left");
    }

    #[test]
    fn coalesced_escape_is_not_motion_specific() {
        // A non-motion command too: Esc+x deletes a char in NORMAL —
        // an hjkl-only patch would fail this.
        let mut editor = crate::editor::Editor::new(strop_core::Buffer::from_text(""));
        editor.feed_text("iwritewrite");
        let events = expanded(expand_key_event(key(KeyCode::Char('x'), KeyModifiers::ALT)));
        for event in events {
            editor.handle_app_event(event);
        }
        assert_eq!(editor.mode, crate::editor::Mode::Normal);
        assert_eq!(
            editor.buf().text().to_string(),
            "writewrit",
            "x deleted a char"
        );
    }
    #[test]
    fn legacy_nul_opens_query_suggestions_in_the_real_input_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = crate::editor::Editor::new_in(
            strop_core::Buffer::from_text(""),
            dir.path().to_path_buf(),
        );
        editor.open_picker(strop_picker::Kind::Files);
        editor.feed_text("lang");
        for event in expanded(expand_key_event(key(KeyCode::Null, KeyModifiers::NONE))) {
            editor.handle_app_event(event);
        }
        editor.feed(crate::editor::Key::Enter);
        assert_eq!(
            editor.picker.as_ref().unwrap().picker.input.text,
            "language:"
        );
    }
}
