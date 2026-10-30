//! strop: native terminal editing and the same editor in the scripted driver.
mod bench;
mod cli;
mod headless;
mod render;
mod replay;
mod terminal;
mod ui_stdio;
mod update;

use strop_engine::{config, editor, files, keymap, session};

use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use strop_core::Buffer;
use strop_trace::{record_with, EventKind, TraceOptions};

fn main() -> ExitCode {
    match launch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("strop: {error}");
            ExitCode::FAILURE
        }
    }
}

fn launch() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    // `strop --worker-stdio` (0058 WK04): the unified native worker.
    // This entry precedes CLI parsing, TUI setup, session restoration,
    // tracing and config: stdout is protocol frames from the first
    // byte, diagnostics stay on private bounded stderr. (The separate
    // `--terminal-helper` process is gone in WK12: PTYs are the
    // worker's own exec family now.)
    #[cfg(unix)]
    if arguments
        .first()
        .is_some_and(|argument| argument == "--worker-stdio")
    {
        if arguments.len() != 1 {
            return Err("worker mode takes no arguments".into());
        }
        // StdoutLock is !Send (worker request threads write too); the
        // shared `Stdout` handle locks per write and serve's own writer
        // mutex holds frame atomicity across the header/body/flush calls.
        strop_worker::serve::run(io::stdin().lock(), io::stdout())?;
        return Ok(());
    }
    let options = cli::parse(arguments.clone())?;
    match &options.command {
        cli::Command::ReplayFull { trace } => {
            return replay::run_full(trace, &mut io::stdout().lock()).map_err(Into::into);
        }
        cli::Command::ExportMetadata { trace } => {
            let source = io::BufReader::new(std::fs::File::open(trace)?);
            strop_trace::export::metadata(source, &mut io::stdout().lock())?;
            return Ok(());
        }
        cli::Command::UiStdio => {
            // Early like replay/export (0056 AR09): stdout is reserved
            // for protocol frames from the first byte; bounded
            // diagnostics go to stderr.
            return ui_stdio::run(options.theme);
        }
        _ => {}
    }
    let trace_session = options
        .trace_path
        .as_deref()
        .map(|path| {
            strop_trace::start(
                path,
                TraceOptions {
                    content: options.content,
                    ..TraceOptions::default()
                },
            )
        })
        .transpose()?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_with(EventKind::Panic, || {
            serde_json::json!({
                "message":info.to_string(), "backtrace":std::backtrace::Backtrace::force_capture().to_string(),
            })
        });
        previous_hook(info);
    }));
    record_with(EventKind::SessionStart, || {
        serde_json::json!({
            "version":env!("CARGO_PKG_VERSION"),
            "argv":arguments.iter().map(|value|value.to_string_lossy()).collect::<Vec<_>>(),
            "cwd":std::env::current_dir().ok().map(|p|p.to_string_lossy().into_owned()),
            "platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "term":std::env::var("TERM").ok(), "colorterm":std::env::var("COLORTERM").ok(),
            "full_content":strop_trace::capture_content(),
            "terminal_content":options.terminal_capture,
            "headless":matches!(options.command, cli::Command::Headless { .. }),
        })
    });
    let result = execute(options.command, options.terminal_capture, options.theme);
    if let Err(error) = &result {
        record_with(
            EventKind::Error,
            || serde_json::json!({"source":"application","message":error.to_string()}),
        );
    }
    record_with(
        EventKind::SessionEnd,
        || serde_json::json!({"success":result.is_ok()}),
    );
    let trace_result = trace_session.map(|session| session.finish()).transpose();
    result?;
    trace_result?;
    Ok(())
}

