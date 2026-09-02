//! Git memory surfaces (M3): commit browser, changed-files dive, delta
//! view, blame card, permalinks. Every surface is a real readonly buffer
//! (0001 §3: motions, /, yank work); jobs post onto the event loop
//! (0001 §5.6: no blocking the input path on shell git).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::Buffer;
use strop_git::memory::{self, BlameCard, ChangedFile, LogRow};

use super::{Editor, Key, Mode};

/// What a readonly buffer is — drives Enter/q and line parsing.
#[derive(Debug, Clone)]
pub enum Surface {
    CommitLog {
        rows: Vec<LogRow>,
    },
    ChangedFiles {
        sha: String,
        files: Vec<ChangedFile>,
    },
    /// Unified diff text for one file at one commit.
    DeltaView,
}

/// Jobs post results here; the event loop drains (never blocks input).
pub enum GitJob {
    Log { buffer: usize, rows: Vec<LogRow> },
    Blame(Box<BlameCard>),
    Error(String),
}

impl Editor {
    // ---- surface lifecycle --------------------------------------------

    fn push_surface(&mut self, name: Option<&str>, text: &str, surface: Surface) {
        let mut buf = Buffer::from_text(text);
        buf.readonly = true;
        buf.name = name.map(|n| n.to_string());
        self.buffers.push(buf);
        self.surfaces.push(Some(surface));
        self.highlighters.push(None); // surfaces render via delta/plain rules
        self.current = self.buffers.len() - 1;
        self.touch_mru(self.current);
        self.cursor = 0;
        self.view_top = 0;
    }

    pub fn surface(&self) -> Option<&Surface> {
        self.surfaces.get(self.current).and_then(|s| s.as_ref())
    }

