//! Streaming sources. Workers run on plain threads and post onto the
//! event loop through a channel — the async invariant (0001 §5.6): input
//! never waits on a source. Tokio arrives with LSP; threads suffice here.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;

use crate::{Item, Payload};

/// Messages workers post to the event loop.
pub enum PickerMsg {
    Items(Vec<Item>),
    Done,
}

/// Walk the working directory (respects .gitignore via `ignore`),
/// streaming paths in chunks.
pub fn spawn_files(cwd: PathBuf, tx: Sender<PickerMsg>) {
    std::thread::spawn(move || {
        let mut batch = Vec::with_capacity(512);
        for entry in ignore::WalkBuilder::new(&cwd)
            .hidden(true)
            .build()
            .flatten()
        {
            let Ok(rel) = entry.path().strip_prefix(&cwd) else {
                continue;
            };
            if entry.file_type().is_some_and(|t| t.is_dir()) {
                continue;
            }
            let rel = rel.to_path_buf();
            batch.push(Item {
                text: rel.display().to_string(),
                payload: Payload::File(rel),
            });
            if batch.len() >= 512
                && tx
                    .send(PickerMsg::Items(std::mem::take(&mut batch)))
                    .is_err()
            {
                return;
            }
        }
        if !batch.is_empty() {
            let _ = tx.send(PickerMsg::Items(batch));
        }
        let _ = tx.send(PickerMsg::Done);
    });
}

/// A grep worker. Kill on drop: each keystroke respawns the search, and
/// dropping the guard kills the previous rg before it can flood.
pub struct GrepWorker {
    child: Option<Child>,
}

impl GrepWorker {
    /// Spawn `rg` for `query` in `cwd`, streaming JSON match events.
    /// The query splits into a pattern plus passthrough filter flags
    /// (`-t rs`, `--glob '!target/*'` — rootle-style power filters).
    pub fn spawn(query: &str, cwd: &std::path::Path, tx: Sender<PickerMsg>) -> Option<Self> {
        let (pattern, filters) = split_query(query);
        if pattern.is_empty() {
            return None;
        }
        let mut cmd = Command::new("rg");
        cmd.args(["--json", "--smart-case"])
            .args(&filters)
            .arg("--")
            .arg(&pattern)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        let mut child = cmd.spawn().ok()?;
        let stdout = child.stdout.take()?;
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let items = parse_json_match(&line);
                if items.is_empty() {
                    continue;
                }
                if tx.send(PickerMsg::Items(items)).is_err() {
                    return; // picker closed
                }
            }
            let _ = tx.send(PickerMsg::Done);
        });
        Some(Self { child: Some(child) })
    }
}

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
fn parse_json_match(line: &str) -> Vec<Item> {
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

impl Drop for GrepWorker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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

#[cfg(test)]
mod worker_tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;

    #[test]
    fn grep_worker_streams_and_finishes() {
        let dir = std::env::temp_dir().join("strop-picker-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn sharpen() {}\nlet x = sharpen;\n").unwrap();
        let (tx, rx) = channel();
        let _w = GrepWorker::spawn("sharpen", &dir, tx).expect("rg available");
        let mut items = 0;
        let mut done = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PickerMsg::Items(batch)) => items += batch.len(),
                Ok(PickerMsg::Done) => done = true,
                Err(_) => break,
            }
        }
        std::fs::remove_dir_all(&dir).ok();
        assert!(done, "worker never sent Done");
        assert_eq!(items, 2);
    }

    #[test]
    fn files_worker_walks() {
        let dir = std::env::temp_dir().join("strop-picker-walk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.rs"), "").unwrap();
        let (tx, rx) = channel();
        spawn_files(dir.clone(), tx);
        let mut found = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PickerMsg::Items(batch)) => {
                    if batch.iter().any(|i| i.text.contains("x.rs")) {
                        found = true;
                    }
                }
                Ok(PickerMsg::Done) => break,
                Err(_) => break,
            }
        }
        std::fs::remove_dir_all(&dir).ok();
        assert!(found);
    }
}
