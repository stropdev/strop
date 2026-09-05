//! Git memory surfaces (M3, reworked 0010): commit browser, changed-files
//! dive, diff view, blame card, permalinks. Every surface is a real
//! readonly buffer (0001 §3: motions, /, yank work); jobs post onto the
//! event loop (0001 §5.6: no blocking the input path on shell git).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::Buffer;
use strop_git::memory::{self, BlameCard, BlameLine, ChangedFile, LogRow};
use strop_git::{Hunk, LineOrigin};

use super::{Document, Editor, Key, Mode};

/// What a readonly buffer is — drives Enter/q and per-row rendering.
#[derive(Debug, Clone)]
pub enum Surface {
    CommitLog {
        rows: Vec<LogRow>,
        /// Sha to land the cursor on once rows arrive (the blame dive
        /// opens the browser *at* a commit, 0011 §3).
        focus: Option<String>,
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
    /// hunk preview belongs to, so `Space g u`/`g s` act on the file;
    /// `commit` carries the commit's other files when this delta came
    /// from the dive chain (the sidebar + `]f`/`[f`, 0011 §4).
    Diff {
        /// Stats-row label: the file path (delta view) or "hunk".
        label: String,
        hunks: Vec<Hunk>,
        added: usize,
        deleted: usize,
        origin: Option<HunkOrigin>,
        commit: Option<CommitFiles>,
        /// tuicr-style: Tab moves focus between the file sidebar and
        /// the diff content (j/k step files when the sidebar has focus).
        sidebar_focus: bool,
        return_to: Option<ReturnPoint>,
    },
}

/// The commit a Diff surface's file belongs to, with the commit's full
/// changed-file list — the sidebar's data (typed numstat rows, the same
/// ones the changed-files surface renders from; 0011 §4).
#[derive(Debug, Clone)]
pub struct CommitFiles {
    pub sha: String,
    pub files: Vec<ChangedFile>,
}

/// Where a surface was opened from: closing it hands the cursor and
/// view back to that buffer (vim's window-close behavior — without
/// this, `q` dumps you on line 1).
#[derive(Debug, Clone)]
pub struct ReturnPoint {
    pub buffer: strop_core::id::DocumentId,
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
    pub buffer: strop_core::id::DocumentId,
    pub epoch: u64,
}

/// Per-buffer blame gutter state (0011 §3), keyed by canonical path —
/// no parallel vector to keep aligned, and index churn can never pair
/// one buffer with another's blame. Valid only while the buffer's edit
/// epoch and line count still match the capture.
#[derive(Debug, Clone)]
pub struct BlameGutter {
    pub lines: Vec<BlameLine>,
    /// Buffer edit epoch when the blame was captured; any edit since
    /// invalidates the line↔buffer-line pairing.
    pub epoch: u64,
}

/// Jobs post results here; the event loop drains (never blocks input).
/// Index-carrying jobs carry the buffer-list `generation` they were
/// spawned under — a dead surface cannot be resurrected by index reuse
/// (0011 §2).
pub enum GitJob {
    Log {
        buffer: strop_core::id::DocumentId,
        generation: u64,
        rows: Vec<LogRow>,
    },
    Card {
        generation: u64,
        card: Box<BlameCard>,
    },
    Gutter {
        path: PathBuf,
        generation: u64,
        lines: Vec<BlameLine>,
    },
    Error(String),
}

impl Editor {
    pub fn surface(&self) -> Option<&Surface> {
        self.cur().surface.as_ref()
    }
    // ---- surface lifecycle --------------------------------------------
    pub(crate) fn push_surface(&mut self, name: Option<&str>, text: &str, mut surface: Surface) {
        self.drop_stale_scratch();
        // surfaces stack: only the first one opened from a plain buffer
        // carries a return point (closing the deepest unwinds the chain)
        if self.surface().is_none() {
            surface.set_return_point(ReturnPoint {
                buffer: self.current(),
                cursor: self.head(),
                view_top: self.view_top(),
            });
        }
        let mut buf = Buffer::from_text(text);
        buf.readonly = true;
        buf.name = name.map(|n| n.to_string());
        // surfaces render via delta/plain rules: no tree-sitter
        let id = self.docs.insert(Document {
            buf,
            highlighter: None,
            surface: Some(surface),
        });
        self.push_jump(); // opening a surface is a jumplist entry
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
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
        self.open_delta(name, label, hunks, origin, None);
    }