fn execute(
    command: cli::Command,
    terminal_capture: bool,
    theme_flag: Option<strop_core::theme::ThemeId>,
) -> Result<(), Box<dyn Error>> {
    match command {
        cli::Command::Help => print_help(),
        cli::Command::ThemeLicenses => {
            println!(
                "Built-in theme licenses\n\n{}",
                strop_core::theme::THEME_NOTICES
            );
        }
        cli::Command::Version => println!("strop {}", env!("CARGO_PKG_VERSION")),
        cli::Command::Compat => print!("{}", keymap::compat_report()),
        cli::Command::Config => {
            let (configuration, error) = config::Config::load();
            if let Some(error) = error {
                eprintln!("warning: {error}");
            }
            println!("strop config ({}):", config_path_display());
            configuration.print_knobs();
        }
        cli::Command::Update { check_only } => update::update(check_only)?,
        cli::Command::Bench { scenario } => bench::run(&scenario)?,
        cli::Command::Replay { trace } => replay::write_script(&trace, &mut io::stdout().lock())?,
        cli::Command::ReplayFull { .. }
        | cli::Command::ExportMetadata { .. }
        | cli::Command::UiStdio => {
            return Err("replay/export/ui-stdio must run outside live startup".into());
        }
        cli::Command::Headless {
            script,
            path,
            remote_view,
        } => {
            let script = std::fs::read_to_string(script)?;
            let (buffer, directory) = initial_buffer(&path)?;
            let mut editor = editor::Editor::new(buffer);
            editor.set_terminal_capture(terminal_capture);
            editor.set_terminal_keyboard(strop_terminal::model::SUPPORTED_KEYBOARD_FLAGS)?;
            let (configuration, error) = config::Config::load();
            strop_core::theme::set_current(theme_flag.unwrap_or(configuration.theme));
            editor.set_config(configuration);
            editor.reresolve_indents();
            editor.set_state_dir(session::state_root());
            if let Some(error) = error {
                editor.set_message(error);
            }
            apply_initial_line(
                &mut editor,
                path.as_ref().and_then(|location| location.line),
            );
            let open = startup_open(&path, remote_view, directory.then_some(editor.cwd()));
            headless::run_script(
                &mut editor,
                &script,
                100,
                30,
                &mut io::stdout().lock(),
                open,
            )?;
        }
        cli::Command::Edit {
            path,
            readonly,
            remote_view,
        } => {
            let (mut buffer, directory) = initial_buffer(&path)?;
            // The CLI --readonly flag lands on the buffer before
            // admission; the editor never sees a post-hoc mutation.
            buffer.readonly = readonly;
            let mut editor = editor::Editor::new(buffer);
            editor.set_terminal_capture(terminal_capture);
            editor.set_frame_draw(Some(headless::frame_draw));
            let (configuration, error) = config::Config::load();
            strop_core::theme::set_current(theme_flag.unwrap_or(configuration.theme));
            editor.set_config(configuration);
            editor.reresolve_indents();
            editor.set_state_dir(session::state_root());
            if path.is_none() {
                if let Err(error) = session::restore(&mut editor) {
                    editor.set_message(format!("session restore failed: {error}"));
                }
            }
            apply_initial_line(
                &mut editor,
                path.as_ref().and_then(|location| location.line),
            );
            editor.trace_state();
            if let Some(error) = error {
                editor.set_message(error);
            }
            if editor.tape().observes() {
                editor
                    .tape()
                    .seed(&editor::trace::seed::Seed::capture(&editor)?)?;
            }
            let tick = editor.tape().sample_tick();
            editor.recorded_action(
                editor::trace::drive::Action::Start {
                    open: startup_open(&path, remote_view, directory.then_some(editor.cwd())),
                },
                tick,
            )?;
            terminal::run(editor)?;
        }
    }
    io::stdout().flush()?;
    Ok(())
}
fn initial_buffer(location: &Option<cli::FileLocation>) -> Result<(Buffer, bool), Box<dyn Error>> {
    let local = location
        .as_ref()
        .and_then(|location| location.path.local_path());
    let directory = local.filter(|path| path.is_dir());
    if directory.is_some()
        && location
            .as_ref()
            .is_some_and(|location| location.line.is_some())
    {
        return Err("a line location requires a file, not a directory".into());
    }
    if let Some(directory) = directory {
        std::env::set_current_dir(directory)?;
    }
    let buffer = match local.filter(|_| directory.is_none()) {
        Some(path) => Buffer::open(path)?,
        None => Buffer::from_text(""),
    };
    Ok((buffer, directory.is_some()))
}

fn startup_open(
    location: &Option<cli::FileLocation>,
    view: editor::remote::RemoteView,
    directory: Option<&std::path::Path>,
) -> Option<editor::trace::drive::StartupOpen> {
    if let Some(directory) = directory {
        return Some(editor::trace::drive::StartupOpen {
            target: files::FileTarget::Local(directory.to_path_buf()),
            intent: editor::io::OpenIntent::Browse,
        });
    }
    let location = location.as_ref()?;
    let intent = match location.path {
        files::FileTarget::Remote(_) => editor::io::OpenIntent::RemoteView {
            view,
            line: location.line,
        },
        files::FileTarget::Container { .. } => editor::io::OpenIntent::Switch { readonly: true },
        files::FileTarget::Local(_) => return None,
    };
    Some(editor::trace::drive::StartupOpen {
        target: location.path.clone(),
        intent,
    })
}

