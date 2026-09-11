//! strop: native terminal editing and the same editor in the scripted driver.
mod bench;
mod cli;
mod headless;
mod render;
mod replay;
mod terminal;
mod update;

use strop_engine::{config, editor, files, keymap, session};

use std::error::Error;
use std::io::{self, Write};
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
            "headless":matches!(options.command, cli::Command::Headless { .. }),
        })
    });
    let result = execute(options.command);
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

fn execute(command: cli::Command) -> Result<(), Box<dyn Error>> {
    match command {
        cli::Command::Help => print_help(),
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
        cli::Command::ReplayFull { .. } | cli::Command::ExportMetadata { .. } => {
            return Err("replay/export must run outside live startup".into());
        }
        cli::Command::Headless {
            script,
            path,
            remote_view,
        } => {
            let script = std::fs::read_to_string(script)?;
            let buffer = path
                .as_ref()
                .and_then(|location| location.path.local_path())
                .map(Buffer::open)
                .transpose()?
                .unwrap_or_else(|| Buffer::from_text(""));
            let mut editor = editor::Editor::new(buffer);
            let (configuration, error) = config::Config::load();
            editor.config = configuration;
            editor.reresolve_indents();
            editor.state_dir = session::state_root();
            if let Some(error) = error {
                editor.message = error;
            }
            apply_initial_line(
                &mut editor,
                path.as_ref().and_then(|location| location.line),
            );
            headless::run_script(
                &mut editor,
                &script,
                100,
                30,
                &mut io::stdout().lock(),
                remote_start(&path, remote_view),
            )?;
        }
        cli::Command::Edit {
            path,
            readonly,
            remote_view,
        } => {
            let directory = path
                .as_ref()
                .and_then(|location| location.path.local_path())
                .filter(|path| path.is_dir())
                .map(std::path::Path::to_owned);
            if directory.is_some()
                && path
                    .as_ref()
                    .is_some_and(|location| location.line.is_some())
            {
                return Err("a line location requires a file, not a directory".into());
            }
            if let Some(directory) = &directory {
                std::env::set_current_dir(directory)?;
            }
            let buffer = match path
                .as_ref()
                .and_then(|location| location.path.local_path())
                .filter(|_| directory.is_none())
            {
                Some(path) => Buffer::open(path)?,
                None => Buffer::from_text(""),
            };
            let mut editor = editor::Editor::new(buffer);
            editor.frame_draw = Some(headless::frame_draw);
            editor.buf_mut().readonly = readonly;
            let (configuration, error) = config::Config::load();
            editor.config = configuration;
            editor.reresolve_indents();
            editor.state_dir = session::state_root();
            if path.is_none() {
                if let Err(error) = session::restore(&mut editor) {
                    editor.message = format!("session restore failed: {error}");
                }
            }
            apply_initial_line(
                &mut editor,
                path.as_ref().and_then(|location| location.line),
            );
            editor.trace_state();
            if let Some(error) = error {
                editor.message = error;
            }
            if editor.tape.observes() {
                editor
                    .tape
                    .seed(&editor::trace::seed::Seed::capture(&editor)?)?;
            }
            let tick = editor.tape.sample_tick();
            editor.recorded_action(
                editor::trace::drive::Action::Start {
                    directory_picker: directory.is_some(),
                    open: remote_start(&path, remote_view),
                },
                tick,
            )?;
            terminal::run(editor)?;
        }
    }
    io::stdout().flush()?;
    Ok(())
}
fn remote_start(
    location: &Option<cli::FileLocation>,
    view: editor::remote::RemoteView,
) -> Option<editor::trace::drive::StartupOpen> {
    let location = location.as_ref()?;
    matches!(location.path, files::FileTarget::Remote(_)).then(|| {
        editor::trace::drive::StartupOpen {
            target: location.path.clone(),
            line: location.line,
            view,
        }
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
    println!(
        "strop {} — see the cut before you make it\n\n\
USAGE:\n  strop [+LINE] [FILE[:LINE]|DIR] terminal editor (-R: readonly)\n\
  strop --headless SCRIPT [+LINE] [FILE[:LINE]]  scripted driver\n\
  strop --script SCRIPT [+LINE] [FILE[:LINE]]    same scripted driver\n\
  strop --replay-script TRACE     extract a headless reproduction script\n\
  strop update [--check]          self-update\n\
  strop config | --version | --dump-compat\n\n\
  strop -- FILE:3                open a literal colon-suffixed filename\n\n\
  strop [+LINE] ssh://[user@]host[:port]/absolute/path[:LINE]  remote file or directory\n\
REMOTE: :remote edit enables saving; :remote verify reconciles an unconfirmed\n\
  save; :browse :filter :tail :range :follow :remote connect/list — :help lists all\n\
TRACING:\n  --log / --log=ALL              all diagnostic categories to strop-log.jsonl\n\
  --log=PATH / --log-file PATH    create a new private JSONL file\n\
  STROP_LOG=PATH                 environment alternative (flag wins)\n\
  --log-content                 include file/paste text, LSP payloads and rendered cells\n\
  Keys, commands, paths and messages can be sensitive even without content.\n\
  Inspect logs before sharing. Existing files are never overwritten.\n\n\
KEYS: h j k l w b e 0 $ gg G %; d y c + motion; i a A o O; v V; u ctrl-r\n\
  / ? search · ctrl-l repaint · :w :q :e · Space ? lists every binding\n\
CONFIG: {}\nhttps://strop.dev · https://github.com/stropdev/strop",
        env!("CARGO_PKG_VERSION"),
        config_path_display()
    );
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
