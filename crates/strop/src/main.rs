//! strop — prototype binary. TUI by default; `--headless` for the
//! scripted, deterministic driver (0006 tier 2).

mod config;
mod editor;
mod headless;
mod render;
mod session;
mod update;

use std::io::{self, Write};
use std::time::Duration;

use editor::{Editor, Key};
use strop_core::Buffer;

fn print_help() {
    println!(
        "strop {} — see the cut before you make it\n\
         \n\
         USAGE:\n\
         \x20   strop [file…]            open files (missing files are new buffers)\n\
         \x20   strop update [--check]   self-update (tarball installs)\n\
         \x20   strop config             print the config knobs with live values\n\
         \x20   strop --version          print version\n\
         \x20   strop --headless S [F]   scripted driver (tests, demos)\n\
         \n\
         KEYS (the vim grammar, live preview):\n\
         \x20   h j k l w b e 0 $ gg G %   motions      i a A o O          insert\n\
         \x20   d y c > < + motion/object  operators    dd yy cc D C Y s x  shortcuts\n\
         \x20   iw i\" i' i( i[ i{{        text objects  f t /              find & search\n\
         \x20   v V                       visual        u ctrl-r .          undo, redo, repeat\n\
         \x20   \"a …                      registers     :w :q :e            ex line\n\
         \n\
         SPACE (leader):\n\
         \x20   f files · b buffers · / grep · j jumplist (soon)\n\
         \x20   g git: l log · h file history · b blame · y/o permalink · u/s/p hunk\n\
         \x20   ? keybindings (soon)\n\
         \n\
         CONFIG: {}\n\
         \x20   tab_size = 4 · indent_guides = true\n\
         \n\
         https://strop.dev · https://github.com/stropdev/strop",
        env!("CARGO_PKG_VERSION"),
        config_path_display()
    );
}

fn config_path_display() -> String {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|b| b.join("strop").join("config.toml").display().to_string())
        .unwrap_or_else(|| "no config dir".into())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "config") {
        let (cfg, err) = config::Config::load();
        if let Some(e) = err {
            eprintln!("warning: {e}");
        }
        println!("strop config ({}):", config_path_display());
        cfg.print_knobs();
        return;
    }
    if args.first().is_some_and(|a| a == "update") {
        let check_only = args.iter().any(|a| a == "--check");
        if let Err(e) = update::update(check_only) {
            eprintln!("strop update: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
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
        let (cfg, _) = config::Config::load();
        let mut editor = Editor::new(buf);
        editor.config = cfg;
        let mut out = io::stdout().lock();
        headless::run_script(&mut editor, &script, 100, 30, &mut out);
        let _ = out.flush();
        return;
    }

    let (cfg, config_err) = config::Config::load();
    let path = args.iter().find(|a| !a.starts_with('-'));
    let buf = match path {
        Some(p) => Buffer::open(p).unwrap_or_else(|e| {
            eprintln!("strop: open {p}: {e}");
            std::process::exit(1);
        }),
        None => Buffer::from_text(""),
    };
    let mut editor = Editor::new(buf);
    editor.config = cfg;
    if let Some(e) = config_err {
        editor.message = e;
    }
    // per-project session: restore where we left off (0001 pillar 4) —
    // explicit file args beat the session
    if path.is_none() {
        session::restore(&mut editor);
    }
    tui(editor);
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
        use crossterm::cursor::SetCursorStyle;
        use crossterm::ExecutableCommand;
        let mut out = io::stdout();
        let shape = match editor.mode {
            editor::Mode::Insert => SetCursorStyle::SteadyBar,
            editor::Mode::Visual | editor::Mode::VisualLine => SetCursorStyle::SteadyUnderScore,
            editor::Mode::Normal => SetCursorStyle::SteadyBlock,
        };
        let _ = out.execute(shape);
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
            editor.drain_git_jobs();
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
            KeyCode::Char('r') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlR,
            KeyCode::Char('w') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlW,
            KeyCode::Char(c) => Key::Char(c),
            _ => continue,
        };
        editor.feed(key);
        editor.drain_picker();
        editor.drain_git_jobs();
        if let Some(payload) = editor.osc52.take() {
            // OSC52: system clipboard over the escape sequence — the
            // ssh-into-a-server answer (0001 pillar 4)
            let b64 = base64_encode(payload.as_bytes());
            let mut out = io::stdout();
            let _ = write!(out, "\x1b]52;c;{b64}\x07");
            let _ = out.flush();
        }
    }

    disable_raw_mode().unwrap();
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
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