    /// `Space g l`: commit browser. `Space g h`: log scoped to the file.
    pub(crate) fn open_log(&mut self, file_scoped: bool) {
        let Some(repo) = &self.git else {
            self.message = "not a git repo".into();
            return;
        };
        let workdir = repo.workdir().to_path_buf();
        let file = if file_scoped {
            self.buf().path.as_deref().and_then(|p| {
                let abs = if Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    workdir.join(p)
                };
                abs.strip_prefix(&workdir).ok().map(|r| r.to_path_buf())
            })
        } else {
            None
        };
        self.push_surface(
            Some(if file_scoped {
                "git log ·file"
            } else {
                "git log"
            }),
            "loading log…",
            Surface::CommitLog { rows: vec![] },
        );
        let idx = self.current;
        let tx = self.git_tx.clone();
        std::thread::spawn(move || {
            let msg = match memory::log_graph(&workdir, 200, file.as_deref()) {
                Ok(rows) => GitJob::Log { buffer: idx, rows },
                Err(e) => GitJob::Error(e),
            };
            let _ = tx.send(msg);
        });
    }

    /// `Space g b`: blame card for the cursor line.
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
        let line = self.buf().line_of(self.cursor) + 1;
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
            let msg = match memory::blame_line(&workdir, &rel, line) {
                Ok(card) => GitJob::Blame(Box::new(card)),
                Err(e) => GitJob::Error(e),
            };
            let _ = tx.send(msg);
        });
    }

    /// `Space g y`: permalink for the cursor line (or visual range) —
    /// SHA-resolved, remote-prioritized (0001 pillar 3.3).
    pub(crate) fn yank_permalink(&mut self) {
        match self.build_permalink() {
            Some(url) => {
                self.set_register(None, url.clone(), false);
                self.osc52 = Some(url);
                self.message = "permalink copied".into();
            }
            None => self.message = "no remote / not a repo".into(),
        }
    }

    /// `Space g o`: open the permalink in the browser.
    pub(crate) fn open_permalink(&mut self) {
        let Some(url) = self.build_permalink() else {
            self.message = "no remote / not a repo".into();
            return;
        };
        for opener in ["wslview", "xdg-open", "open"] {
            if std::process::Command::new(opener)
                .arg(&url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
            {
                self.message = format!("opened {url}");
                return;
            }
        }
        self.message = format!("no opener found — {url}");
    }

    fn build_permalink(&self) -> Option<String> {
        let repo = self.git.as_ref()?;
        let path = self.buf().path.as_deref()?;
        let abs = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            repo.workdir().join(path)
        };
        let rel = abs.strip_prefix(repo.workdir()).ok()?;
        let (a, b) = if self.mode == Mode::Visual || self.mode == Mode::VisualLine {
            (
                self.buf().line_of(self.anchor) + 1,
                self.buf().line_of(self.cursor) + 1,
            )
        } else {
            let l = self.buf().line_of(self.cursor) + 1;
            (l, l)
        };
        memory::permalink(repo, rel, a.min(b), a.max(b))
    }

    // ---- surface interaction -------------------------------------------

    /// Keys for readonly surface buffers (0001 §3): motions, /, yank, and
    /// the leader fall through; q closes, Enter dives, edits refuse.
    pub(crate) fn feed_readonly(&mut self, key: Key) {
        if !self.pending.is_empty() {
            return self.feed_pending_readonly(key);
        }
        match key {
            Key::Char('q') => {
                self.close_buffer(true);
            }
            Key::Enter => self.dive(),
            Key::Char(c) if "hjklwbeWEB0$G%/fFtT[]".contains(c) || c.is_ascii_digit() => {
                self.pending.push(c);
                self.resolve_pending_readonly();
            }
            Key::Char(c @ ('g' | 'y' | ' ' | ':')) => {
                self.pending.push(c);
            }
            Key::Char('v') => {
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
            }
            Key::Char(_) => self.message = "readonly — q closes, enter dives".into(),
            _ => {}
        }
    }

    fn feed_pending_readonly(&mut self, key: Key) {
        match key {
            Key::Esc => self.pending.clear(),
            Key::Enter => {
                if self.pending.starts_with(':') {
                    self.run_ex(); // :q & friends work on surfaces too
                } else if self.pending.contains('/') {
                    self.pending.push('\r');
                    self.resolve_pending_readonly();
                } else {
                    self.pending.clear();
                }
            }
            Key::Char(c) => {
                // leader namespaces still work from a surface
                if self.pending == " " {
                    self.pending.clear();
                    if c == 'g' {
                        self.pending = " g".into();
                    }
                    return;
                }
                if self.pending == " g" {
                    return self.feed_git_pending(c);
                }
                if (self.pending == "]" || self.pending == "[") && c == 'c' {
                    let forward = self.pending == "]";
                    self.pending.clear();
                    return self.jump_hunk(forward);
                }
                self.pending.push(c);
                self.resolve_pending_readonly();
            }
            _ => {}
        }
    }

    /// Motions and yank resolve; mutations refuse with a message.
    fn resolve_pending_readonly(&mut self) {
        match strop_grammar::parse(&self.pending) {
            strop_grammar::Parse::Incomplete => {}
            strop_grammar::Parse::Invalid => {
                self.pending.clear();
            }
            strop_grammar::Parse::Complete(cmd) => {
                self.pending.clear();
                match cmd.op {
                    None => self.move_cursor(&cmd),
                    Some(strop_grammar::Op::Yank) => self.yank_only(&cmd),
                    Some(_) => self.message = "readonly buffer".into(),
                }
            }
        }
    }

    fn yank_only(&mut self, cmd: &strop_grammar::Command) {
        if let Some(r) = strop_grammar::resolve(self.buf(), self.cursor, cmd) {
            let text = self.buf().slice_string(r.range);
            self.set_register(cmd.register, text, r.range.linewise);
            self.flash(r.range);
        }
    }

    /// Enter on a surface line dives deeper (0001 pillar 3.2).
    fn dive(&mut self) {
        let line = self.buf().line_of(self.cursor);
        match self.surface().cloned() {
            Some(Surface::CommitLog { rows }) => {
                let Some(sha) = rows.get(line).and_then(|r| r.sha.clone()) else {
                    return;
                };
                let Some(repo) = &self.git else { return };
                match memory::show_stat(repo.workdir(), &sha) {
                    Ok(files) => {
                        let mut text =
                            format!("commit {}\n\n", sha.chars().take(10).collect::<String>());
                        for f in &files {
                            text.push_str(&format!(
                                "{:<48} +{} -{}\n",
                                f.path.display(),
                                f.added,
                                f.deleted
                            ));
                        }
                        self.push_surface(
                            Some("commit files"),
                            &text,
                            Surface::ChangedFiles { sha, files },
                        );
                    }
                    Err(e) => self.message = e,
                }
            }
            Some(Surface::ChangedFiles { sha, files }) => {
                // row 0/1 are the header
                let Some(file) = line.checked_sub(2).and_then(|i| files.get(i)) else {
                    return;
                };
                let Some(repo) = &self.git else { return };
                match memory::show_file_delta(repo.workdir(), &sha, &file.path) {
                    Ok(text) => self.push_surface(Some("delta"), &text, Surface::DeltaView),
                    Err(e) => self.message = e,
                }
            }
            _ => {}
        }
    }

    // ---- job drain ------------------------------------------------------

    pub fn drain_git_jobs(&mut self) {
        while let Ok(job) = self.git_rx.try_recv() {
            match job {
                GitJob::Log { buffer, rows } => {
                    if buffer < self.buffers.len() {
                        let text = rows
                            .iter()
                            .map(|r| r.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n";
                        self.buffers[buffer].replace_all(&text);
                        if let Some(Some(Surface::CommitLog { rows: slot })) =
                            self.surfaces.get_mut(buffer)
                        {
                            *slot = rows;
                        }
                    }
                }
                GitJob::Blame(card) => self.blame_card = Some(*card),
                GitJob::Error(e) => self.message = e,
            }
        }
    }
}

