//! LSP snippet text (`insertTextFormat: 2`) parsing: tabstops, placeholder
//! defaults, choices and the final stop, with `\$`/`\\`/`\}` escapes —
//! pure data, no editor state (0059 §12's snippet engine, parse half).

/// One snippet segment in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Literal text, escapes resolved.
    Text(String),
    /// `${1:default}` / `$1`: a placeholder at `index` carrying default
    /// text (possibly empty). Repeated indexes are linked tabstops.
    Tabstop { index: u32, default: String },
    /// `${1|a,b,c|}`: a choice at `index`. Expansion takes the first
    /// option; the picker affordance is a recorded boundary, not a guess.
    Choice { index: u32, options: Vec<String> },
    /// `$0`: the final stop (caret lands here; ends the session).
    Final,
}

/// A parsed snippet: segments plus the expanded plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub segments: Vec<Segment>,
    /// The expanded text (defaults and first choices spliced in).
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnippetError {
    #[error("snippet contains an unterminated construct")]
    Unterminated,
    #[error("snippet contains an invalid tabstop index")]
    Index,
    #[error("snippet choice has no options")]
    EmptyChoice,
}

/// Parse LSP snippet text. Returns the segments and the expanded text;
/// a malformed snippet is a typed error, never a guessed expansion.
pub fn parse(source: &str) -> Result<Snippet, SnippetError> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut text = String::new();
    let mut literal = String::new();
    let bytes = source.as_bytes();
    let mut at = 0usize;
    fn flush(segments: &mut Vec<Segment>, literal: &mut String) {
        if !literal.is_empty() {
            segments.push(Segment::Text(std::mem::take(literal)));
        }
    }
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => {
                at += 1;
                let Some(&escaped) = bytes.get(at) else {
                    return Err(SnippetError::Unterminated);
                };
                if matches!(escaped, b'$' | b'\\' | b'}') {
                    literal.push(escaped as char);
                    text.push(escaped as char);
                } else {
                    // a lone backslash is literal (spec: unknown escapes
                    // pass through)
                    literal.push('\\');
                    literal.push(escaped as char);
                    text.push('\\');
                    text.push(escaped as char);
                }
                at += 1;
            }
            b'$' => {
                at += 1;
                let (index, default, choice, final_stop) = if bytes.get(at) == Some(&b'{') {
                    at += 1;
                    let close = source[at..].find('}').ok_or(SnippetError::Unterminated)? + at;
                    let body = &source[at..close];
                    at = close + 1;
                    let (index_part, rest) = body
                        .split_once(':')
                        .or_else(|| body.split_once('|'))
                        .map_or((body, None), |(head, tail)| (head, Some(tail)));
                    let index: u32 = index_part.parse().map_err(|_| SnippetError::Index)?;
                    if body.contains('|') {
                        // `${i|a,b,c|}`: the trailing pipe closes the list
                        let tail = rest.unwrap_or_default();
                        let tail = tail.strip_suffix('|').unwrap_or(tail);
                        if tail.is_empty() {
                            return Err(SnippetError::EmptyChoice);
                        }
                        let options: Vec<String> = tail.split(',').map(str::to_string).collect();
                        (index, String::new(), Some(options), false)
                    } else {
                        (
                            index,
                            rest.unwrap_or_default().to_string(),
                            None,
                            index == 0,
                        )
                    }
                } else {
                    let start = at;
                    while bytes.get(at).is_some_and(|b| b.is_ascii_digit()) {
                        at += 1;
                    }
                    if at == start {
                        // `$` followed by nothing tabstop-shaped: literal $
                        literal.push('$');
                        text.push('$');
                        continue;
                    }
                    let index: u32 = source[start..at].parse().map_err(|_| SnippetError::Index)?;
                    (index, String::new(), None, index == 0)
                };
                if final_stop {
                    flush(&mut segments, &mut literal);
                    segments.push(Segment::Final);
                } else if let Some(options) = choice {
                    flush(&mut segments, &mut literal);
                    text.push_str(&options[0]);
                    segments.push(Segment::Choice { index, options });
                } else {
                    flush(&mut segments, &mut literal);
                    text.push_str(&default);
                    segments.push(Segment::Tabstop { index, default });
                }
            }
            _ => {
                let ch = source[at..]
                    .chars()
                    .next()
                    .ok_or(SnippetError::Unterminated)?;
                literal.push(ch);
                text.push(ch);
                at += ch.len_utf8();
            }
        }
    }
    flush(&mut segments, &mut literal);
    Ok(Snippet { segments, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through_verbatim() {
        let parsed = parse("println!(\"{}\", value);").unwrap();
        assert_eq!(parsed.text, "println!(\"{}\", value);");
        assert_eq!(parsed.segments.len(), 1);
    }

    #[test]
    fn tabstops_and_defaults_expand_and_keep_indexes() {
        let parsed = parse("fn ${1:name}(${2:arg}) {\n    ${0}\n}").unwrap();
        assert_eq!(parsed.text, "fn name(arg) {\n    \n}");
        assert_eq!(
            parsed.segments,
            vec![
                Segment::Text("fn ".into()),
                Segment::Tabstop {
                    index: 1,
                    default: "name".into()
                },
                Segment::Text("(".into()),
                Segment::Tabstop {
                    index: 2,
                    default: "arg".into()
                },
                Segment::Text(") {\n    ".into()),
                Segment::Final,
                Segment::Text("\n}".into()),
            ]
        );
    }

    #[test]
    fn choices_expand_to_the_first_option() {
        let parsed = parse("let x: ${1|usize,String,Vec<u8>|} = $2;").unwrap();
        assert_eq!(parsed.text, "let x: usize = ;");
        assert_eq!(
            parsed.segments,
            vec![
                Segment::Text("let x: ".into()),
                Segment::Choice {
                    index: 1,
                    options: vec!["usize".into(), "String".into(), "Vec<u8>".into()]
                },
                Segment::Text(" = ".into()),
                Segment::Tabstop {
                    index: 2,
                    default: String::new()
                },
                Segment::Text(";".into()),
            ]
        );
    }

    #[test]
    fn escapes_and_linked_tabstops() {
        let parsed = parse("\\$not-a-stop $1 and ${1:linked}").unwrap();
        assert_eq!(parsed.text, "$not-a-stop  and linked");
        assert_eq!(
            parsed.segments,
            vec![
                Segment::Text("$not-a-stop ".into()),
                Segment::Tabstop {
                    index: 1,
                    default: String::new()
                },
                Segment::Text(" and ".into()),
                Segment::Tabstop {
                    index: 1,
                    default: "linked".into()
                },
            ]
        );
    }

    #[test]
    fn malformed_constructs_are_typed_errors() {
        assert!(parse("fn ${1:name").is_err());
        assert!(parse("fn ${x:name}").is_err());
        assert!(parse("fn ${1|}").is_err());
    }
}
