//! Ex addresses (`:help cmdline-ranges`): `N . $ % 'a /pat/ ?pat?` with
//! `+N` / `-N` offsets, joined by `,` or `;`. Line numbers here are Vim's:
//! one-based, with 0 meaning "above the first line" for the commands that
//! take it (`:m0`, `:0pu`).

use strop_grammar::CompiledQuery;

use crate::editor::Editor;

/// The addresses typed before a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineRange {
    pub first: usize,
    pub last: usize,
}

impl LineRange {
    /// Zero-based inclusive lines; address 0 reads as the first line.
    pub(crate) fn lines(self) -> (usize, usize) {
        (self.first.saturating_sub(1), self.last.saturating_sub(1))
    }
}

/// A command-line pattern, compiled by the compiler `/` uses: one case
/// policy for both, `\c` / `\C` override it per pattern. `case` is
/// `:s`'s `i` (`Some(true)`) or `I` flag, stated as the compiler's own
/// leading directive so a `\c` in the pattern still wins.
pub(crate) fn compile_pattern(pattern: &str, case: Option<bool>) -> Result<CompiledQuery, String> {
    let (magic, rest) = match pattern.get(..2) {
        Some(prefix @ ("\\v" | "\\m")) => (prefix, &pattern[2..]),
        _ => ("", pattern),
    };
    let source = match case {
        Some(_) if rest.starts_with("case:") => pattern.to_owned(),
        Some(true) => format!("{magic}case:insensitive{rest}"),
        Some(false) => format!("{magic}case:sensitive{rest}"),
        None => pattern.to_owned(),
    };
    CompiledQuery::compile(&source, false).map_err(|error| error.to_string())
}

/// The text up to an unescaped `delimiter`, and what follows it. An
/// unterminated pattern runs to the end of the line.
pub(crate) fn split_pattern(text: &str, delimiter: char) -> (&str, &str, bool) {
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if c == delimiter && !escaped {
            return (&text[..index], &text[index + c.len_utf8()..], true);
        }
        escaped = c == '\\' && !escaped;
    }
    (text, "", false)
}

/// `\/` inside a `/pat/` address is a plain slash to the pattern.
pub(crate) fn unescape_delimiter(raw: &str, delimiter: char) -> String {
    raw.replace(&format!("\\{delimiter}"), &delimiter.to_string())
}

fn leading_number(text: &str) -> Option<(usize, &str)> {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    // A number too large to hold is past every buffer's last line.
    Some((
        text[..digits].parse().unwrap_or(usize::MAX),
        &text[digits..],
    ))
}

/// The number of lines an address may name (a trailing newline's
/// phantom line is not one).
pub(crate) fn ex_line_count(editor: &Editor) -> usize {
    editor.buf().last_content_line() + 1
}

/// The first line after (or before) `from` that `pattern` matches,
/// wrapping around the buffer as `/` does. Lines are one-based.
fn address_search(
    editor: &Editor,
    pattern: &str,
    from: usize,
    backward: bool,
) -> Result<usize, String> {
    let source = if pattern.is_empty() {
        editor
            .last_search
            .as_ref()
            .map(|search| search.query.source().to_owned())
            .ok_or("no previous search pattern")?
    } else {
        pattern.to_owned()
    };
    let query = compile_pattern(&source, None)?;
    let buf = editor.buf();
    let count = ex_line_count(editor);
    // Forward: the nearest line below, else (wrapping) the first match.
    // Backward: the nearest line above, else the last match.
    let (mut first, mut last, mut above, mut below) = (None, None, None, None);
    strop_grammar::search_visit(buf, &query, |hit| {
        let line = buf.line_of(hit.start.get()) + 1;
        if line > count {
            return std::ops::ControlFlow::Break(());
        }
        first = first.or(Some(line));
        last = Some(line);
        if line < from {
            above = Some(line);
        }
        if line > from {
            below = below.or(Some(line));
            if !backward {
                return std::ops::ControlFlow::Break(());
            }
        }
        std::ops::ControlFlow::Continue(())
    })
    .map_err(|error| error.to_string())?;
    let found = if backward {
        above.or(last)
    } else {
        below.or(first)
    };
    found.ok_or_else(|| format!("pattern not found: {source}"))
}