    /// The diff-surface builder: `commit` rides along when the delta
    /// came from the dive chain (sidebar + `]f`/`[f`, 0011 §4).
    pub(crate) fn open_delta(
        &mut self,
        name: &str,
        label: &str,
        hunks: Vec<Hunk>,
        origin: Option<HunkOrigin>,
        commit: Option<CommitFiles>,
    ) {
        let (added, deleted) = hunk_stats(&hunks);
        let text = diff_surface_text(label, &hunks);
        self.push_surface(
            Some(name),
            &text,
            Surface::Diff {
                label: label.to_string(),
                hunks,
                added,
                deleted,
                origin,
                commit,
                sidebar_focus: false,
                return_to: None,
            },
        );
        // syntax highlighting under the origin tint (delta's look):
        // the label is the file path for commit deltas; "hunk" and
        // friends resolve to None and keep origin colors
        if let Some(hl) = strop_syntax::Highlighter::for_path(label) {
            self.cur_mut().highlighter = Some(hl);
        }
    }

    /// `Space g l`: commit browser. `Space g h`: log scoped to the file.
    pub(crate) fn open_log(&mut self, file_scoped: bool) {
        self.open_log_inner(file_scoped, None, None);
    }

    /// Open the commit browser *at* a commit — the blame dive lands on
    /// the row it was asked about (0011 §3), not the newest entry.
    pub(crate) fn open_log_at(&mut self, sha: &str) {
        self.open_log_inner(false, Some(sha.to_string()), None);
    }

    /// `Space g h` in visual mode: the history of the selected lines
    /// (git log -L) — selection archaeology (0014 wave 4).
    pub(crate) fn open_line_history(&mut self, start: usize, end: usize) {
        self.open_log_inner(true, None, Some((start, end)));
    }

