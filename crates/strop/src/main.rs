//! strop — prototype binary. TUI by default; `--headless` for the
//! scripted, deterministic driver (0006 tier 2).

mod editor;
mod headless;
mod render;

use std::io::{self, Write};
use std::time::Duration;

use editor::{Editor, Key};
use strop_core::Buffer;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("strop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--headless") {
        let file = args.get(i + 1).filter(|a| !a.starts_with('-')).cloned();
        let script = file
            .as_deref()
            .map(std::fs::read_to_string)
            .transpose()
            .unwrap_or_else(|e| panic!("read script: {e}"))
            .or_else(|| args.get(i + 2).cloned())
            .unwrap_or_default();
        let path = args
            .iter()
            .find(|a| !a.starts_with('-') && Some(*a) != file.as_ref());
        let buf = path.map_or_else(
            || Buffer::from_text(""),
            |p| Buffer::open(p).unwrap_or_else(|e| panic!("open {p}: {e}")),
        );
        let mut editor = Editor::new(buf);
        let mut out = io::stdout().lock();
        headless::run_script(&mut editor, &script, 100, 30, &mut out);
        let _ = out.flush();
        return;
    }

    let path = args.iter().find(|a| !a.starts_with('-'));
    let buf = match path {
        Some(p) => Buffer::open(p).unwrap_or_else(|e| {
            eprintln!("strop: open {p}: {e}");
            std::process::exit(1);
        }),
        None => Buffer::from_text(""),
    };
    tui(Editor::new(buf));
}

fn tui(mut editor: Editor) {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };

    enable_raw_mode().unwrap();
    let mut out = io::stdout();
    crossterm::execute!(out, EnterAlternateScreen).unwrap();
    let backend = ratatui::backend::CrosstermBackend::new(out);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();

    loop {
        // quit check before draw: the last :q leaves zero buffers and
        // rendering needs one (caught by the demo tape)
        if editor.should_quit {
            break;
        }
        terminal.draw(|f| render::render(&mut editor, f)).unwrap();
        // 16ms cap so the flash overlay expires on time; input-to-echo
        // stays under one frame (0001 §4).
        let timeout = if editor.flash_range().is_some() {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(500)
        };
        if !event::poll(timeout).unwrap() {
            editor.drain_picker();
            continue;
        }
        let Event::Key(ev) = event::read().unwrap() else {
            continue;
        };
        if ev.modifiers.contains(KeyModifiers::CONTROL) && ev.code == KeyCode::Char('c') {
            break;
        }
        let key = match ev.code {
            KeyCode::Esc => Key::Esc,
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Tab => Key::Tab,
            KeyCode::BackTab => Key::Backtab,
            KeyCode::Char('n') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Down,
            KeyCode::Char('p') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::Up,
            KeyCode::Char(c) => Key::Char(c),
            _ => continue,
        };
        editor.feed(key);
        editor.drain_picker();
    }

    disable_raw_mode().unwrap();
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
}
