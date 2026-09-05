//! Per-project sessions (0001 pillar 4): buffers, cursor positions, view
//! offsets, and undo histories serialize to XDG state on save/quit and
//! restore on open. Undo depth is capped (0001 §3: full trees bloat).
//! Readonly surfaces and scratch buffers never persist.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::editor::Editor;
use strop_core::history::History;
use strop_core::Buffer;

const UNDO_CAP: usize = 200;

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Session {
    pub buffers: Vec<BufferState>,
    pub current: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BufferState {
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub view_top: usize,
    /// Linear undo path (root→current, capped). Branches don't cross
    /// sessions — the tree lives in-memory; the cap is the contract.
    pub undo: Option<History>,
    /// Hash of the text the undo path was captured against (0015): a
    /// session may persist dirty state whose history assumes text the
    /// disk never held — restore verifies before replaying.
    #[serde(default)]
    pub undo_hash: u64,
}

/// `base_dir` is the resolved XDG state dir (None → sessions off).
/// Tests pass their tempdir explicitly — process-global env never
/// enters the write path (the 0.4.0 flake class).
fn session_path(base_dir: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    let base = base_dir?.to_path_buf();
    // stable per-project identity without leaking the path
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&cwd, &mut hasher);
    let key = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    Some(
        base.join("strop")
            .join("sessions")
            .join(format!("{key}.json")),
    )
}

/// Snapshot the editor into a Session.
pub(crate) fn capture(editor: &Editor) -> Option<Session> {
    let mut buffers = Vec::new();
    for (id, doc) in editor.docs.iter() {
        let buf = &doc.buf;
        if buf.readonly || buf.path.is_none() {
            continue;
        }
        let path = buf.path.clone()?;
        let (undo, undo_hash) = if buf.history.depth() > 0 {
            let mut h = buf.history.clone();
            h.cap(UNDO_CAP);
            (Some(h), content_hash(&buf.rope.to_string()))
        } else {
            (None, 0)
        };
        buffers.push(BufferState {
            path,
            line: if id == editor.current() {
                editor.buf().line_of(editor.head())
            } else {
                0
            },
            col: if id == editor.current() {
                editor.buf().col_of(editor.head())
            } else {
                0
            },
            view_top: if id == editor.current() {
                editor.view_top()
            } else {
                0
            },
            undo,
            undo_hash,
        });
    }
    if buffers.is_empty() {
        return None;
    }
    let current = buffers
        .iter()
        .position(|b| Some(&b.path) == editor.cur().buf.path.as_ref())
        .unwrap_or(0);
    Some(Session { buffers, current })
}

/// Restore a session into the editor (replaces its initial buffer).
/// FNV-1a over the full text — cheap, deterministic, and only ever
/// compared within one machine's sessions.
fn content_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn restore(editor: &mut Editor) -> bool {
    let Some(path) = session_path(editor.state_dir.as_deref(), &editor.cwd) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(session) = serde_json::from_str::<Session>(&text) else {
        return false;
    };
    if session.buffers.is_empty() {
        return false;
    }
    editor.docs.clear();
    for b in &session.buffers {
        let mut buf = Buffer::open(&b.path).unwrap_or_else(|_| Buffer::from_text(""));
        if let Some(h) = &b.undo {
            // the history only speaks for the text it was captured
            // against — a mismatch (the session saved dirty state the
            // disk never held) drops it, never replays (0015)
            if content_hash(&buf.rope.to_string()) == b.undo_hash {
                buf.history = h.clone();
            } else {
                editor.message =
                    format!("{}: changed since capture — undo history dropped", b.path);
            }
        }
        editor.docs.insert(crate::editor::Document::new(buf));
    }
    let nth = session.current.min(editor.docs.len().saturating_sub(1));
    let cur_id = editor
        .docs
        .iter()
        .nth(nth)
        .map(|(id, _)| id)
        .expect("docs non-empty");
    editor.view_mut().doc = cur_id;
    let b = &session.buffers[nth];
    editor.view_mut().view_top = b.view_top;
    let line_start = editor
        .buf()
        .line_start(b.line.min(editor.buf().len_lines() - 1));
    editor.set_head(editor.buf().clamp_boundary(line_start + b.col));
    editor.clamp_cursor();
    editor.mru = editor.docs.iter().map(|(id, _)| id).collect();
    editor.touch_mru(editor.current());
    editor.discover_git();
    true
}

/// Save the editor's session for the project (called on :w/:q paths).
pub fn save(editor: &Editor) {
    let Some(path) = session_path(editor.state_dir.as_deref(), &editor.cwd) else {
        return;
    };
    let Some(session) = capture(editor) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string(&session).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn roundtrip_restores_buffers_and_position() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.state_dir = Some(root.join("state"));
        e.feed_text("jl"); // line 2, col 2
        e.feed_text("ix"); // dirty edit (recorded in undo history)
        e.feed(crate::editor::Key::Esc);
        save(&e);
        let mut e2 = Editor::new(Buffer::from_text(""));
        e2.cwd = root.to_path_buf();
        e2.state_dir = Some(root.join("state"));
        assert!(restore(&mut e2));
        assert_eq!(
            e2.buf().path.as_deref(),
            Some(root.join("a.rs").to_str().unwrap())
        );
        assert_eq!(e2.buf().line_of(e2.head()), 1);
        assert_eq!(e2.buf().col_of(e2.head()), 1);
        // the session captured DIRTY history; the disk text differs —
        // the history must NOT cross (0015: replaying it against the
        // wrong text corrupts). A clean save is what carries undo.
        // (depth 1 = the root sentinel alone = a fresh history)
        assert_eq!(
            e2.buf().history.depth(),
            1,
            "dirty history must not replay against the disk version"
        );
        assert!(e2.message.contains("undo history dropped"));
        let _ = Command::new("true").output();
    }

    #[test]
    fn empty_or_readonly_never_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text(""));
        e.cwd = dir.path().to_path_buf();
        e.state_dir = Some(dir.path().join("state"));
        save(&e);
        assert!(!dir.path().join("state").exists());
    }
    #[test]
    fn undo_history_crosses_when_disk_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.state_dir = Some(root.join("state"));
        e.feed_text("o// note");
        e.feed(crate::editor::Key::Esc);
        e.feed_text(":w\r"); // the disk now matches the capture
        save(&e);
        let mut e2 = Editor::new(Buffer::from_text(""));
        e2.cwd = root.to_path_buf();
        e2.state_dir = Some(root.join("state"));
        assert!(restore(&mut e2));
        assert!(
            e2.buf().history.depth() > 1,
            "history crosses when the text matches"
        );
        e2.feed_text("u");
        assert_eq!(e2.buf().rope.to_string(), "fn a() {}\n");
    }
}