/// One address at the start of `text`: its line and the rest. `None`
/// when no address starts here.
fn parse_address<'a>(
    editor: &Editor,
    text: &'a str,
    cursor: usize,
) -> Result<Option<(usize, &'a str)>, String> {
    let mut chars = text.chars();
    let (base, mut rest) = match chars.next() {
        Some('.') => (Some(cursor), chars.as_str()),
        Some('$') => (Some(ex_line_count(editor)), chars.as_str()),
        Some('\'') => {
            let name = chars.next().ok_or("mark not set")?;
            let offset = editor.mark_offset(name).ok_or("mark not set")?;
            (Some(editor.buf().line_of(offset) + 1), chars.as_str())
        }
        Some(delimiter @ ('/' | '?')) => {
            let (raw, rest, _) = split_pattern(chars.as_str(), delimiter);
            let pattern = unescape_delimiter(raw, delimiter);
            let line = address_search(editor, &pattern, cursor, delimiter == '?')?;
            (Some(line), rest)
        }
        Some(c) if c.is_ascii_digit() => {
            let (number, rest) = leading_number(text).ok_or("invalid range")?;
            (Some(number), rest)
        }
        _ => (None, text),
    };
    // Offsets: `+N`, `-N`, a bare sign (one line), or a bare number
    // after an address (`.5` is `.+5`).
    let mut line = base.unwrap_or(cursor) as i128;
    let mut any = base.is_some();
    loop {
        let trimmed = rest.trim_start_matches(' ');
        let (sign, after) = match trimmed.chars().next() {
            Some('+') => (1, &trimmed[1..]),
            Some('-') => (-1, &trimmed[1..]),
            Some(c) if c.is_ascii_digit() && any => (1, trimmed),
            _ => break,
        };
        let (amount, after) = leading_number(after).unwrap_or((1, after));
        line += sign * amount as i128;
        rest = after;
        any = true;
    }
    if !any {
        return Ok(None);
    }
    if line < 0 {
        return Err("invalid range".into());
    }
    Ok(Some((usize::try_from(line).unwrap_or(usize::MAX), rest)))
}

/// The range before a command, and the command text after it. `head`
/// is the cursor the addresses are relative to (the preview passes the
/// prompt's origin).
pub(crate) fn parse_ex_range<'a>(
    editor: &Editor,
    head: usize,
    text: &'a str,
) -> Result<(Option<LineRange>, &'a str), String> {
    let text = text.trim_start_matches([' ', ':']);
    if let Some(rest) = text.strip_prefix('%') {
        return Ok((
            Some(LineRange {
                first: 1,
                last: ex_line_count(editor),
            }),
            rest,
        ));
    }
    let mut cursor = editor.buf().line_of(head) + 1;
    let mut rest = text;
    let mut addresses: Vec<usize> = Vec::new();
    loop {
        let address = parse_address(editor, rest, cursor)?;
        let after = address
            .map_or(rest, |(_, after)| after)
            .trim_start_matches(' ');
        let separator = after.chars().next().filter(|c| matches!(c, ',' | ';'));
        if address.is_none() && separator.is_none() && addresses.is_empty() {
            return Ok((None, text));
        }
        // A missing address beside a separator is the cursor line.
        let line = address.map_or(cursor, |(line, _)| line);
        addresses.push(line);
        rest = after;
        match separator {
            Some(separator) => {
                if separator == ';' {
                    cursor = line.clamp(1, ex_line_count(editor));
                }
                rest = &rest[1..];
            }
            None => break,
        }
    }
    let last = addresses[addresses.len() - 1];
    let first = if addresses.len() > 1 {
        addresses[addresses.len() - 2]
    } else {
        last
    };
    Ok((Some(LineRange { first, last }), rest))
}

/// Vim's two range errors: reversed, and past the last line.
pub(crate) fn check_ex_range(editor: &Editor, range: LineRange) -> Result<(), String> {
    if range.first > range.last {
        return Err("backwards range".into());
    }
    if range.last > ex_line_count(editor) {
        return Err("invalid range".into());
    }
    Ok(())
}
