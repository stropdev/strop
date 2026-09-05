//! strop — prototype binary. TUI by default; `--headless` for the
//! scripted, deterministic driver (0006 tier 2).

mod config;
mod editor;
mod headless;
mod keymap;
mod render;
mod session;
mod update;

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyModifiers};
use std::time::Duration;

use editor::Editor;
use strop_core::Buffer;

fn print_help() {
    println!(
        "strop {} — see the cut before you make it\n\
         \n\
         USAGE:\n\
         \x20   strop [file|dir]         open file, or land on the file picker in dir\n\
         \x20   strop update [--check]   self-update (tarball installs)\n\
         \x20   strop config             print the config knobs with live values\n\
         \x20   strop --version          print version\n\
         \x20   strop --headless S [F]   scripted driver (tests, demos)\n\
         \n\
         KEYS (the vim grammar, live preview):\n\
         \x20   h j k l w b e 0 $ gg G %   motions      i a A o O          insert\n\
         \x20   d y c > < + motion/object  operators    dd yy cc D C Y s x  shortcuts\n\
         \x20   iw i\" i' i( i[ i{{        text objects  f t / ? n N        find & search\n\
         \x20   v V                       visual        u ctrl-r .          undo, redo, repeat\n\
         \x20   \"a … \"+                   registers     :w :q :e            ex line\n\
         \n\
         SPACE (leader):\n\
         \x20   f files · b buffers · / grep · R replace · d diagnostics · k hover\n\
         \x20   y/p/P system clipboard · j jumplist (soon) · u undo tree (soon)\n\
         \x20   g git: l log · h file history · b blame gutter · y/o permalink · u/s/p hunk\n\
         \x20   ? keybindings\n\
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
    if args.iter().any(|a| a == "--dump-compat") {
        // the vim-compatibility report, generated from the command
        // table (0016: docs can't drift from dispatch)
        print!("{}", keymap::compat_report());
        std::process::exit(0);
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
            .unwrap_or_else(|e| {
                // a user-supplied path — a typo deserves an error, not a
                // backtrace
                eprintln!("strop: read script: {e}");
                std::process::exit(2);
            })
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
        editor.lsp_maybe_attach();
        let mut out = io::stdout().lock();
        headless::run_script(&mut editor, &script, 100, 30, &mut out);
        for srv in std::mem::take(&mut editor.lsp_servers) {
            srv.client.shutdown();
            srv.client.wait(Duration::from_millis(500));
        }
        let _ = out.flush();
        return;
    }

    let (cfg, config_err) = config::Config::load();
    let path = args.iter().find(|a| !a.starts_with('-'));
    // a directory arg means "project here": cd into it and land on the
    // file picker (helix's `hx .`), instead of erroring on EISDIR
    let dir_arg = path.filter(|p| std::path::Path::new(p).is_dir()).cloned();
    let buf = match &path {
        Some(p) if dir_arg.is_none() => Buffer::open(p).unwrap_or_else(|e| {
            eprintln!("strop: open {p}: {e}");
            std::process::exit(1);
        }),
        _ => Buffer::from_text(""),
    };
    let mut editor = Editor::new(buf);
    // vim -R / view: readonly browsing
    if args.iter().any(|a| a == "-R" || a == "--readonly") {
        editor.buf_mut().readonly = true;
    }
    if let Some(d) = &dir_arg {
        let dir = std::path::Path::new(d)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(d));
        if std::env::set_current_dir(&dir).is_ok() {
            editor.cwd = dir;
            editor.open_picker(strop_picker::Kind::Files);
        }
    }
    editor.config = cfg;
    editor.lsp_maybe_attach();
    if let Some(e) = config_err {
        editor.message = e;
    }
    // per-project session: restore where we left off (0001 pillar 4) —
    // explicit file args beat the session
    if path.is_none() {
        // XDG state dir resolves once here; the editor carries it so the
        // write path never reads process-global env mid-run
        editor.state_dir = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".local").join("state"))
            });
        session::restore(&mut editor);
    }
    tui(editor);
}

