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
                // `:` opens the modal ex line (0003 §1): the text owns
                // every later key until Enter/Esc — never per-char
                // resolve (:set noro died here)
                if self.pending.starts_with(':') {
                    self.pending.push(c);
                    return;
                }
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
        loop {
            let next = self.git_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            match next {
                Some(job) => self.handle_git_job(job),
                None => break,
            }
        }
    }

    /// One git job result (TUI events land here directly — 0018).
    pub(crate) fn handle_git_job(&mut self, job: GitJob) {
        {
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
                        return;
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
                        return;
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
            text.push_str(&line.text_str());
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
mod tests;
