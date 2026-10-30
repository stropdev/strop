//! The supported script vocabulary drives parsing and unknown-command errors.
use std::io;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectiveKind {
    Buffer,
    Keys,
    Key,
    Input,
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
}

const DIRECTIVES: &[DirectiveSpec] = &[
    DirectiveSpec {
        kind: DirectiveKind::Buffer,
        name: "buffer",
        arguments: "JSON_STRING",
    },
    DirectiveSpec {
        kind: DirectiveKind::Keys,
        name: "keys",
        arguments: "KEYS",
    },
    DirectiveSpec {
        kind: DirectiveKind::Key,
        name: "key",
        arguments: "JSON_KEY",
    },
    DirectiveSpec {
        kind: DirectiveKind::Input,
        name: "input",
        arguments: "JSON_INPUT",
    },
    DirectiveSpec {
        kind: DirectiveKind::Paste,
        name: "paste",
        arguments: "JSON_STRING",
    },
    DirectiveSpec {
        kind: DirectiveKind::Resize,
        name: "resize",
        arguments: "COLS ROWS",
    },
    DirectiveSpec {
        kind: DirectiveKind::Frame,
        name: "frame",
        arguments: "",
    },
    DirectiveSpec {
        kind: DirectiveKind::State,
        name: "state",
        arguments: "",
    },
    DirectiveSpec {
        kind: DirectiveKind::Settle,
        name: "settle",
        arguments: "[MS]",
    },
    DirectiveSpec {
        kind: DirectiveKind::Wait,
        name: "wait",
        arguments: "MS",
    },
    DirectiveSpec {
        kind: DirectiveKind::QuitIntent,
        name: "quit-intent",
        arguments: "",
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
            format!("unknown script directive {name:?}; expected {names}; see docs/terminal-and-scripting.md"),
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
