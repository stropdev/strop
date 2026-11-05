use super::DevcontainerError;
use serde_json::Value;
use std::path::Path;

/// Default upper bound for one devcontainer.json file.
pub const DEFAULT_INPUT_LIMIT: usize = 1024 * 1024;

/// Parse JSONC without evaluating substitutions, commands or any other input.
/// Comments and trailing commas are blanked rather than deleted so serde's
/// line/column diagnostics still refer to the original file.
pub(crate) fn parse_bytes(
    bytes: &[u8],
    path: &Path,
    limit: usize,
) -> Result<Value, DevcontainerError> {
    if bytes.len() > limit {
        return Err(DevcontainerError::InputTooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|error| DevcontainerError::Json {
        path: path.to_path_buf(),
        line: 0,
        column: error.valid_up_to(),
        detail: "input is not UTF-8".into(),
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let uncommented = blank_comments(text).map_err(|detail| DevcontainerError::Json {
        path: path.to_path_buf(),
        line: 0,
        column: 0,
        detail,
    })?;
    let normalized = blank_trailing_commas(&uncommented);
    let value: Value =
        serde_json::from_str(&normalized).map_err(|error| DevcontainerError::Json {
            path: path.to_path_buf(),
            line: error.line(),
            column: error.column(),
            detail: error.to_string(),
        })?;
    if !value.is_object() {
        return Err(DevcontainerError::InvalidRoot);
    }
    Ok(value)
}

fn blank_comments(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0;
    let mut string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            string = true;
            index += 1;
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            output[index] = b' ';
            output[index + 1] = b' ';
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                output[index] = b' ';
                index += 1;
            }
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            output[index] = b' ';
            output[index + 1] = b' ';
            index += 2;
            let mut closed = false;
            while index < bytes.len() {
                if index + 1 < bytes.len() && bytes[index] == b'*' && bytes[index + 1] == b'/' {
                    output[index] = b' ';
                    output[index + 1] = b' ';
                    index += 2;
                    closed = true;
                    break;
                }
                if bytes[index] != b'\n' && bytes[index] != b'\r' {
                    output[index] = b' ';
                }
                index += 1;
            }
            if !closed {
                return Err("unterminated block comment".into());
            }
            continue;
        }
        index += 1;
    }
    // Every replacement was ASCII, including inside UTF-8 comments where whole
    // bytes became spaces; unmodified UTF-8 therefore remains valid.
    match String::from_utf8(output) {
        Ok(output) => Ok(output),
        Err(_) => panic!("comment blanking preserves UTF-8"),
    }
}

fn blank_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0;
    let mut string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            string = true;
        } else if byte == b',' {
            let mut next = index + 1;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if next < bytes.len() && matches!(bytes[next], b'}' | b']') {
                output[index] = b' ';
            }
        }
        index += 1;
    }
    match String::from_utf8(output) {
        Ok(output) => output,
        Err(_) => panic!("comma blanking preserves UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Value, DevcontainerError> {
        parse_bytes(text.as_bytes(), Path::new("devcontainer.json"), 4096)
    }

    #[test]
    fn comments_commas_and_string_contents_are_distinct() {
        let value = parse(
            r#"{
              // A URL and delimiters in strings are not comments.
              "image": "registry.example/a//b:latest",
              "workspaceFolder": "/work/}/*not-comment*/,",
              "runArgs": ["--name", "quote: \" and comma,",],
              /* block
                 comment */
              "forwardPorts": [3000, "127.0.0.1:4000:4000",],
            }"#,
        )
        .unwrap();
        assert_eq!(value["image"], "registry.example/a//b:latest");
        assert_eq!(value["workspaceFolder"], "/work/}/*not-comment*/,");
        assert_eq!(value["runArgs"][1], "quote: \" and comma,");
        assert_eq!(value["forwardPorts"][1], "127.0.0.1:4000:4000");
    }

    #[test]
    fn malformed_strings_and_comments_are_rejected() {
        assert!(parse(r#"{"image":"unterminated}"#).is_err());
        assert!(parse("{/* never closed").is_err());
        assert!(parse(r#"{"image":"a", "privileged": tru,}"#).is_err());
        assert!(parse("[]").is_err());
    }

    #[test]
    fn input_bound_is_enforced_before_parsing() {
        assert!(matches!(
            parse_bytes(b"{}", Path::new("large.json"), 1),
            Err(DevcontainerError::InputTooLarge { limit: 1, .. })
        ));
    }
}
