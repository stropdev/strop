//! `:s/pattern/replacement/flags count` and its repeats (`:s`, `:&`,
//! `:&&`, `:~`) into one [`SubstituteSpec`]. Preview and execution both
//! come through here.

use super::super::range::{compile_pattern, split_pattern, unescape_delimiter, LineRange};
use super::replace::expand_tilde;
use super::{Flags, LastSubstitute, SubstituteError, SubstituteSpec};
use crate::editor::ex::*;
use crate::editor::Editor;

/// Which member of the family was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// `:s`, `:substitute`
    Substitute,
    /// `:&` (and `:&&`, whose second `&` is the keep-flags flag)
    Again,
    /// `:~`: the last replacement with the last search pattern
    Tilde,
}

impl Form {
    pub(crate) fn of(name: &str) -> Option<Self> {
        match name {
            "substitute" => Some(Self::Substitute),
            "&" => Some(Self::Again),
            "~" => Some(Self::Tilde),
            _ => None,
        }
    }
}

fn valid_delimiter(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && !matches!(c, '\\' | '"' | '|')
}

/// `[&][cegiInr…] [count]` after the replacement (or alone, for a repeat).
fn parse_flags(text: &str, previous: Flags) -> Result<(Flags, Option<usize>), SubstituteError> {
    let mut flags = Flags::default();
    let mut rest = text;
    if let Some(after) = rest.strip_prefix('&') {
        flags = previous;
        rest = after;
    }
    while let Some(c) = rest.chars().next() {
        match c {
            'g' => flags.global = !flags.global,
            'c' => flags.confirm = !flags.confirm,
            'n' => flags.count_only = true,
            'e' => flags.quiet = true,
            'i' => flags.case = Some(true),
            'I' => flags.case = Some(false),
            // Print flags and `r` change nothing strop shows.
            'p' | '#' | 'l' | 'r' => {}
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok((flags, None));
    }
    match rest.parse::<usize>() {
        Ok(0) => Err(SubstituteError::Invalid("positive count required".into())),
        Ok(count) => Ok((flags, Some(count))),
        Err(_) => Err(SubstituteError::Invalid(format!(
            "trailing characters: {rest}"
        ))),
    }
}

/// The pattern an empty `//` stands for: the last search.
fn last_pattern(editor: &Editor) -> Result<String, SubstituteError> {
    editor
        .last_search
        .as_ref()
        .map(|search| search.query.source().to_owned())
        .ok_or_else(|| SubstituteError::Invalid("no previous regular expression".into()))
}

/// One member of the `:s` family, as typed after its range. `default`
/// is the cursor line (zero-based) a rangeless command applies to.
pub(crate) fn parse_substitute(
    editor: &Editor,
    form: Form,
    range: Option<LineRange>,
    default: usize,
    arg: &str,
) -> Result<SubstituteSpec, SubstituteError> {
    let previous = editor.pending.ex.last_substitute.as_ref();
    let previous_flags = previous.map(|last| last.flags).unwrap_or_default();
    let delimiter = arg.chars().next().filter(|c| valid_delimiter(*c));
    let (pattern, replacement, flags, count) = match (form, delimiter) {
        (Form::Substitute, Some(delimiter)) if delimiter != '&' => {
            let body = &arg[delimiter.len_utf8()..];
            let (raw_pattern, after_pattern, terminated) = split_pattern(body, delimiter);
            if !terminated {
                return Err(SubstituteError::Incomplete(
                    ":s needs a delimiter after the pattern".into(),
                ));
            }
            let (raw_replacement, flags, _) = split_pattern(after_pattern, delimiter);
            let trailing = raw_replacement.len() - raw_replacement.trim_end_matches('\\').len();
            if trailing % 2 == 1 {
                return Err(SubstituteError::Incomplete(
                    "replacement ends with an unfinished escape".into(),
                ));
            }
            let pattern = if raw_pattern.is_empty() {
                last_pattern(editor)?
            } else {
                unescape_delimiter(raw_pattern, delimiter)
            };
            let replacement = expand_tilde(
                &unescape_delimiter(raw_replacement, delimiter),
                previous.map(|last| last.replacement.as_str()),
            );
            let (flags, count) = parse_flags(flags, previous_flags)?;
            (pattern, replacement, flags, count)
        }
        (Form::Substitute, None) if arg.is_empty() && previous.is_none() => {
            return Err(SubstituteError::Incomplete(
                ":s needs a delimiter, pattern and replacement".into(),
            ));
        }
        (Form::Substitute, None)
            if arg
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() && !"cegiInp#lr".contains(c)) =>
        {
            return Err(SubstituteError::Invalid(
                "invalid substitute delimiter".into(),
            ));
        }
        _ => {
            let Some(last) = previous else {
                return Err(SubstituteError::Invalid(
                    "no previous substitute regular expression".into(),
                ));
            };
            let pattern = if form == Form::Tilde {
                last_pattern(editor)?
            } else {
                last.pattern.clone()
            };
            let (flags, count) = parse_flags(arg.trim_start(), previous_flags)?;
            (pattern, last.replacement.clone(), flags, count)
        }
    };
    if let Some(range) = range {
        check_ex_range(editor, range).map_err(SubstituteError::Invalid)?;
    }
    let (mut first, mut last) = range.map_or((default, default), LineRange::lines);
    if let Some(count) = count {
        // Vim: a count starts at the range's last line.
        first = last;
        last = (first + count - 1).min(editor.buf().last_content_line());
    }
    let query = compile_pattern(&pattern, flags.case).map_err(SubstituteError::Invalid)?;
    Ok(SubstituteSpec {
        range: (first, last),
        query,
        pattern,
        replacement,
        flags,
        within: editor.collection_body_spans(editor.current()),
    })
}

/// A whole command line, when it is a member of the `:s` family
/// (`Ok(None)` otherwise). `head` is the cursor the range reads.
pub(crate) fn parse_substitute_line(
    editor: &Editor,
    body: &str,
    head: usize,
) -> Result<Option<SubstituteSpec>, SubstituteError> {
    let (range, rest) = parse_ex_range(editor, head, body).map_err(SubstituteError::Invalid)?;
    let Some((command, _, arg)) = super::super::split_command(rest) else {
        return Ok(None);
    };
    let Some(form) = Form::of(command.name) else {
        return Ok(None);
    };
    parse_substitute(editor, form, range, editor.buf().line_of(head), arg).map(Some)
}

pub(crate) fn remember_substitute(editor: &mut Editor, spec: &SubstituteSpec) {
    editor.pending.ex.last_substitute = Some(LastSubstitute {
        pattern: spec.pattern.clone(),
        replacement: spec.replacement.clone(),
        flags: spec.flags,
    });
}
