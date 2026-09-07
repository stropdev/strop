//! Query parsing: the rg pattern/filters split and the JSON match
//! decoder (unchanged behavior, isolated for reuse by the grep
//! supervisor).

use std::path::PathBuf;

use crate::{Item, Payload};

/// Split a grep query into the rg pattern and passthrough filter args:
/// `-t rs` / `--type rs` / `--type=rs`, `-g 'glob'` / `--glob 'glob'` /
/// `--glob=glob`. Everything else joins back into the pattern.
pub fn split_query(input: &str) -> (String, Vec<String>) {
    let mut pattern: Vec<&str> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut it = input.split_whitespace().peekable();
    while let Some(tok) = it.next() {
        match tok {
            "-t" | "--type" => {
                if let Some(v) = it.next() {
                    args.extend(["--type".to_string(), v.to_string()]);
                }
            }
            "-g" | "--glob" => {
                if let Some(v) = it.next() {
                    args.extend(["--glob".to_string(), v.to_string()]);
                }
            }
            _ if tok.starts_with("-t") && tok.len() > 2 => {
                args.extend(["--type".to_string(), tok[2..].to_string()]);
            }
            _ if tok.starts_with("-g") && tok.len() > 2 => {
                args.extend(["--glob".to_string(), tok[2..].to_string()]);
            }
            _ if tok.starts_with("--type=") || tok.starts_with("--glob=") => {
                args.push(tok.to_string());
            }
            _ => pattern.push(tok),
        }
    }
    (pattern.join(" "), args)
}

/// One rg --json event line → one item per submatch (a line can hold
/// several). Non-match events and byte-encoded paths are skipped.
pub fn parse_json_match(line: &str) -> Vec<Item> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    if v["type"] != "match" {
        return Vec::new();
    }
    let data = &v["data"];
    let Some(path) = data["path"]["text"].as_str() else {
        return Vec::new(); // invalid-UTF8 path names arrive as bytes
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
                text: format!("{}:{line_no} · {short}", path),
                payload: Payload::Grep {
                    path: PathBuf::from(path),
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

    #[test]
    fn query_filters_split_out() {
        let (pat, args) = split_query("sharpen -t rs --glob !target/*");
        assert_eq!(pat, "sharpen");
        assert_eq!(args, ["--type", "rs", "--glob", "!target/*"]);
        let (pat, args) = split_query("foo bar -trs");
        assert_eq!(pat, "foo bar");
        assert_eq!(args, ["--type", "rs"]);
        let (pat, args) = split_query("--type=py read");
        assert_eq!(pat, "read");
        assert_eq!(args, ["--type=py"]);
    }
}
