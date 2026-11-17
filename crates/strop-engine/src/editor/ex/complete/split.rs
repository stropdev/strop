//! Where the cursor is in a command line (plan 0110 §3.2): a range, a
//! command name, `!`, an argument — split by one pure pass, with the
//! same name rules dispatch uses (`command_word`). The cursor's part
//! decides what the menu lists.

use super::super::table::{Arg, ExCommand};

/// What the text before the cursor is completing; spans are byte
/// offsets into the body (the line without its sigil).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// Nothing completes here (a pattern, a register, shell text).
    None,
    /// A command name: `[start, end)` is what was typed of it.
    Name { start: usize, end: usize },
    /// A command's argument of `arg`'s kind: `[start, end)` is the part
    /// being completed.
    Arg {
        command: &'static ExCommand,
        arg: Arg,
        start: usize,
        end: usize,
    },
}

/// The command the line names (the outer one: `:g`, not what it runs).
pub(crate) fn command(body: &str) -> Option<&'static ExCommand> {
    let mut at = leading(body);
    at += range_len(&body[at..]);
    super::super::command_word(&body[at..]).map(|(command, _, _)| command)
}

/// The target at `cursor` in `body`.
pub(crate) fn target(body: &str, cursor: usize) -> Target {
    let cursor = cursor.min(body.len());
    command_at(body, 0, cursor)
}

/// A command line starting at `from` (after `:g/pat/`, `:cdo`, or the
/// line's own start).
fn command_at(body: &str, from: usize, cursor: usize) -> Target {
    let mut at = from + leading(&body[from..]);
    at += range_len(&body[at..]);
    if cursor < at {
        return Target::None;
    }
    let rest = &body[at..];
    let Some((command, _, length)) = super::super::command_word(rest) else {
        // an unknown word is still a name being typed
        let letters = word_len(rest);
        return if cursor <= at + letters {
            Target::Name {
                start: at,
                end: cursor,
            }
        } else {
            Target::None
        };
    };
    let name_end = at + length;
    if cursor <= name_end {
        return Target::Name {
            start: at,
            end: cursor,
        };
    }
    let mut after = name_end;
    if command.bang && body[after..].starts_with('!') {
        after += 1;
    }
    let arg_start = after + body[after..].len() - body[after..].trim_start().len();
    if cursor < arg_start || (arg_start == after && command.arg != Arg::Command) {
        // no blank before the argument yet (`:e!`, `:m-2`, `:s/`)
        return Target::None;
    }
    argument(body, command, arg_start, cursor)
}

fn argument(body: &str, command: &'static ExCommand, start: usize, cursor: usize) -> Target {
    let typed = &body[start..cursor];
    match command.arg {
        Arg::None | Arg::Register | Arg::Mark | Arg::Pattern | Arg::Shell | Arg::Text => {
            Target::None
        }
        // `:cdo {cmd}`; `:g/pat/{cmd}` once the pattern is closed
        Arg::Command => {
            if matches!(command.name(), "cdo" | "cfdo") {
                return command_at(body, start, cursor);
            }
            match pattern_end(typed) {
                Some(end) => command_at(body, start + end, cursor),
                None => Target::None,
            }
        }
        // a path is the whole argument (a name may hold a blank)
        Arg::File | Arg::Directory | Arg::HelpTopic => Target::Arg {
            command,
            arg: command.arg,
            start,
            end: cursor,
        },
        Arg::Buffer | Arg::Theme | Arg::Words(_) => {
            let word = typed.rfind(char::is_whitespace).map_or(0, |at| at + 1);
            Target::Arg {
                command,
                arg: command.arg,
                start: start + word,
                end: cursor,
            }
        }
    }
}

/// Leading colons and blanks, which Vim skips.
fn leading(text: &str) -> usize {
    text.len() - text.trim_start_matches([':', ' ', '\t']).len()
}

fn word_len(text: &str) -> usize {
    text.len()
        - text
            .trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '-')
            .len()
}

/// The length of the range at the start of `text`, read lexically:
/// numbers, `. $ %`, marks, `/pat/` and `?pat?`, offsets and separators.
pub(crate) fn range_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'0'..=b'9' | b'.' | b'$' | b'%' | b',' | b';' | b'+' | b'-' | b' ' | b'\t' => at += 1,
            b'\'' => {
                at += 1;
                if let Some(mark) = text[at..].chars().next() {
                    at += mark.len_utf8();
                }
            }
            b'/' | b'?' => match pattern_end(&text[at..]) {
                Some(end) => at += end,
                None => return text.len(),
            },
            b'\\' if matches!(bytes.get(at + 1), Some(b'/' | b'?' | b'&')) => at += 2,
            _ => break,
        }
    }
    at
}

/// `/pat/` (any delimiter): the length through the closing delimiter,
/// `None` while it is still open.
fn pattern_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, delimiter) = chars.next()?;
    if delimiter.is_alphanumeric() || matches!(delimiter, '"' | '|' | ' ') {
        return None;
    }
    let mut escaped = false;
    for (at, c) in chars {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter {
            return Some(at + c.len_utf8());
        }
    }
    None
}