    fn open_log_inner(
        &mut self,
        file_scoped: bool,
        focus: Option<String>,
        range: Option<(usize, usize)>,
    ) {
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
            Some(if range.is_some() {
                "git log ·lines"
            } else if file_scoped {
                "git log ·file"
            } else {
                "git log"
            }),
            "loading log…",
            Surface::CommitLog {
                rows: vec![],
                focus,
                return_to: None,
            },
        );
        let idx = self.current();
        let generation = self.generation;
        let tx = self.git_tx.clone();
        std::thread::spawn(move || {
            let msg = match memory::log_graph_range(&workdir, 200, file.as_deref(), range) {
                Ok(rows) => GitJob::Log {
                    buffer: idx,
                    generation,
                    rows,
                },
                Err(e) => GitJob::Error(e),
            };
            let _ = tx.send(msg);
        });
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
                self.close_surface();
            }
            // C-w works from surfaces too: splits are core grammar
            Key::CtrlW => self.pending = "\x17".into(),
            Key::CtrlO => self.jump_back(),
            // tuicr's tab: focus hops between the file sidebar and the
            // diff content; everywhere else Tab walks the jumplist
            Key::Tab | Key::Backtab => {
                let has_sidebar = matches!(
                    self.surface(),
                    Some(Surface::Diff {
                        commit: Some(_),
                        ..
                    })
                );
                if has_sidebar {
                    self.toggle_sidebar_focus();
                } else {
                    self.jump_forward();
                }
            }
            Key::Char('j') if self.sidebar_focused() => self.commit_file_step(true),
            Key::Char('k') if self.sidebar_focused() => self.commit_file_step(false),
            Key::Enter if self.sidebar_focused() => self.toggle_sidebar_focus(),
            Key::Enter => self.dive(),
            // arrows speak hjkl on surfaces too (sidebar-aware)
            Key::Up => {
                if self.sidebar_focused() {
                    self.commit_file_step(false);
                } else {
                    self.run_motion("k");
                }
            }
            Key::Down => {
                if self.sidebar_focused() {
                    self.commit_file_step(true);
                } else {
                    self.run_motion("j");
                }
            }
            Key::Left => self.run_motion("h"),
            Key::Right => self.run_motion("l"),
            // searches repeat on surfaces too (diff preview power tools)
            Key::Char('n') => self.repeat_search(false),
            Key::Char('N') => self.repeat_search(true),
            Key::Char('v') => {
                self.mode = Mode::Visual;
                let h = self.head();
                self.sels_mut().stretch_primary(h, h);
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
                // window commands (C-w): h l j k w move, v s split,
                // q closes the pane-or-surface (0011 §1)
                if self.pending == "\x17" {
                    self.pending.clear();
                    return match c {
                        'h' | 'l' | 'j' | 'k' | 'w' => self.pane_move(c),
                        'v' => self.split(true, None),
                        's' => self.split(false, None),
                        'q' => self.close_surface(),
                        _ => self.message = "C-w: h l j k w move · v s split · q close".into(),
                    };
                }
                if self.pending == " g" {
                    return self.feed_git_pending(c);
                }
                if (self.pending == "]" || self.pending == "[") && (c == 'c' || c == 'f') {
                    let forward = self.pending == "]";
                    self.pending.clear();
                    return if c == 'c' {
                        self.jump_hunk(forward)
                    } else {
                        self.commit_file_step(forward)
                    };
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
        if let Some(r) = strop_grammar::resolve(self.buf(), self.head(), cmd) {
            let text = self.buf().slice_string(r.range);
            self.set_register(cmd.register, text, r.range.is_linewise());
            self.flash(r.range);
        }
    }

    /// `q`: pop one surface (0011 §1). In a split the *pane* closes —
    /// the buffer stays, vim `:q` semantics — and only the last pane's
    /// close closes the buffer, running the guaranteed return-point
    /// restore.
    fn close_surface(&mut self) {
        self.close_pane_or_buffer(true);
        let doc = self.current();
        if let Some(pane) = self.panes.get_mut(self.active_pane) {
            pane.doc = doc; // the pane follows the successor
        }
    }

    // ---- job drain ------------------------------------------------------

    pub fn drain_git_jobs(&mut self) {
        while let Ok(job) = self.git_rx.try_recv() {
            match job {
                GitJob::Log {
                    buffer,
                    generation,
                    rows,
                } => {
                    // a closed surface's index may be recycled by the
                    // next buffer: only same-generation results land
                    // (0011 §2)
                    if generation != self.generation || self.docs.get(buffer).is_none() {
                        continue;
                    }
                    let text = rows
                        .iter()
                        .map(|r| r.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                        + "\n";
                    self.doc_mut(buffer).buf.replace_all_system(&text);
                    let mut focus_row = None;
                    if let Some(Some(Surface::CommitLog {
                        rows: slot, focus, ..
                    })) = self.docs.get_mut(buffer).map(|d| &mut d.surface)
                    {
                        focus_row = focus.take().and_then(|sha| {
                            rows.iter().position(|r| r.sha.as_deref() == Some(&sha))
                        });
                        *slot = rows;
                    }
                    if let Some(row) = focus_row {
                        // the blame dive asked for this commit: land on
                        // it (only when the browser is still what's
                        // being driven)
                        if self.current() == buffer {
                            self.set_head(self.doc(buffer).buf.line_start(row));
                            self.view_mut().view_top = row;
                        }
                    }
                }
                GitJob::Card { generation, card } => {
                    if generation == self.generation {
                        self.blame_card = Some(*card);
                    }
                }
                GitJob::Gutter {
                    path,
                    generation,
                    lines,
                } => {
                    // toggled off meanwhile → the entry is gone → drop
                    if generation != self.generation {
                        continue;
                    }
                    if let Some(gutter) = self.blame_gutters.get_mut(&path) {
                        gutter.lines = lines;
                        // the gutter supersedes the card that covered
                        // the load for this buffer
                        if self
                            .buf()
                            .path
                            .as_deref()
                            .is_some_and(|p| self.blame_key_of(p) == path)
                        {
                            self.blame_card = None;
                        }
                    }
                }
                GitJob::Error(e) => self.message = e,
            }
        }
    }
}

/// Added/deleted counts across hunks.
pub(crate) fn hunk_stats(hunks: &[Hunk]) -> (usize, usize) {
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

/// The buffer text a diff surface shows: stats row, then per hunk a
/// header row and unprefixed content rows — exactly the rendered
/// layout (0010 §2).
pub(crate) fn diff_surface_text(label: &str, hunks: &[Hunk]) -> String {
    let (added, deleted) = hunk_stats(hunks);
    let mut text = format!("{label} +{added} -{deleted}\n");
    for hunk in hunks {
        text.push_str(&hunk.header());
        text.push('\n');
        for line in &hunk.lines {
            text.push_str(&line.text);
            text.push('\n');
        }
    }
    text
}

/// The git job channel ends (created once in `Editor::new`).
pub fn git_channel() -> (Sender<GitJob>, Receiver<GitJob>) {
    channel()
}

impl Editor {
    /// Table shim (0008 stage 2).
    pub(crate) fn open_log_pub(&mut self, file_scoped: bool) {
        self.open_log(file_scoped);
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use crate::editor::{Editor, GitJob, Key, Surface};
    use strop_core::Buffer;
    use strop_git::memory::LogRow;
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
        assert_eq!(e.build_permalink().unwrap_err(), "no remote configured");
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

    fn git_out(root: &std::path::Path, args: &[&str]) -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string()
    }

    fn pump_ready(e: &mut Editor, ready: impl Fn(&Editor) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !ready(e) && std::time::Instant::now() < deadline {
            e.drain_git_jobs();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// `Space g b` toggles a per-buffer gutter; Enter dives into the
    /// cursor line's commit, positioned at its sha (0011 §3).
    #[test]
    fn blame_gutter_toggles_and_dives() {
        let (dir, mut e) = fixture();
        let root = dir.path().to_path_buf();
        e.feed_text(" gb");
        assert_eq!(e.blame_gutters.len(), 1, "gutter on for the buffer");
        pump_ready(&mut e, |e| e.blame_gutter_for(e.first_doc()).is_some());
        let gutter = e
            .blame_gutter_for(e.first_doc())
            .expect("gutter data loaded");
        assert_eq!(gutter.lines.len(), 2, "one blame per file line");
        assert_eq!(
            gutter.lines[0].sha,
            git_out(&root, &["rev-parse", "HEAD~1"])
        );
        assert_eq!(gutter.lines[1].sha, git_out(&root, &["rev-parse", "HEAD"]));

        // cursor on line 1 → Enter dives into "first", landing on its row
        e.feed(Key::Enter);
        pump_ready(&mut e, |e| {
            e.surface().is_some_and(
                |s| matches!(s, crate::editor::Surface::CommitLog { rows, .. } if !rows.is_empty()),
            )
        });
        assert!(
            matches!(e.surface(), Some(Surface::CommitLog { .. })),
            "dive opened the browser"
        );
        assert_eq!(
            e.buf().line_of(e.head()),
            1,
            "cursor on the first-commit row"
        );
        assert_eq!(e.view_top(), 1, "view positioned at the focused sha");
        let text = e.buf().rope.to_string();
        assert!(text.contains("first"), "{text}");

        // q returns; the gutter survives; toggle off removes it
        e.feed_text("q");
        assert_eq!(e.blame_gutters.len(), 1, "gutter is per-buffer view state");
        e.feed_text(" gb");
        assert!(e.blame_gutters.is_empty(), "second toggle turns it off");
        e.feed(Key::Enter);
        assert!(
            !matches!(e.surface(), Some(Surface::CommitLog { .. })),
            "Enter without a gutter stays inert"
        );
    }

    /// The gutter refuses to dive after edits (stale pairing) and falls
    /// back to the single-line card (0011 §3).
    #[test]
    fn stale_gutter_falls_back_to_card() {
        let (_d, mut e) = fixture();
        e.feed_text(" gb");
        // settle both spawned jobs (gutter + interim card): a sentinel
        // through the same FIFO channel proves everything before it
        // was delivered
        e.git_tx.send(GitJob::Error("\u{0}settled".into())).unwrap();
        pump_ready(&mut e, |e| e.message.contains('\u{0}'));
        e.message.clear();
        e.blame_card = None;
        // edit the buffer: line count changes, epoch bumps. Save so
        // the disk-blame card can speak about the new line at all
        e.feed_text("o");
        e.feed_text("fn c() {}");
        e.feed(Key::Esc);
        e.feed_text(":w<cr>");
        assert!(
            e.blame_gutter_for(e.first_doc()).is_none(),
            "edits void the line↔blame pairing"
        );
        e.blame_card = None;
        e.feed(Key::Enter);
        assert!(
            !matches!(e.surface(), Some(Surface::CommitLog { .. })),
            "no dive from stale data"
        );
        // the card is the fallback: it blames the cursor's own line
        // (wait for the *new* card — the toggle's line-1 card may
        // still be in flight)
        pump_ready(&mut e, |e| {
            e.blame_card.as_ref().is_some_and(|c| c.line == 3)
        });
    }

    /// The return point restores even when the origin buffer is not
    /// the one the close would land on next (0011 §1).
    #[test]
    fn return_point_restores_when_origin_not_current() {
        let (dir, mut e) = fixture();
        let root = dir.path();
        e.feed_text("j$"); // line 2, end
        let want = e.head();
        e.open_log(false);
        pump(&mut e);
        std::fs::write(root.join("g.rs"), "other\n").unwrap();
        let origin = e.first_doc();
        e.open_buffer(root.join("g.rs").to_str().unwrap()).unwrap();
        assert_ne!(e.current(), origin, "switched away from the log's origin");
        let log_surface = e.mru.iter().copied().find(|&id| {
            e.doc(id)
                .buf
                .name
                .as_deref()
                .is_some_and(|n| n.contains("log"))
        });
        e.view_mut().doc = log_surface.expect("log surface in mru"); // back onto the log surface
        e.set_head(0);
        e.feed_text("q");
        assert_eq!(e.current(), origin, "closing switches back to the origin");
        assert_eq!(e.head(), want, "cursor restored, not line 1");
        assert_eq!(e.buf().line_of(e.head()), 1);
    }

    /// A log result for a dead surface cannot land in the buffer that
    /// recycled its index (0011 §2).
    #[test]
    fn stale_log_results_are_dropped() {
        let (_d, mut e) = fixture();
        e.open_log(false);
        pump(&mut e);
        let stale = e.generation;
        let dead_surface = e.current(); // the log surface's id
        e.feed_text("q"); // closes the surface; generation moves on
        assert_ne!(stale, e.generation);
        e.git_tx
            .send(GitJob::Log {
                buffer: dead_surface,
                generation: stale,
                rows: vec![LogRow {
                    text: "POISON ROW".into(),
                    sha: None,
                }],
            })
            .unwrap();
        e.drain_git_jobs();
        for (i, (_, d)) in e.docs.iter().enumerate() {
            let text = d.buf.rope.to_string();
            assert!(!text.contains("POISON"), "document {i} clobbered: {text}");
        }
        // the live path still delivers
        e.open_log(false);
        pump(&mut e);
        assert!(e.buf().rope.to_string().contains("add b"));
    }

    /// A late gutter result for a toggled-off buffer is dropped: the
    /// entry is the toggle, not the job (0011 §2).
    #[test]
    fn gutter_result_dropped_after_toggle_off() {
        let (dir, mut e) = fixture();
        let key = dir.path().join("f.rs").canonicalize().unwrap();
        e.feed_text(" gb"); // on (job in flight)
        e.feed_text(" gb"); // off
        assert!(e.blame_gutters.is_empty());
        e.git_tx
            .send(GitJob::Gutter {
                path: key,
                generation: e.generation, // even a current generation
                lines: vec![strop_git::memory::BlameLine {
                    sha: "deadbeef".into(),
                    author: "nobody".into(),
                    age: "1m".into(),
                    ts: 0,
                }],
            })
            .unwrap();
        e.drain_git_jobs();
        assert!(
            e.blame_gutters.is_empty(),
            "a late job must not re-open a closed gutter"
        );
    }

    /// Two files in one fixture repo; the second commit touches both.
    fn multi_file_fixture() -> (tempfile::TempDir, Editor) {
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
        std::fs::write(root.join("a.rs"), "one\n").unwrap();
        std::fs::write(root.join("b.rs"), "uno\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(root.join("a.rs"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("b.rs"), "uno\ndos\n").unwrap();
        git(&["commit", "-qam", "touch both"]);
        let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.discover_git();
        (dir, e)
    }

    /// Dive to a file delta: the surface carries the commit's files,
    /// and `]f`/`[f` walk them, wrapping (0011 §4).
    #[test]
    fn commit_file_nav_walks_files() {
        let (_d, mut e) = multi_file_fixture();
        e.open_log(false);
        pump(&mut e);
        e.feed(Key::Enter); // newest commit → changed files
        e.feed_text("jj");
        e.feed(Key::Enter); // a.rs → delta
        let (label, files) = match e.surface() {
            Some(Surface::Diff {
                label,
                commit: Some(cf),
                ..
            }) => (label.clone(), cf.files.len()),
            other => panic!("not a commit diff: {other:?}"),
        };
        assert_eq!(label, "a.rs");
        assert_eq!(files, 2, "the sidebar's data rides the surface");

        e.feed_text("]f");
        match e.surface() {
            Some(Surface::Diff { label, .. }) => assert_eq!(label, "b.rs"),
            other => panic!("surface lost: {other:?}"),
        }
        let text = e.buf().rope.to_string();
        assert!(text.starts_with("b.rs +1 -0\n"), "{text}");
        assert!(text.contains("dos"), "{text}");
        assert!(e.message.contains("b.rs · 2/2"), "{}", e.message);

        e.feed_text("[f");
        assert!(
            matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "a.rs"),
            "back to the first file"
        );
        e.feed_text("[f"); // wraparound
        assert!(
            matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "b.rs"),
            "wraparound to the last file"
        );
        assert_eq!(
            e.docs.len(),
            4,
            "]f rewrites the surface in place (no new buffers)"
        );
    }

    /// Tab hops focus between sidebar and diff; focused j/k steps
    /// files (tuicr's model); Enter hops back (0011 §4).
    #[test]
    fn tab_cycles_focus_between_sidebar_and_diff() {
        let (_d, mut e) = multi_file_fixture();
        e.open_log(false);
        pump(&mut e);
        e.feed(Key::Enter); // changed files
        e.feed_text("jj");
        e.feed(Key::Enter); // a.rs delta
        assert!(!e.sidebar_focused());

        e.feed(crate::editor::Key::Tab);
        assert!(e.sidebar_focused(), "tab focuses the sidebar");
        e.feed_text("j"); // focused j steps to the next file
        assert!(
            matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "b.rs"),
            "j stepped to b.rs"
        );
        assert!(e.sidebar_focused(), "focus survives the file step");
        e.feed(crate::editor::Key::Enter);
        assert!(!e.sidebar_focused(), "enter hops back to the diff");
        e.feed(crate::editor::Key::Backtab);
        assert!(e.sidebar_focused(), "shift-tab focuses too");
    }

    /// `q` in a split closes the pane (buffer stays); the last pane's
    /// `q` closes the buffer and restores the origin (0011 §1).
    #[test]
    fn q_in_split_closes_pane_then_buffer() {
        let (_d, mut e) = fixture();
        e.open_log(false);
        pump(&mut e);
        e.feed(Key::CtrlW);
        e.feed_text("v"); // split: both panes show the log
        assert_eq!(e.panes.len(), 2);
        e.feed_text("q");
        assert_eq!(e.panes.len(), 1, "q closes the pane in a split");
        assert_eq!(e.docs.len(), 2, "the surface buffer survives");
        assert!(
            matches!(e.surface(), Some(Surface::CommitLog { .. })),
            "still on the log"
        );
        e.feed_text("q");
        assert_eq!(e.docs.len(), 1, "the last pane's q closes the buffer");
        assert_eq!(e.current(), e.first_doc(), "back on the origin buffer");
        assert!(e.surface().is_none());
    }

    /// Golden shape: the blame column renders per line; the commit
    /// sidebar renders beside the delta with the current file marked.
    #[test]
    fn gutters_and_sidebar_render() {
        let (dir, mut e) = fixture();
        let root = dir.path().to_path_buf();
        e.feed_text(" gb");
        pump_ready(&mut e, |e| e.blame_gutter_for(e.first_doc()).is_some());
        let frame = crate::headless::frame_string(&mut e, 100, 10);
        let first_sha = git_out(&root, &["rev-parse", "HEAD~1"]);
        assert!(
            frame.contains(&format!("{} t ", &first_sha[..7])),
            "blame cell: {frame}"
        );
        assert!(
            frame.contains("fn a() {}"),
            "content still renders right of the gutter: {frame}"
        );

        let (_d, mut e) = multi_file_fixture();
        e.open_log(false);
        pump(&mut e);
        e.feed(Key::Enter);
        e.feed_text("jj");
        e.feed(Key::Enter); // a.rs delta
        let frame = crate::headless::frame_string(&mut e, 100, 12);
        assert!(frame.contains("▌a.rs"), "current file marked: {frame}");
        assert!(frame.contains(" b.rs"), "sibling files listed: {frame}");
        e.feed_text("]f");
        let frame = crate::headless::frame_string(&mut e, 100, 12);
        assert!(frame.contains("▌b.rs"), "marker follows ]f: {frame}");
    }
}