/// The git job channel ends (created once in `Editor::new`).
pub fn git_channel() -> (Sender<GitJob>, Receiver<GitJob>) {
    channel()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use crate::editor::{Editor, Key};
    use strop_core::Buffer;

    /// Repo with two commits; second changes f.rs.
    fn fixture() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "first"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        git(&["commit", "-qam", "add b"]);
        let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.discover_git();
        (dir, e)
    }

    fn pump(e: &mut Editor) {
        // let job threads deliver (bounded, like headless settle)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            e.drain_git_jobs();
            let loaded = e.surface().is_some_and(
                |s| matches!(s, crate::editor::Surface::CommitLog { rows } if !rows.is_empty()),
            );
            if loaded || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn commit_browser_dives_to_delta() {
        let (_d, mut e) = fixture();
        e.open_log(false);
        pump(&mut e);
        let text = e.buf().rope.to_string();
        assert!(text.contains("add b"), "{text}");
        assert!(text.contains("first"), "{text}");
        assert!(e.buf().readonly, "browser is a readonly real buffer");

        // motions work on the surface
        e.feed_text("j");
        // Enter on a commit row → changed files
        e.feed_text("k");
        e.feed(Key::Enter);
        let text = e.buf().rope.to_string();
        assert!(text.contains("commit"), "{text}");
        assert!(text.contains("f.rs"), "{text}");
        assert!(text.contains("+1 -0"), "{text}");

        // Enter on the file row → delta view
        e.feed_text("j");
        e.feed_text("j");
        e.feed(Key::Enter);
        let text = e.buf().rope.to_string();
        assert!(text.contains("+fn b() {}"), "{text}");

        // edits refuse, q climbs out
        e.feed_text("x");
        assert!(e.message.contains("readonly"));
        e.feed_text("q");
        assert!(matches!(
            e.surface(),
            Some(crate::editor::Surface::ChangedFiles { .. })
        ));
    }

    #[test]
    fn blame_card_shows_commit() {
        let (_d, mut e) = fixture();
        e.feed_text("j"); // line 2 (fn b)
        e.blame_line();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while e.blame_card.is_none() && std::time::Instant::now() < deadline {
            e.drain_git_jobs();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let card = e.blame_card.as_ref().expect("blame card");
        assert_eq!(card.summary, "add b");
        assert_eq!(card.author, "t");
    }

    #[test]
    fn permalink_needs_remote() {
        let (_d, e) = fixture();
        // no remote configured → honest refusal
        assert!(e.build_permalink().is_none());
    }

    #[test]
    fn permalink_resolves_sha_and_ssh_remote() {
        let (_d, mut e) = fixture();
        let root = e.cwd.clone();
        Command::new("git")
            .args([
                "-C",
                &root.display().to_string(),
                "remote",
                "add",
                "origin",
                "git@github.com:stropdev/strop.git",
            ])
            .output()
            .unwrap();
        e.discover_git();
        e.feed_text("j"); // line 2
        let url = e.build_permalink().expect("permalink");
        assert!(
            url.starts_with("https://github.com/stropdev/strop/blob/"),
            "{url}"
        );
        assert!(url.ends_with("/f.rs#L2"), "{url}");
        assert!(!url.contains("/main/"), "branch must resolve to SHA: {url}");
        e.yank_permalink();
        assert_eq!(e.register(None).0, url);
        assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
    }
}
