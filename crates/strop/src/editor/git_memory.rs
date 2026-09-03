//! Git memory surfaces (M3, reworked 0010): commit browser, changed-files
//! dive, diff view, blame card, permalinks. Every surface is a real
//! readonly buffer (0001 §3: motions, /, yank work); jobs post onto the
//! event loop (0001 §5.6: no blocking the input path on shell git).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::Buffer;
use strop_git::memory::{self, BlameCard, ChangedFile, LogRow};
use strop_git::{Hunk, LineOrigin};

use super::{Editor, Key, Mode};

/// What a readonly buffer is — drives Enter/q and per-row rendering.
#[derive(Debug, Clone)]
pub enum Surface {
    CommitLog {
        rows: Vec<LogRow>,
        return_to: Option<ReturnPoint>,
    },
    ChangedFiles {
        sha: String,
        files: Vec<ChangedFile>,
        return_to: Option<ReturnPoint>,
    },
    /// A diff as a readonly buffer (0010 §2): the file's delta at a
    /// commit, or the `Space g p` hunk preview. The buffer's rows mirror
    /// the rendered layout — a stats row, then per hunk a `@@` header
    /// row and unprefixed content rows — so motions, `/` and yank see
    /// exactly what's on screen. `origin` names the working buffer a
    /// hunk preview belongs to, so `Space g u`/`g s` act on the file.
    Diff {
        /// Stats-row label: the file path (delta view) or "hunk".
        label: String,
        hunks: Vec<Hunk>,
        added: usize,
        deleted: usize,
        origin: Option<HunkOrigin>,
        return_to: Option<ReturnPoint>,
    },
}

/// Where a surface was opened from: closing it hands the cursor and
/// view back to that buffer (vim's window-close behavior — without
/// this, `q` dumps you on line 1).
#[derive(Debug, Clone)]
pub struct ReturnPoint {
    pub buffer: usize,
    pub cursor: usize,
    pub view_top: usize,
}

impl Surface {
    fn set_return_point(&mut self, ret: ReturnPoint) {
        *self.return_slot() = Some(ret);
    }

    pub(crate) fn return_point(&self) -> Option<&ReturnPoint> {
        match self {
            Surface::CommitLog { return_to, .. }
            | Surface::ChangedFiles { return_to, .. }
            | Surface::Diff { return_to, .. } => return_to.as_ref(),
        }
    }

    fn return_slot(&mut self) -> &mut Option<ReturnPoint> {
        match self {
            Surface::CommitLog { return_to, .. }
            | Surface::ChangedFiles { return_to, .. }
            | Surface::Diff { return_to, .. } => return_to,
        }
    }
}

/// Where a hunk preview came from: the buffer it undoes/stages in, at
/// the edit epoch it was captured. Edits since then invalidate it —
/// applying a stale region would cut the wrong lines.
#[derive(Debug, Clone)]
pub struct HunkOrigin {
    pub buffer: usize,
    pub epoch: u64,
}

/// Jobs post results here; the event loop drains (never blocks input).
pub enum GitJob {
    Log { buffer: usize, rows: Vec<LogRow> },
    Blame(Box<BlameCard>),
    Error(String),
}

impl Editor {
    pub fn surface(&self) -> Option<&Surface> {
        self.surfaces.get(self.current).and_then(|s| s.as_ref())
    }
    // ---- surface lifecycle --------------------------------------------

