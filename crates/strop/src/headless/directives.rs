//! The supported script vocabulary drives parsing, help and unknown-command errors.
use std::io;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectiveKind {
    Buffer,
    Keys,
    Key,
    Paste,
    Resize,
    Frame,
    State,
    Settle,
    Wait,
    QuitIntent,
}

struct DirectiveSpec {
    kind: DirectiveKind,
    name: &'static str,
    arguments: &'static str,
    help: &'static str,
}

const DIRECTIVES: &[DirectiveSpec] = &[
    DirectiveSpec {
        kind: DirectiveKind::Buffer,
        name: "buffer",
        arguments: "JSON_STRING",
        help: "initial buffer text; must be the first directive",
    },
    DirectiveSpec {
        kind: DirectiveKind::Keys,
        name: "keys",
        arguments: "KEYS",
        help: "literal keys and tokens: <esc> <cr> <bs> <c-r> <space>",
    },
    DirectiveSpec {
        kind: DirectiveKind::Key,
        name: "key",
        arguments: "JSON_KEY",
        help: "one serialized key event, e.g. \"Esc\" or {\"Char\":\"x\"}",
    },
    DirectiveSpec {
        kind: DirectiveKind::Paste,
        name: "paste",
        arguments: "JSON_STRING",
        help: "one bracketed-paste event, e.g. \"first\\nsecond\"",
    },
    DirectiveSpec {
        kind: DirectiveKind::Resize,
        name: "resize",
        arguments: "COLS ROWS",
        help: "resize the terminal cell grid",
    },
    DirectiveSpec {
        kind: DirectiveKind::Frame,
        name: "frame",
        arguments: "",
        help: "print the current rendered cell grid",
    },
    DirectiveSpec {
        kind: DirectiveKind::State,
        name: "state",
        arguments: "",
        help: "print editor state as JSON",
    },
    DirectiveSpec {
        kind: DirectiveKind::Settle,
        name: "settle",
        arguments: "[MS]",
        help: "return when finite work drains; timeout fails (default 30000 ms)",
    },
    DirectiveSpec {
        kind: DirectiveKind::Wait,
        name: "wait",
        arguments: "MS",
        help: "deliberately wait this long while processing events",
    },
    DirectiveSpec {
        kind: DirectiveKind::QuitIntent,
        name: "quit-intent",
        arguments: "",
        help: "send the same quit request as Ctrl-C",
    },
];

pub(super) fn parse(line: &str) -> io::Result<(DirectiveKind, &str)> {
    let (name, arguments) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let Some(spec) = DIRECTIVES.iter().find(|spec| spec.name == name) else {
        let names = DIRECTIVES
            .iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown script directive {name:?}; expected {names}; see strop --help"),
        ));
    };
    if spec.arguments.is_empty() && !arguments.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} takes no arguments"),
        ));
    }
    Ok((spec.kind, arguments))
}

pub(super) fn duration(arguments: &str, default: Option<Duration>) -> io::Result<Duration> {
    let text = arguments.trim();
    if text.is_empty() {
        return default.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "wait requires milliseconds")
        });
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "milliseconds must be a nonnegative decimal integer",
        ));
    }
    let millis = text
        .parse::<u64>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    Ok(Duration::from_millis(millis))
}

pub(crate) fn print_help() {
    println!("\nHEADLESS SCRIPT (one directive per line; empty lines and # comments ignored):");
    for spec in DIRECTIVES {
        println!(
            "  {:<23} {}",
            format!("{} {}", spec.name, spec.arguments),
            spec.help
        );
    }
    println!("  JSON strings use double quotes and JSON escapes; keys text is not quoted.\n  Open: keys :e path<cr>    Insert: keys ihello<esc>\n  :trust uses the same explicit project consent and state root as the TUI.");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_key_spaces_survive_directive_parsing() {
        let (kind, arguments) = parse("keys   x ").unwrap();
        assert!(matches!(kind, DirectiveKind::Keys));
        assert_eq!(arguments, "  x ");
    }
    #[test]
    fn invalid_durations_are_not_silently_defaulted() {
        for value in ["-1", "+1", "1.5", "18446744073709551616"] {
            assert_eq!(
                duration(value, Some(Duration::ZERO)).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(duration("0", None).unwrap(), Duration::ZERO);
        assert!(duration("", None).is_err());
    }
}