fn apply_initial_line(editor: &mut editor::Editor, line: Option<strop_core::id::LineIndex>) {
    if let Some(line) = line {
        let line = line.get().min(editor.buf().last_content_line());
        editor.set_head(editor.buf().line_start(line));
        editor.run_motion("^");
    }
}

fn print_help() {
    let color = std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb");
    let paint = |code: &str, text: &str| {
        if color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    };
    fn section(out: &mut String, title: &str) {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(title);
        out.push('\n');
    }
    fn usage(out: &mut String, lines: &[&str]) {
        for line in lines {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    fn rows(out: &mut String, entries: &[(&str, &str)]) {
        let width = entries
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (name, about) in entries {
            out.push_str(&format!("  {name:<width$}  {about}\n", width = width));
        }
    }
    let mut out = String::new();

    out.push_str(&paint(
        "1",
        &format!(
            "strop {} — see the cut before you make it",
            env!("CARGO_PKG_VERSION")
        ),
    ));
    out.push_str("\n\nThe modal terminal editor: vim's grammar, a live preview for every\n");
    out.push_str("cut, and one query language across every picker.\n");
    section(&mut out, &paint("1", "Usage:"));
    usage(
        &mut out,
        &["strop [OPTIONS] [FILE[:LINE] | DIR | ssh://URL]"],
    );
    section(&mut out, &paint("1", "Commands:"));
    rows(
        &mut out,
        &[
            (
                "edit",
                "open a file, directory, or an empty buffer (the default)",
            ),
            ("remote", "open an ssh:// file or directory for editing"),
            ("headless", "run the scripted driver over a file"),
            (
                "replay",
                "replay a session trace (--script extracts a driver script)",
            ),
            (
                "export",
                "export a trace's metadata (privacy-safe, not replayable)",
            ),
            ("ui-stdio", "UI protocol backend on stdin/stdout"),
            ("config", "show the config file and its current values"),
            ("update", "self-update (--check only checks)"),
            ("bench", "run the performance-gate scenarios"),
            ("licenses", "built-in theme attributions"),
            ("compat", "dump the vim-compatibility table"),
            ("help", "print this help"),
        ],
    );
    section(&mut out, &paint("1", "Options:"));
    rows(
        &mut out,
        &[
            ("-R, --readonly", "open read-only"),
            ("+LINE", "initial line for FILE (or use FILE:LINE)"),
            (
                "    --theme NAME",
                "session-start palette (see THEMES below)",
            ),
            ("    --remote URL", "shorthand for `strop remote URL`"),
            ("    --follow", "follow a growing remote file"),
            ("    --tail BYTES", "remote view: the last BYTES"),
            ("    --range A:B", "remote view: one byte range"),
            (
                "    --script PATH",
                "headless driver script (same as `headless`)",
            ),
            (
                "    --log[=PATH]",
                "record a session trace (STROP_LOG works too)",
            ),
            (
                "    --log-content",
                "include file text in the trace (sensitive)",
            ),
            (
                "    --log-terminal-content",
                "also terminal keys and output",
            ),
            ("-h, --help", "print help"),
            ("-V, --version", "print version"),
        ],
    );
    section(&mut out, &paint("1", "Remote:"));
    usage(&mut out, &["ssh://[user@]host[:port]/absolute/path[:LINE]"]);
    out.push_str(&paint(
        "2",
        "  :remote edit enables saving · :remote verify reconciles saves\n  :browse · :filter · :tail · :range · :follow · :remote connect/list\n",
    ));
    section(&mut out, &paint("1", "Keys:"));
    out.push_str(&paint(
        "2",
        "  h j k l w b e 0 $ gg G % · d y c + motion · i a A o O · v V · u ctrl-r\n  / ? search · ctrl-l repaint · :w :q :e · Space ? lists every binding\n",
    ));
    section(&mut out, &paint("1", "Config:"));
    out.push_str(&format!("  {}\n", config_path_display()));
    out.push_str(&paint(
        "2",
        "Themes: :theme live · strop licenses lists attributions\nhttps://strop.dev · https://github.com/stropdev/strop\n",
    ));
    print!("{out}");
    println!("\n{}", paint("1", "THEMES (select live with :theme):"));
    for id in strop_core::theme::ThemeId::ALL {
        println!("  {:<20} {} — {}", id.name(), id.label(), id.description());
    }
    headless::directives::print_help();
}

fn config_path_display() -> String {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .map(|path| path.join("strop/config.toml").display().to_string())
        .unwrap_or_else(|| "no config dir".into())
}
