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
    /// Spawn `rg` for `pattern` in `cwd`, streaming vimgrep lines.
    pub fn spawn(pattern: &str, cwd: &std::path::Path, tx: Sender<PickerMsg>) -> Option<Self> {
        if pattern.is_empty() {
            return None;
        }
        let mut child = Command::new("rg")
            .args([
                "--vimgrep",
                "--color",
                "never",
                "--no-heading",
                "--smart-case",
                "--",
                pattern,
            ])
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(item) = parse_vimgrep(&line) {
                    if tx.send(PickerMsg::Items(vec![item])).is_err() {
                        return; // picker closed
                    }
                }
            }
            let _ = tx.send(PickerMsg::Done);
        });
        Some(Self { child: Some(child) })
    }
}

impl Drop for GrepWorker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// `path:line:col:text` → item. Display is `path:line · trimmed match`.
fn parse_vimgrep(line: &str) -> Option<Item> {
    let mut parts = line.splitn(4, ':');
    let path = PathBuf::from(parts.next()?);
    let line_no: usize = parts.next()?.parse().ok()?;
    let col: usize = parts.next()?.parse().ok()?;
    let text = parts.next()?.to_string();
    let trimmed = text.trim();
    let short = if trimmed.len() > 60 {
        &trimmed[..60]
    } else {
        trimmed
    };
    Some(Item {
        text: format!("{}:{line_no} · {short}", path.display()),
        payload: Payload::Grep {
            path,
            line: line_no,
            col,
            line_text: text,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vimgrep_parses() {
        let item = parse_vimgrep("src/main.rs:12:7:    let x = sharpen();").unwrap();
        match item.payload {
            Payload::Grep {
                path, line, col, ..
            } => {
                assert_eq!(path, PathBuf::from("src/main.rs"));
                assert_eq!((line, col), (12, 7));
            }
            _ => panic!("wrong payload"),
        }
        assert!(item.text.contains("let x = sharpen();"));
        assert!(parse_vimgrep("not a match line").is_none());
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
