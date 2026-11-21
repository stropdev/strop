//! Command-line parsing. File operands remain OsString/PathBuf end to end.
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use strop_trace::ContentPolicy;

mod location;
pub(crate) use location::FileLocation;

#[derive(Debug)]
pub enum Command {
    Edit {
        path: Option<FileLocation>,
        readonly: bool,
        /// `strop -`: the buffer is stdin (streamed until EOF).
        stdin: bool,
    },
    Headless {
        script: PathBuf,
        path: Option<FileLocation>,
        readonly: bool,
        /// `-`: the initial buffer is stdin, read whole.
        stdin: bool,
    },
    Replay {
        trace: PathBuf,
    },
    /// `replay TRACE`: full forensic replay of a complete full-content
    /// capture (R11). Distinct from `replay --script` input extraction.
    ReplayFull {
        trace: PathBuf,
    },
    /// `export TRACE`: positive-projection privacy export;
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
    pub terminal_capture: bool,
    /// `--theme NAME`: session-start palette override over the
    /// config file's `theme` knob.
    pub theme: Option<strop_core::theme::ThemeId>,
}

pub fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut args = args.into_iter().peekable();
    let mut trace_path = std::env::var_os("STROP_LOG")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let mut content = ContentPolicy::Metadata;
    let mut terminal_capture = false;
    let mut readonly = false;
    let mut initial_line = None;
    let mut theme = None;
    let mut headless = false;
    let mut script = None;
    let mut replay = None;
    let mut operand = None;
    let mut command = None;
    let mut positional = false;
    let mut stdin = false;
    while let Some(argument) = args.next() {
        if !positional {
            if let Some(path) = argument.as_bytes().strip_prefix(b"--log=") {
                if path.is_empty() {
                    return Err("--log= requires a nonempty path".into());
                }
                trace_path = Some(OsStr::from_bytes(path).into());
                continue;
            }
            let text = argument.to_string_lossy();
            if let Some(line) = text.strip_prefix('+') {
                if initial_line.is_some() {
                    return Err(location::LocationError::DuplicateLine.to_string());
                }
                initial_line = Some(location::parse_line(line).map_err(|error| error.to_string())?);
                continue;
            }
            match text.as_ref() {
                "--theme" => {
                    let value = args
                        .next()
                        .ok_or("--theme requires a palette name; see --help")?;
                    let value = value
                        .to_str()
                        .ok_or("--theme requires a UTF-8 palette name")?;
                    theme =
                        Some(strop_core::theme::ThemeId::from_name(value).ok_or_else(|| {
                            format!(
                                "unknown theme {value:?}; choose {}",
                                strop_core::theme::ThemeId::choices()
                            )
                        })?);
                    continue;
                }
                "--" => {
                    positional = true;
                    continue;
                }
                "-" => {
                    stdin = true;
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
                "--readonly" | "-R" => {
                    readonly = true;
                    continue;
                }
                "--log" => {
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
                "--log-terminal-content" => {
                    content = ContentPolicy::Full;
                    terminal_capture = true;
                    continue;
                }
                "config"
                    if command.is_none() && !headless && replay.is_none() && operand.is_none() =>
                {
                    command = Some(Command::Config);
                    continue;
                }
                "update"
                    if command.is_none() && !headless && replay.is_none() && operand.is_none() =>
                {
                    let check_only = args.peek().is_some_and(|arg| arg == "--check");
                    if check_only {
                        args.next();
                    }
                    command = Some(Command::Update { check_only });
                    continue;
                }
                "remote"
                    if command.is_none() && !headless && replay.is_none() && operand.is_none() =>
                {
                    let url = args.next().ok_or(
                        "remote expects an ssh:// URL: ssh://[user@]host[:port]/absolute/path[:LINE]",
                    )?;
                    let file =
                        FileLocation::parse(url, positional).map_err(|error| error.to_string())?;
                    match file.path {
                        crate::files::FileTarget::Remote(_) => {
                            operand = Some(file);
                        }
                        _ => {
                            return Err(
                                "remote expects an ssh:// URL: ssh://[user@]host[:port]/absolute/path[:LINE]".into(),
                            )
                        }
                    }
                    continue;
                }
                "headless" if command.is_none() && !headless && replay.is_none() => {
                    headless = true;
                    if args
                        .peek()
                        .is_some_and(|value| !value.to_string_lossy().starts_with('-'))
                    {
                        script = args.next().map(PathBuf::from);
                    }
                    continue;
                }
                "replay" if command.is_none() && !headless && replay.is_none() => {
                    // `strop replay [--script] TRACE`: the default is the
                    // full forensic replay; --script extracts the headless
                    // reproduction script instead.
                    let mut extract_script = false;
                    let mut trace = None;
                    for argument in args.by_ref() {
                        let text = argument.to_string_lossy();
                        if text == "--script" && trace.is_none() {
                            extract_script = true;
                            continue;
                        }
                        if text.starts_with('-') {
                            return Err(format!("unknown replay option: {text}"));
                        }
                        if trace.replace(argument.into()).is_some() {
                            return Err("replay takes exactly one trace file".into());
                        }
                    }
                    let trace = trace.ok_or("replay requires a trace file")?;
                    if extract_script {
                        replay = Some(trace);
                    } else {
                        command = Some(Command::ReplayFull { trace });
                    }
                    continue;
                }
                "export" if command.is_none() && !headless && replay.is_none() => {
                    let trace = args.next().ok_or("export requires a trace file")?.into();
                    command = Some(Command::ExportMetadata { trace });
                    continue;
                }
                "compat" if command.is_none() && !headless && replay.is_none() => {
                    command = Some(Command::Compat);
                    continue;
                }
                "help" if command.is_none() && !headless && replay.is_none() => {
                    command = Some(Command::Help);
                    continue;
                }
                "bench" if command.is_none() && !headless && replay.is_none() => {
                    let scenario = args
                        .peek()
                        .is_some_and(|value| !value.to_string_lossy().starts_with('-'))
                        .then(|| {
                            args.next()
                                .map(|value| value.to_string_lossy().into_owned())
                        })
                        .flatten()
                        .unwrap_or_else(|| "all".into());
                    command = Some(Command::Bench { scenario });
                    continue;
                }
                _ => {}
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
    if stdin && operand.is_some() {
        return Err("`-` reads the buffer from stdin; give no file operand with it".into());
    }
    let operand = match (operand, initial_line) {
        (Some(file), line) => Some(file.with_line(line).map_err(|error| error.to_string())?),
        (None, Some(_)) => return Err(location::LocationError::MissingPath.to_string()),
        (None, None) => None,
    };
    if content == ContentPolicy::Full && trace_path.is_none() {
        return Err(if terminal_capture {
            "--log-terminal-content requires --log or STROP_LOG"
        } else {
            "--log-content requires --log or STROP_LOG"
        }
        .into());
    }
    if (command.is_some() || replay.is_some()) && (operand.is_some() || stdin) {
        return Err("this command does not take an editor file or stdin operand".into());
    }
    // Replay/export consume exactly one trace file and never record one.
    if matches!(
        &command,
        Some(Command::ReplayFull { .. } | Command::ExportMetadata { .. })
    ) && (trace_path.is_some() || content == ContentPolicy::Full)
    {
        return Err(
            "replay/export take exactly one trace and record no log; unset STROP_LOG".into(),
        );
    }
    let command = match (command, replay, headless) {
        (Some(command), None, false) => command,
        (None, Some(trace), false) => Command::Replay { trace },
        (None, None, true) => Command::Headless {
            script: script.ok_or("headless requires a script path")?,
            path: operand,
            readonly,
            stdin,
        },
        (None, None, false) => Command::Edit {
            path: operand,
            readonly,
            stdin,
        },
        _ => return Err("conflicting launch modes".into()),
    };
    Ok(Options {
        command,
        trace_path,
        content,
        terminal_capture,
        theme,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        super::parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn a_dash_operand_reads_stdin() {
        assert!(matches!(
            parse(&["-"]).unwrap().command,
            Command::Edit {
                stdin: true,
                path: None,
                ..
            }
        ));
        assert!(matches!(
            parse(&["headless", "s.keys", "-"]).unwrap().command,
            Command::Headless { stdin: true, .. }
        ));
        assert!(parse(&["-", "file.txt"]).is_err());
        // after `--`, a file literally named `-`
        assert!(matches!(
            parse(&["--", "-"]).unwrap().command,
            Command::Edit {
                stdin: false,
                path: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn subcommands_map_to_their_commands() {
        assert!(matches!(parse(&["help"]).unwrap().command, Command::Help));
        assert!(matches!(
            parse(&["compat"]).unwrap().command,
            Command::Compat
        ));
        assert!(matches!(
            parse(&["config"]).unwrap().command,
            Command::Config
        ));
        assert!(matches!(
            parse(&["update", "--check"]).unwrap().command,
            Command::Update { check_only: true }
        ));
        assert!(matches!(
            parse(&["bench"]).unwrap().command,
            Command::Bench { scenario } if scenario == "all"
        ));
        assert!(matches!(
            parse(&["bench", "startup"]).unwrap().command,
            Command::Bench { scenario } if scenario == "startup"
        ));
        assert!(matches!(
            parse(&["export", "t.jsonl"]).unwrap().command,
            Command::ExportMetadata { trace } if trace.as_os_str() == "t.jsonl"
        ));
        assert!(matches!(
            parse(&["replay", "t.jsonl"]).unwrap().command,
            Command::ReplayFull { trace } if trace.as_os_str() == "t.jsonl"
        ));
        assert!(matches!(
            parse(&["replay", "--script", "t.jsonl"]).unwrap().command,
            Command::Replay { trace } if trace.as_os_str() == "t.jsonl"
        ));
        let headless = parse(&["headless", "drive.txt", "a.rs"]).unwrap();
        assert!(matches!(
            headless.command,
            Command::Headless {
                ref path,
                ..
            } if path.is_some()
        ));
    }

    #[test]
    fn remote_subcommand_requires_an_ssh_url() {
        for form in [
            vec!["remote", "ssh://tnawara@123.space:4343/etc/conf"],
            vec!["remote", "ssh://tnawara@123.space:4343/etc/conf:99"],
        ] {
            let options = parse(&form).unwrap();
            assert!(
                matches!(
                    options.command,
                    Command::Edit {
                        ref path,
                        ..
                    } if matches!(
                        path.as_ref().map(|location| &location.path),
                        Some(crate::files::FileTarget::Remote(_))
                    )
                ),
                "{form:?}"
            );
        }
        for form in [vec!["remote"], vec!["remote", "/tmp/x"]] {
            assert!(
                parse(&form)
                    .unwrap_err()
                    .contains("remote expects an ssh:// URL"),
                "{form:?}"
            );
        }
    }

    #[test]
    fn command_flags_are_refused() {
        // Commands are words; there is no flag spelling of any of them.
        for retired in [
            "--headless",
            "--script",
            "--replay",
            "--remote",
            "--bench",
            "--dump-compat",
            "--replay-script",
            "--export-metadata",
        ] {
            assert!(
                parse(&[retired, "t.jsonl"])
                    .unwrap_err()
                    .contains("unknown option"),
                "{retired}"
            );
        }
        // The default edit form with no subcommand is untouched.
        assert!(matches!(
            parse(&["-R", "a.rs:12"]).unwrap().command,
            Command::Edit { readonly: true, .. }
        ));
    }

    #[test]
    fn non_editing_commands_do_not_discard_an_editor_operand() {
        for args in [
            &["file.txt", "compat"][..],
            &["compat", "file.txt"],
            &["config", "file.txt"],
            &["file.txt", "bench", "startup"],
            &["help", "-"],
            &["--version", "--", "file.txt"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        let Command::Edit {
            path: Some(file), ..
        } = parse(&["--", "compat"]).unwrap().command
        else {
            panic!("expected to edit the literal file");
        };
        assert_eq!(file.path, crate::files::FileTarget::Local("compat".into()));
    }
}
