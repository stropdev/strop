//! Command-line parsing. File operands remain OsString/PathBuf end to end.
use crate::editor::remote::{view, RemoteView};
use std::ffi::OsString;
use std::path::PathBuf;
use strop_remote::{ReadLimit, ReadSelection};
use strop_trace::ContentPolicy;

mod location;
pub(crate) use location::FileLocation;

#[derive(Debug)]
pub enum Command {
    Edit {
        path: Option<FileLocation>,
        readonly: bool,
        remote_view: RemoteView,
    },
    Headless {
        script: PathBuf,
        path: Option<FileLocation>,
        remote_view: RemoteView,
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
    /// `--ui-stdio`: the bounded UI protocol backend on stdin/stdout
    /// (0056 AR09); executed early like replay/export — stdout carries
    /// frames only.
    UiStdio,
    Help,
    /// Built-in theme attribution, also retained in source packages.
    ThemeLicenses,
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
    let mut remote_selection = None;
    let mut follow = false;
    let mut theme = None;
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
                "--tail" | "--range" => {
                    if remote_selection.is_some() {
                        return Err("choose only one --tail or --range".into());
                    }
                    let value = args
                        .next()
                        .ok_or("remote view option requires byte arguments")?;
                    let value = value
                        .to_str()
                        .ok_or("byte arguments must be decimal text")?;
                    remote_selection = Some(if text == "--tail" {
                        ReadSelection::Tail(view::limit(value).map_err(|error| error.to_string())?)
                    } else {
                        view::range(value).map_err(|error| error.to_string())?
                    });
                    continue;
                }
                "--follow" => {
                    follow = true;
                    continue;
                }
                "--theme" => {
                    let value = args
                        .next()
                        .ok_or("--theme requires a palette name; see --help")?;
                    let value = value
                        .to_str()
                        .ok_or("--theme requires a UTF-8 palette name")?;
                    theme =
                        Some(strop_core::theme::ThemeId::from_name(value).ok_or_else(|| {
                            let choices = strop_core::theme::ThemeId::ALL
                                .iter()
                                .map(|id| id.name())
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("unknown theme {value:?}; choose {choices}")
                        })?);
                    continue;
                }
                "--" => {
                    positional = true;
                    continue;
                }
                "--help" | "-h" => {
                    command = Some(Command::Help);
                    continue;
                }
                "--licenses" => {
                    command = Some(Command::ThemeLicenses);
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
                "--log-terminal-content" => {
                    content = ContentPolicy::Full;
                    terminal_capture = true;
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
                "--ui-stdio" => {
                    if command.is_some() || replay.is_some() || headless {
                        return Err("--ui-stdio conflicts with other launch modes".into());
                    }
                    command = Some(Command::UiStdio);
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
                // ---- subcommands (flags above remain the synonyms) ----
                "edit" if command.is_none() && !headless && operand.is_none() => {
                    continue;
                }
                "remote" | "--remote"
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
                "ui-stdio" if command.is_none() && !headless && replay.is_none() => {
                    command = Some(Command::UiStdio);
                    continue;
                }
                "licenses" if command.is_none() && !headless && replay.is_none() => {
                    command = Some(Command::ThemeLicenses);
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
    let remote_view = match (follow, remote_selection) {
        (true, None) => RemoteView::Follow(ReadLimit::DEFAULT_TAIL),
        (true, Some(ReadSelection::Tail(limit))) => RemoteView::Follow(limit),
        (true, Some(_)) => return Err("--follow cannot use a fixed byte range".into()),
        (false, selection) => RemoteView::Snapshot(selection.unwrap_or(ReadSelection::Full)),
    };
    if command.is_none()
        && remote_view != RemoteView::default()
        && !operand
            .as_ref()
            .is_some_and(|file| matches!(file.path, crate::files::FileTarget::Remote(_)))
    {
        return Err("remote view options require an ssh:// operand".into());
    }
    if content == ContentPolicy::Full && trace_path.is_none() {
        return Err(if terminal_capture {
            "--log-terminal-content requires --log or STROP_LOG"
        } else {
            "--log-content requires --log or STROP_LOG"
        }
        .into());
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
    // The protocol backend runs early like replay/export: no file
    // operand (clients open through admitted actions), no trace flags
    // (startup returns before trace setup).
    if matches!(&command, Some(Command::UiStdio))
        && (trace_path.is_some() || content == ContentPolicy::Full || operand.is_some())
    {
        return Err("--ui-stdio takes no operand and records no log; unset STROP_LOG".into());
    }
    let command = match (command, replay, headless) {
        (Some(command), None, false) => command,
        (None, Some(trace), false) if operand.is_none() => Command::Replay { trace },
        (None, None, true) => Command::Headless {
            script: script.ok_or("--headless requires a script path")?,
            path: operand,
            remote_view,
        },
        (None, None, false) => Command::Edit {
            path: operand,
            readonly,
            remote_view,
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
    fn subcommands_map_to_their_commands() {
        assert!(matches!(parse(&["help"]).unwrap().command, Command::Help));
        assert!(matches!(
            parse(&["licenses"]).unwrap().command,
            Command::ThemeLicenses
        ));
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
            parse(&["ui-stdio"]).unwrap().command,
            Command::UiStdio
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
    fn remote_subcommand_and_flag_require_an_ssh_url() {
        for form in [
            vec!["remote", "ssh://tnawara@123.space:4343/etc/conf"],
            vec!["--remote", "ssh://tnawara@123.space:4343/etc/conf:99"],
            vec!["edit", "ssh://tnawara@123.space:4343/etc/conf"],
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
        for form in [
            vec!["remote"],
            vec!["remote", "/tmp/x"],
            vec!["--remote", "x.txt"],
        ] {
            assert!(
                parse(&form)
                    .unwrap_err()
                    .contains("remote expects an ssh:// URL"),
                "{form:?}"
            );
        }
    }

    #[test]
    fn legacy_flags_still_parse() {
        assert!(matches!(
            parse(&["--headless", "s.txt", "a.rs"]).unwrap().command,
            Command::Headless { .. }
        ));
        assert!(matches!(
            parse(&["--replay", "t.jsonl"]).unwrap().command,
            Command::ReplayFull { .. }
        ));
        assert!(matches!(
            parse(&["--replay-script", "t.jsonl"]).unwrap().command,
            Command::Replay { .. }
        ));
        assert!(matches!(
            parse(&["--ui-stdio"]).unwrap().command,
            Command::UiStdio
        ));
        assert!(matches!(
            parse(&["--licenses"]).unwrap().command,
            Command::ThemeLicenses
        ));
        // The default edit form with no subcommand is untouched.
        assert!(matches!(
            parse(&["-R", "a.rs:12"]).unwrap().command,
            Command::Edit { readonly: true, .. }
        ));
    }

    #[test]
    fn a_word_after_a_command_is_still_an_operand_slot() {
        // The first word chose the command; the trailing word takes the
        // one operand slot (the flag commands ignore it, like their
        // --licenses ancestors).
        assert!(matches!(
            parse(&["config", "x.txt"]).unwrap().command,
            Command::Config
        ));
        assert!(parse(&["config", "x.txt", "y.txt"]).is_err());
    }
}
