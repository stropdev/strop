//! Command-line parsing. File operands remain OsString/PathBuf end to end.
use std::ffi::OsString;
use std::path::PathBuf;
use strop_trace::ContentPolicy;

mod location;
pub(crate) use location::FileLocation;

#[derive(Debug)]
pub enum Command {
    Edit {
        path: Option<FileLocation>,
        readonly: bool,
    },
    Headless {
        script: PathBuf,
        path: Option<FileLocation>,
    },
    Replay {
        trace: PathBuf,
    },
    /// `--replay TRACE`: full forensic replay of a complete full-content
    /// capture (R11). Distinct from `--replay-script` input extraction.
    ReplayFull {
        trace: PathBuf,
    },
    /// `--export-metadata TRACE`: positive-projection privacy export;
    /// explicitly not replayable.
    ExportMetadata {
        trace: PathBuf,
    },
    Help,
    Version,
    Compat,
    Config,
    Update {
        check_only: bool,
    },
    Bench {
        scenario: String,
    },
}
#[derive(Debug)]
pub struct Options {
    pub command: Command,
    pub trace_path: Option<PathBuf>,
    pub content: ContentPolicy,
}

pub fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut args = args.into_iter().peekable();
    let mut trace_path = std::env::var_os("STROP_LOG")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let mut content = ContentPolicy::Metadata;
    let mut readonly = false;
    let mut initial_line = None;
    let mut headless = false;
    let mut script = None;
    let mut replay = None;
    let mut operand = None;
    let mut command = None;
    let mut positional = false;
    while let Some(argument) = args.next() {
        let text = argument.to_string_lossy();
        if !positional {
            if let Some(line) = text.strip_prefix('+') {
                if initial_line.is_some() {
                    return Err(location::LocationError::DuplicateLine.to_string());
                }
                initial_line = Some(location::parse_line(line).map_err(|error| error.to_string())?);
                continue;
            }
            match text.as_ref() {
                "--" => {
                    positional = true;
                    continue;
                }
                "--help" | "-h" => {
                    command = Some(Command::Help);
                    continue;
                }
                "--version" | "-V" => {
                    command = Some(Command::Version);
                    continue;
                }
                "--dump-compat" => {
                    command = Some(Command::Compat);
                    continue;
                }
                "--readonly" | "-R" => {
                    readonly = true;
                    continue;
                }
                "--log" | "--log=ALL" => {
                    trace_path = Some("strop-log.jsonl".into());
                    continue;
                }
                "--log-file" => {
                    trace_path = Some(args.next().ok_or("--log-file requires a path")?.into());
                    continue;
                }
                "--log-content" => {
                    content = ContentPolicy::Full;
                    continue;
                }
                "--headless" => {
                    headless = true;
                    if args
                        .peek()
                        .is_some_and(|value| !value.to_string_lossy().starts_with('-'))
                    {
                        script = args.next().map(PathBuf::from);
                    }
                    continue;
                }
                "--script" => {
                    headless = true;
                    script = Some(args.next().ok_or("--script requires a path")?.into());
                    continue;
                }
                "--replay-script" => {
                    replay = Some(
                        args.next()
                            .ok_or("--replay-script requires a trace file")?
                            .into(),
                    );
                    continue;
                }
                "--replay" => {
                    let trace = args.next().ok_or("--replay requires a trace file")?.into();
                    if command.is_some() || replay.is_some() || headless {
                        return Err("--replay conflicts with other launch modes".into());
                    }
                    command = Some(Command::ReplayFull { trace });
                    continue;
                }
                "--export-metadata" => {
                    let trace = args
                        .next()
                        .ok_or("--export-metadata requires a trace file")?
                        .into();
                    if command.is_some() || replay.is_some() || headless {
                        return Err("--export-metadata conflicts with other launch modes".into());
                    }
                    command = Some(Command::ExportMetadata { trace });
                    continue;
                }
                "--bench" => {
                    let scenario = args
                        .next()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "all".into());
                    command = Some(Command::Bench { scenario });
                    continue;
                }
                "config" if operand.is_none() => {
                    command = Some(Command::Config);
                    continue;
                }
                "update" if operand.is_none() => {
                    let check_only = args.peek().is_some_and(|arg| arg == "--check");
                    if check_only {
                        args.next();
                    }
                    command = Some(Command::Update { check_only });
                    continue;
                }
                _ => {}
            }
            if let Some(path) = text.strip_prefix("--log=") {
                if path.is_empty() {
                    return Err("--log= requires a nonempty path".into());
                }
                trace_path = Some(path.into());
                continue;
            }
            if text.starts_with('-') {
                return Err(format!("unknown option: {text}"));
            }
        }
        let file = FileLocation::parse(argument, positional).map_err(|error| error.to_string())?;
        if operand.replace(file).is_some() {
            return Err("only one file or directory operand is supported".into());
        }
    }
    let operand = match (operand, initial_line) {
        (Some(file), line) => Some(file.with_line(line).map_err(|error| error.to_string())?),
        (None, Some(_)) => return Err(location::LocationError::MissingPath.to_string()),
        (None, None) => None,
    };
    if content == ContentPolicy::Full && trace_path.is_none() {
        return Err("--log-content requires --log or STROP_LOG".into());
    }
    // Replay/export consume exactly one trace file and never record one.
    if matches!(
        &command,
        Some(Command::ReplayFull { .. } | Command::ExportMetadata { .. })
    ) && (trace_path.is_some() || content == ContentPolicy::Full || operand.is_some())
    {
        return Err(
            "--replay/--export-metadata take exactly one trace and record no log; unset STROP_LOG"
                .into(),
        );
    }
    let command = match (command, replay, headless) {
        (Some(command), None, false) => command,
        (None, Some(trace), false) if operand.is_none() => Command::Replay { trace },
        (None, None, true) => Command::Headless {
            script: script.ok_or("--headless requires a script path")?,
            path: operand,
        },
        (None, None, false) => Command::Edit {
            path: operand,
            readonly,
        },
        _ => return Err("conflicting launch modes".into()),
    };
    Ok(Options {
        command,
        trace_path,
        content,
    })
}
