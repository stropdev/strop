//! The rg `--json` match decoder (0051: the UI query language lives in
//! `crate::query`; this file is only the wire format adapter).

use base64::Engine;
use std::path::PathBuf;

use crate::{Item, Payload};

/// One rg --json event line → one item per submatch (a line can hold
/// several). Native path bytes are preserved separately from display text.
pub fn parse_json_match(line: &str) -> Vec<Item> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    if v["type"] != "match" {
        return Vec::new();
    }
    let data = &v["data"];
    let path = if let Some(path) = data["path"]["text"].as_str() {
        PathBuf::from(path)
    } else if let Some(bytes) = data["path"]["bytes"].as_str() {
        let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(bytes) else {
            return Vec::new();
        };
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(bytes))
        }
        #[cfg(not(unix))]
        {
            let Ok(path) = String::from_utf8(bytes) else {
                return Vec::new();
            };
            PathBuf::from(path)
        }
    } else {
        return Vec::new();
    };
    let Some(line_no) = data["line_number"].as_u64() else {
        return Vec::new();
    };
    let line_text = data["lines"]["text"]
        .as_str()
        .unwrap_or("")
        .trim_end_matches('\n')
        .to_string();
    let Some(subs) = data["submatches"].as_array() else {
        return Vec::new();
    };
    let trimmed = line_text.trim();
    let short: String = trimmed.chars().take(80).collect();
    subs.iter()
        .filter_map(|s| {
            let start = s["start"].as_u64()? as usize;
            let end = s["end"].as_u64()? as usize;
            Some(Item {
                badge: None,
                text: format!("{}:{line_no} · {short}", path.display()),
                payload: Payload::Grep {
                    path: path.clone(),
                    line: line_no as usize,
                    col: start + 1,
                    match_len: end.saturating_sub(start),
                    line_text: line_text.clone(),
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_match_parses_all_submatches() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"    let x = sharpen(); sharpen();\n"},"line_number":12,"absolute_offset":42,"submatches":[{"match":{"text":"sharpen"},"start":14,"end":21},{"match":{"text":"sharpen"},"start":33,"end":40}]}}"#;
        let items = parse_json_match(line);
        assert_eq!(items.len(), 2);
        match &items[0].payload {
            Payload::Grep {
                path,
                line,
                col,
                match_len,
                ..
            } => {
                assert_eq!(path, &PathBuf::from("src/main.rs"));
                assert_eq!((*line, *col, *match_len), (12, 15, 7));
            }
            _ => panic!("wrong payload"),
        }
        assert!(parse_json_match("not json").is_empty());
        assert!(parse_json_match(r#"{"type":"begin","data":{}}"#).is_empty());
    }
}
