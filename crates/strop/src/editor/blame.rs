//! Blame (0011 §3): the left-margin gutter (rootle's shape), the card,
//! and the dive into the blamed commit. Jobs fill gutters off the input
//! path; a stale epoch or edited buffer drops the pairing honestly.

use std::path::{Path, PathBuf};

use super::git_memory::BlameGutter;
use super::{Editor, GitJob};

impl Editor {
    /// `Space g b`: on a file buffer, toggle the blame gutter; anywhere
    /// else (or as feedback while the gutter loads) the single-line
    /// card (0011 §3).
    pub(crate) fn toggle_blame_gutter(&mut self) {
        if self.buf().readonly || self.buf().path.is_none() {
            return self.blame_line();
        }
        let key = self.blame_key();
        if self.blame_gutters.remove(&key).is_some() {
            return; // toggle off
        }
        self.blame_gutters.insert(
            key.clone(),
            BlameGutter {
                lines: Vec::new(),
                epoch: self.buf().epoch,
            },
        );
        self.spawn_blame_file(&key);
        self.blame_line(); // the card covers the line until data lands
    }

    /// Canonical path key for the current buffer's gutter entry — the
    /// same normalization every lookup uses, so `f.rs` and an absolute
    /// path for one file share one entry.
    pub(crate) fn blame_key(&self) -> PathBuf {
        self.blame_key_of(self.buf().path.as_deref().unwrap_or(""))
    }

    pub(crate) fn blame_key_of(&self, path: &str) -> PathBuf {
        Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| self.cwd.join(path))
    }

    fn spawn_blame_file(&mut self, key: &Path) {
        let Some(repo) = &self.git else {
            self.message = "not a git repo".into();
            return;
        };
        let workdir = repo.workdir().to_path_buf();
        let key = key.to_path_buf(); // owned: the job outlives the caller
        let Ok(rel) = key.strip_prefix(&workdir).map(|r| r.to_path_buf()) else {
            self.message = "buffer not under workdir".into();
            return;
        };
        let generation = self.generation;
        let tx = self.git_tx.clone();
        std::thread::spawn(move || {
            let msg = match strop_git::memory::blame_file(&workdir, &rel) {
                Ok(lines) => GitJob::Gutter {
                    path: key.to_path_buf(),
                    generation,
                    lines,
                },
                Err(e) => GitJob::Error(e),
            };
            let _ = tx.send(msg);
        });
    }

    /// The buffer's blame gutter, if its data is still trustworthy:
    /// same edit epoch, same line count. Any edit since the capture
    pub fn blame_gutter_for(&self, buffer: strop_core::id::DocumentId) -> Option<&BlameGutter> {
        let buf = self.docs.get(buffer).map(|d| &d.buf)?;
        let path = buf.path.as_deref()?;
        let key = Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| self.cwd.join(path));
        let gutter = self.blame_gutters.get(&key)?;
        // len_lines counts the trailing newline's phantom line — the
        // content count is what blame rows pair with
        let content_lines = buf.last_content_line() + 1;
        (gutter.epoch == buf.epoch && gutter.lines.len() == content_lines).then_some(gutter)
    }

    /// `Space g b` fallback / surface blame: the card for the cursor
    /// line.
    pub(crate) fn blame_line(&mut self) {
        let Some(repo) = &self.git else {
            self.message = "not a git repo".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            self.message = "blame works on file buffers".into();
            return;
        };
        let workdir = repo.workdir().to_path_buf();
        let line = self.buf().line_of(self.head()) + 1;
        let generation = self.generation;
        let tx = self.git_tx.clone();
        std::thread::spawn(move || {
            let abs = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                workdir.join(&path)
            };
            let rel = match abs.strip_prefix(&workdir) {
                Ok(r) => r.to_path_buf(),
                Err(_) => {
                    let _ = tx.send(GitJob::Error("not under workdir".into()));
                    return;
                }
            };
            let msg = match strop_git::memory::blame_line(&workdir, &rel, line) {
                Ok(card) => GitJob::Card {
                    generation,
                    card: Box::new(card),
                },
                Err(e) => GitJob::Error(e),
            };
            let _ = tx.send(msg);
        });
    }
}
