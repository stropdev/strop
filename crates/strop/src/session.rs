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
}

fn session_path(cwd: &Path) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state"))
        })?;
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
    for (i, buf) in editor.buffers.iter().enumerate() {
        if buf.readonly || buf.path.is_none() {
            continue;
        }
        let path = buf.path.clone()?;
        let undo = if buf.history.depth() > 0 {
            let mut h = buf.history.clone();
            h.cap(UNDO_CAP);
            Some(h)
        } else {
            None
        };
        buffers.push(BufferState {
            path,
            line: if i == editor.current {
                editor.buf().line_of(editor.cursor)
            } else {
                0
            },
            col: if i == editor.current {
                editor.buf().col_of(editor.cursor)
            } else {
                0
            },
            view_top: if i == editor.current {
                editor.view_top
            } else {
                0
            },
            undo,
        });
    }
    if buffers.is_empty() {
        return None;
    }
    let current = buffers
        .iter()
        .position(|b| Some(&b.path) == editor.buffers[editor.current].path.as_ref())
        .unwrap_or(0);
    Some(Session { buffers, current })
}

/// Restore a session into the editor (replaces its initial buffer).
pub fn restore(editor: &mut Editor) -> bool {
    let Some(path) = session_path(&editor.cwd) else {
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
    editor.buffers.clear();
    editor.surfaces.clear();
    editor.highlighters.clear();
    for b in &session.buffers {
        let mut buf = Buffer::open(&b.path).unwrap_or_else(|_| Buffer::from_text(""));
        if let Some(h) = &b.undo {
            buf.history = h.clone();
        }
        let hl = buf
            .path
            .as_deref()
            .and_then(strop_syntax::Highlighter::for_path);
        editor.buffers.push(buf);
        editor.surfaces.push(None);
        editor.highlighters.push(hl);
    }
    editor.current = session.current.min(editor.buffers.len() - 1);
    let b = &session.buffers[editor.current];
    editor.view_top = b.view_top;
    let line_start = editor
        .buf()
        .line_start(b.line.min(editor.buf().len_lines() - 1));
    editor.cursor = editor.buf().clamp_boundary(line_start + b.col);
    editor.clamp_cursor();
    editor.mru = (0..editor.buffers.len()).collect();
    editor.touch_mru(editor.current);
    editor.discover_git();
    true
}

/// Save the editor's session for the project (called on :w/:q paths).
pub fn save(editor: &Editor) {
    let Some(path) = session_path(&editor.cwd) else {
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

    /// XDG_STATE_HOME is process-global — parallel tests mutating it
    /// race (the flake class of 0.3.9). Serialize the module.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn roundtrip_restores_buffers_and_position() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let _guard = ENV_LOCK.lock().unwrap();
        // no HOME pollution: point XDG_STATE_HOME at the tempdir
        std::env::set_var("XDG_STATE_HOME", root.join("state"));
        let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.feed_text("jl"); // line 2, col 2
        e.feed_text("ix"); // dirty edit (recorded in undo history)
        e.feed(crate::editor::Key::Esc);
        save(&e);
        let mut e2 = Editor::new(Buffer::from_text(""));
        e2.cwd = root.to_path_buf();
        assert!(restore(&mut e2));
        assert_eq!(
            e2.buf().path.as_deref(),
            Some(root.join("a.rs").to_str().unwrap())
        );
        assert_eq!(e2.buf().line_of(e2.cursor), 1);
        assert_eq!(e2.buf().col_of(e2.cursor), 1);
        assert!(
            e2.buf().history.depth() > 0,
            "undo history crossed the session"
        );
        let _ = Command::new("true").output();
    }

    #[test]
    fn empty_or_readonly_never_persist() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("XDG_STATE_HOME", dir.path().join("state"));
        let mut e = Editor::new(Buffer::from_text(""));
        e.cwd = dir.path().to_path_buf();
        save(&e);
        assert!(!dir.path().join("state").exists());
    }
}