fn tui(mut editor: Editor) {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };

    // a panic must hand the terminal back — raw mode + the alt screen
    // otherwise swallow the user's shell along with the crash
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        default_hook(info);
    }));
    enable_raw_mode().unwrap();
    let mut out = io::stdout();
    crossterm::execute!(out, EnterAlternateScreen, EnableBracketedPaste).unwrap();
    let backend = ratatui::backend::CrosstermBackend::new(out);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();

    // 0018 §services: one event channel. The terminal reader thread
    // translates crossterm events to editor keys; every job's results
    // forward through editor.connect_events. The loop parks on the
    // channel — no 500ms poll between a job landing and the UI moving.
    use editor::events::AppEvent;
    let (app_tx, app_rx) = std::sync::mpsc::channel::<AppEvent>();
    editor.connect_events(app_tx.clone());
    std::thread::spawn({
        let tx = app_tx.clone();
        move || loop {
            match event::read() {
                Ok(Event::Key(ev))
                    if (ev.modifiers.contains(KeyModifiers::CONTROL)
                        && ev.code == KeyCode::Char('c'))
                        || ev.code == KeyCode::Char('\x03') =>
                {
                    if tx.send(AppEvent::QuitIntent).is_err() {
                        break;
                    }
                }
                Ok(Event::Key(ev)) => {
                    let Some(key) = key_from_event(ev) else {
                        continue;
                    };
                    if tx.send(AppEvent::Terminal(key)).is_err() {
                        break;
                    }
                }
                Ok(Event::Paste(text)) => {
                    if tx.send(AppEvent::Paste(text)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

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
            editor::Mode::Insert if editor.input_normal() => SetCursorStyle::SteadyBlock,
            editor::Mode::Insert => SetCursorStyle::SteadyBar,
            editor::Mode::Visual | editor::Mode::VisualLine | editor::Mode::VisualBlock => {
                SetCursorStyle::SteadyUnderScore
            }
            // insert-mode input fields (picker fields, the : line) want
            // the bar even when the editor's mode is Normal
            _ if editor.input_normal() => SetCursorStyle::SteadyBlock,
            _ if editor.picker_open()
                || (editor.pending.starts_with([':', '/', '|']) && !editor.pending_normal) =>
            {
                SetCursorStyle::SteadyBar
            }
            // modal input boxes sit in normal-mode semantics on the
            // editor's normal mode too — block says so (rootle's boxes)
            _ if editor.input_normal() => SetCursorStyle::SteadyBlock,
            editor::Mode::Normal => SetCursorStyle::SteadyBlock,
        };
        let _ = out.execute(shape);
        terminal.draw(|f| render::render(&mut editor, f)).unwrap();
        // a flash animation gets a 16ms frame budget; everything else
        // parks until an event lands
        let ev = if editor.flash_range().is_some() {
            match app_rx.recv_timeout(Duration::from_millis(16)) {
                Ok(ev) => Some(ev),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match app_rx.recv() {
                Ok(ev) => Some(ev),
                Err(_) => break,
            }
        };
        let Some(ev) = ev else { continue };
        editor.handle_app_event(ev);
        // a paste/key may change text — push LSP syncs promptly
        editor.lsp_sync_changed();
        if let Some(payload) = editor.osc52.take() {
            // OSC52: system clipboard over the escape sequence — the
            // ssh-into-a-server answer (0001 pillar 4)
            let b64 = base64_encode(payload.as_bytes());
            let mut out = io::stdout();
            let _ = write!(out, "\x1b]52;c;{b64}\x07");
            let _ = out.flush();
        }
    }

    // sessions persist on exit, not only on :w — quitting without a
    // write still restores where you were (0005 session layer)
    crate::session::save(&editor);

    // the LSP exit sequence must land before the pipes close with us —
    // and the runtime thread must be joined: dropping the socket under a
    // live mainloop panics inside async-lsp ("Sender is alive")
    for srv in std::mem::take(&mut editor.lsp_servers) {
        srv.client.shutdown();
        srv.client.wait(Duration::from_millis(500));
    }
    disable_raw_mode().unwrap();
    crossterm::execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )
    .unwrap();
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
        KeyCode::Char('v') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlV,
        KeyCode::Char('\x16') => Key::CtrlV,
        KeyCode::Char('w') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlW,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}
