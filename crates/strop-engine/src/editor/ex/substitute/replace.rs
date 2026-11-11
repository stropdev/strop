//! The `:s` replacement string (`:help sub-replace-special`): `&` and
//! `\0`–`\9` insert the match and its groups, `\r` breaks the line, `\n`
//! is a NUL, `\t` a tab, and `\u \l \U \L \E` change the case of what
//! follows. Parsed once per substitution; expanded once per match.

use strop_core::Buffer;
use strop_grammar::SearchMatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    /// `\u` / `\l`: the next character only.
    UpperNext,
    LowerNext,
    /// `\U` / `\L`: everything up to `\E`.
    UpperRun,
    LowerRun,
    /// `\E` / `\e`.
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// 0 is the whole match.
    Group(usize),
    Case(Case),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
    pieces: Vec<Piece>,
}

impl Template {
    /// `source` is the replacement as typed, with `~` already expanded
    /// and the command's own delimiter still escaped.
    pub(crate) fn parse(source: &str) -> Self {
        let mut pieces = Vec::new();
        let mut text = String::new();
        let flush = |text: &mut String, pieces: &mut Vec<Piece>, piece: Piece| {
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(text)));
            }
            pieces.push(piece);
        };
        let mut chars = source.chars();
        while let Some(c) = chars.next() {
            match c {
                '&' => flush(&mut text, &mut pieces, Piece::Group(0)),
                '\r' => text.push('\n'),
                '\\' => match chars.next() {
                    Some(digit @ '0'..='9') => flush(
                        &mut text,
                        &mut pieces,
                        Piece::Group(digit as usize - '0' as usize),
                    ),
                    Some('n') => text.push('\0'),
                    Some('r') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some('u') => flush(&mut text, &mut pieces, Piece::Case(Case::UpperNext)),
                    Some('l') => flush(&mut text, &mut pieces, Piece::Case(Case::LowerNext)),
                    Some('U') => flush(&mut text, &mut pieces, Piece::Case(Case::UpperRun)),
                    Some('L') => flush(&mut text, &mut pieces, Piece::Case(Case::LowerRun)),
                    Some('e' | 'E') => flush(&mut text, &mut pieces, Piece::Case(Case::End)),
                    // `\&`, `\~`, `\\`, the escaped delimiter: itself.
                    Some(other) => text.push(other),
                    None => text.push('\\'),
                },
                other => text.push(other),
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Self { pieces }
    }

    /// The text one match becomes. `groups[n - 1]` is `\n`.
    pub(crate) fn expand(
        &self,
        buf: &Buffer,
        hit: SearchMatch,
        groups: &[Option<SearchMatch>],
    ) -> String {
        let mut output = String::new();
        let mut next: Option<Case> = None;
        let mut run: Option<Case> = None;
        let push = |output: &mut String, text: &str, next: &mut Option<Case>, run: Option<Case>| {
            for c in text.chars() {
                match next.take().or(run) {
                    Some(Case::UpperNext | Case::UpperRun) => output.extend(c.to_uppercase()),
                    Some(Case::LowerNext | Case::LowerRun) => output.extend(c.to_lowercase()),
                    Some(Case::End) | None => output.push(c),
                }
            }
        };
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => push(&mut output, text, &mut next, run),
                Piece::Group(index) => {
                    let group = if *index == 0 {
                        Some(hit)
                    } else {
                        groups.get(index - 1).copied().flatten()
                    };
                    if let Some(group) = group {
                        let text = buf
                            .text()
                            .byte_slice(group.start.get()..group.end.get())
                            .to_string();
                        push(&mut output, &text, &mut next, run);
                    }
                }
                Piece::Case(case @ (Case::UpperNext | Case::LowerNext)) => next = Some(*case),
                Piece::Case(case @ (Case::UpperRun | Case::LowerRun)) => run = Some(*case),
                Piece::Case(Case::End) => {
                    next = None;
                    run = None;
                }
            }
        }
        output
    }
}

/// `~` in a replacement is the previous replacement string; `\~` is a
/// tilde. Done before the template is parsed, as Vim does, so the result
/// is what `~` means next time.
pub(crate) fn expand_tilde(source: &str, previous: Option<&str>) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => output.push_str(previous.unwrap_or("")),
            '\\' => {
                output.push('\\');
                if let Some(next) = chars.next() {
                    output.push(next);
                }
            }
            other => output.push(other),
        }
    }
    output
}