    fn push_surface(&mut self, name: Option<&str>, text: &str, mut surface: Surface) {
        // surfaces stack: only the first one opened from a plain buffer
        // carries a return point (closing the deepest unwinds the chain)
        if self.surface().is_none() {
            surface.set_return_point(ReturnPoint {
                buffer: self.current,
                cursor: self.cursor,
                view_top: self.view_top,
            });
        }
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

    /// A diff surface from structured hunks (0010 §2). `label` heads the
    /// stats row; `origin` is set only for working-tree hunk previews.
    pub(crate) fn open_diff_surface(
        &mut self,
        name: &str,
        label: &str,
        hunks: Vec<Hunk>,
        origin: Option<HunkOrigin>,
    ) {
        let (added, deleted) = hunk_stats(&hunks);
        let mut text = format!("{label} +{added} -{deleted}\n");
        for hunk in &hunks {
            text.push_str(&hunk.header());
            text.push('\n');
            for line in &hunk.lines {
                text.push_str(&line.text);
                text.push('\n');
            }
        }
        self.push_surface(
            Some(name),
            &text,
            Surface::Diff {
                label: label.to_string(),
                hunks,
                added,
                deleted,
                origin,
                return_to: None,
            },
        );
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
            Surface::CommitLog {
                rows: vec![],
                return_to: None,
            },
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

    /// Keys for readonly surface buffers (0001 §3): q closes, Enter
    /// dives, and everything else goes through the shared grammar
    /// resolver — motions and yank resolve, mutations refuse. The
    /// resolver is the source of truth, not a hand-maintained motion
    /// whitelist (0010 §6).
    pub(crate) fn feed_readonly(&mut self, key: Key) {
        if !self.pending.is_empty() {
            return self.feed_pending_readonly(key);
        }
        match key {
            Key::Char('q') => {
                self.close_buffer(true);
            }
            Key::Enter => self.dive(),
            Key::Char('v') => {
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
            }
            Key::Char(c) => {
                // multi-char heads wait for their second key; the rest
                // parse immediately (Invalid clears, Incomplete waits)
                self.pending.push(c);
                if !matches!(c, ' ' | ':' | 'g' | 'y' | ']' | '[') {
                    self.resolve_pending_readonly();
                }
            }
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
                self.message = "readonly — q closes, enter dives".into();
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
            Some(Surface::CommitLog { rows, .. }) => {
                let Some(sha) = rows.get(line).and_then(|r| r.sha.clone()) else {
                    return;
                };
                let Some(repo) = &self.git else { return };
                match memory::show_stat(repo.workdir(), &sha) {
                    Ok(files) => {
                        let mut text = format!("commit {}\n\n", &sha[..10.min(sha.len())]);
                        for f in &files {
                            text.push_str(&f.path.display().to_string());
                            text.push('\n');
                        }
                        self.push_surface(
                            Some("commit files"),
                            &text,
                            Surface::ChangedFiles {
                                sha,
                                files,
                                return_to: None,
                            },
                        );
                    }
                    Err(e) => self.message = e,
                }
            }
            Some(Surface::ChangedFiles { sha, files, .. }) => {
                // row 0/1 are the header
                let Some(file) = line.checked_sub(2).and_then(|i| files.get(i)) else {
                    return;
                };
                let Some(repo) = &self.git else { return };
                match repo.commit_file_diff(&sha, &file.path) {
                    Ok(diff) => self.open_diff_surface(
                        "delta",
                        &file.path.display().to_string(),
                        diff.hunks,
                        None,
                    ),
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
                        if let Some(Some(Surface::CommitLog { rows: slot, .. })) =
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

/// Added/deleted counts across hunks.
fn hunk_stats(hunks: &[Hunk]) -> (usize, usize) {
    hunks.iter().fold((0, 0), |(a, d), h| {
        let adds = h
            .lines
            .iter()
            .filter(|l| l.origin == LineOrigin::Addition)
            .count();
        let dels = h
            .lines
            .iter()
            .filter(|l| l.origin == LineOrigin::Deletion)
            .count();
        (a + adds, d + dels)
    })
}

/// The git job channel ends (created once in `Editor::new`).
pub fn git_channel() -> (Sender<GitJob>, Receiver<GitJob>) {
    channel()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use crate::editor::{Editor, Key, Surface};
    use strop_core::Buffer;

    use strop_git::LineOrigin;

    /// Repo with two commits; second adds a line to f.rs.
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
                |s| matches!(s, crate::editor::Surface::CommitLog { rows, .. } if !rows.is_empty()),
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
        assert!(matches!(e.surface(), Some(Surface::ChangedFiles { .. })));

        // Enter on the file row → the diff surface
        e.feed_text("j");
        e.feed_text("j");
        e.feed(Key::Enter);
        let text = e.buf().rope.to_string();
        assert!(text.contains("fn b() {}"), "{text}");
        assert!(text.starts_with("f.rs +1 -0\n"), "{text}");
        assert!(!text.contains("diff --git"), "no raw patch noise: {text}");
        assert!(text.contains("@@ -1,1 +1,2 @@"), "hunk header row: {text}");

        // edits refuse, q climbs out
        e.feed_text("x");
        assert!(e.message.contains("readonly"));
        e.feed_text("q");
        assert!(matches!(e.surface(), Some(Surface::ChangedFiles { .. })));
    }

    #[test]
    fn diff_surface_rows_carry_line_numbers() {
        let (_d, mut e) = fixture();
        e.open_log(false);
        pump(&mut e);
        e.feed_text("k"); // newest commit is row 0? feed j then k lands on 0
        e.feed(Key::Enter);
        e.feed_text("jj");
        e.feed(Key::Enter);
        let Some(Surface::Diff { hunks, .. }) = e.surface() else {
            panic!("not a diff surface");
        };
        let h = &hunks[0];
        let ctx = h
            .lines
            .iter()
            .find(|l| l.origin == LineOrigin::Context)
            .expect("context line");
        assert_eq!((ctx.old_lineno, ctx.new_lineno), (Some(1), Some(1)));
        let add = h
            .lines
            .iter()
            .find(|l| l.origin == LineOrigin::Addition)
            .expect("addition");
        assert_eq!(add.new_lineno, Some(2));
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
