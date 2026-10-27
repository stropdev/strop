use crate::languages::LanguageId;
use std::path::Path;
use strop_core::{Buffer, Range, Replacement};

/// Line-comment token used by visual Ctrl-C. Block-only languages have no
/// line toggle; callers report that limitation rather than rewriting delimiters.
pub(crate) fn line_comment(language: LanguageId) -> Option<&'static str> {
    use LanguageId::*;
    match language {
        Rust | JavaScript | TypeScript | Tsx | Go | C | Cpp | Json | Java | CSharp => Some("//"),
        Python | Bash | Fish | CMake | Ruby | Toml | Yaml => Some("#"),
        Lua | Sql => Some("--"),
        Php => Some("//"),
        Css | Html | Markdown | MarkdownInline | MarkdownRustdoc | Sovel => None,
    }
}

/// Toggle complete selected lines using the source language's line token.
/// Returns one non-overlapping edit batch, or a visible refusal before editing.
pub fn toggle(
    path: Option<&Path>,
    buffer: &Buffer,
    selected: Range,
) -> Result<Vec<Replacement>, String> {
    let Some(path) = path else {
        return Err("comment toggle needs a recognized file language".into());
    };
    let first = buffer.line_text(0);
    let first = first.lines().next().unwrap_or("").to_owned();
    let Some(language) = crate::languages::detect(path, Some(&first)) else {
        return Err("comment toggle needs a recognized file language".into());
    };
    let Some(token) = line_comment(language.id) else {
        return Err(format!("{} has no line-comment token", language.name));
    };
    let first = buffer.line_of(selected.start.get());
    let last = buffer.line_of(selected.end.get().saturating_sub(1));
    let mut lines = Vec::new();
    for line in first..=last {
        let start = buffer.line_start(line);
        let content = buffer.line_text(line);
        let content = content.trim_end_matches(['\r', '\n']).to_owned();
        let indent = content.len() - content.trim_start_matches([' ', '\t']).len();
        lines.push((start, content, indent));
    }
    let uncomment = lines
        .iter()
        .filter(|(_, content, _)| !content.trim().is_empty())
        .all(|(_, content, indent)| content[*indent..].starts_with(token));
    let mut edits = Vec::new();
    for (start, content, indent) in lines {
        if content.trim().is_empty() {
            continue;
        }
        if uncomment {
            let at = start + indent;
            let mut end = at + token.len();
            if content.as_bytes().get(indent + token.len()) == Some(&b' ') {
                end += 1;
            }
            edits.push(Replacement::new(Range::charwise(at, end), ""));
        } else {
            edits.push(Replacement::new(
                Range::charwise(start + indent, start + indent),
                format!("{token} "),
            ));
        }
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncomment_removes_token_and_optional_space_independently() {
        let buffer = Buffer::from_text("// one\r\n//two\r\n");
        let edits = toggle(
            Some(Path::new("example.rs")),
            &buffer,
            Range::linewise(0, buffer.len_bytes()),
        )
        .unwrap();
        let ranges: Vec<_> = edits
            .iter()
            .map(|edit| {
                (
                    edit.range.start.get(),
                    edit.range.end.get(),
                    edit.text.as_str(),
                )
            })
            .collect();
        assert_eq!(ranges, [(0, 3, ""), (8, 10, "")]);
    }
}
