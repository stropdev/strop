//! Git working surface (M2, hunk preview reworked 0010): hunks between
//! HEAD and the live buffer, refreshed on edit epochs; hunk nav and the
//! hunk verbs.

use strop_git::{Hunk, HunkKind, Repo, Sign};

use super::git_memory::HunkOrigin;
use super::Editor;

/// What a hunk verb (`Space g u`/`g s`) targets from the current view.
enum HunkTarget {
    /// Not on a hunk surface: act on the cursor's own buffer.
    NotASurface,
    /// A hunk preview whose origin buffer still matches the epoch it
    /// was captured at.
    Fresh { buffer: usize, hunk: Hunk },
    /// The origin buffer changed since the preview opened — applying
    /// the stored region would cut the wrong lines.
    Stale,
}

impl Editor {
    /// Discover the repo for the current buffer (once per buffer switch).
    pub(crate) fn discover_git(&mut self) {
        let from = self
            .buf()
            .path
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.cwd.clone());
        self.git = Repo::discover(&from);
        self.hunks.clear();
        self.hunks_epoch = u64::MAX;
    }

    /// Recompute hunks when the buffer changed since the last diff.
    /// libgit2 in-memory diff — no process spawn per keystroke (0001 §3).
    pub fn refresh_hunks(&mut self) {
        let Some(repo) = &self.git else {
            return;
        };
        let epoch = self.buf().epoch;
        if epoch == self.hunks_epoch {
            return;
        }
        let path = self.buf().path.clone();
        self.hunks = match &path {
            Some(p) => repo.hunks(std::path::Path::new(p), &self.buf().rope.to_string()),
            None => vec![],
        };
        self.hunks_epoch = epoch;
    }

    /// Gutter sign for a 1-based buffer line: `+` add, `~` change,
    /// `-` deletion below (0001 pillar 3.1).
    pub fn sign_at(&self, line_1based: usize) -> Option<char> {
        let total = self.buf().len_lines();
        for h in &self.hunks {
            for (l, kind) in h.signs() {
                let matched = match kind {
                    Sign::AddOrChange => l == line_1based,
                    Sign::DeleteAfter => l.min(total) == line_1based,
                };
                if matched {
                    return Some(match kind {
                        Sign::DeleteAfter => '-',
                        Sign::AddOrChange => {
                            if h.kind == HunkKind::Add {
                                '+'
                            } else {
                                '~'
                            }
                        }
                    });
                }
            }
        }
        None
    }

    /// `]c` / `[c`: jump to the next/previous changed line.
    pub(crate) fn jump_hunk(&mut self, forward: bool) {
        self.refresh_hunks();
        let cur = self.buf().line_of(self.cursor) + 1;
        let total = self.buf().len_lines();
        let mut lines: Vec<usize> = self
            .hunks
            .iter()
            .flat_map(|h| h.signs().iter().map(|&(l, _)| l).collect::<Vec<_>>())
            .map(|l| l.min(total))
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let target = if forward {
            lines.iter().copied().find(|&l| l > cur)
        } else {
            lines.iter().copied().rev().find(|&l| l < cur)
        };
        match target {
            Some(l) => {
                self.cursor = self.buf().line_start(l - 1);
                self.clamp_cursor();
            }
            None => self.message = "no more hunks".into(),
        }
    }

    /// The hunk under the cursor, if any.
    fn hunk_under_cursor(&mut self) -> Option<Hunk> {
        self.refresh_hunks();
        let line = self.buf().line_of(self.cursor) + 1;
        let total = self.buf().len_lines();
        self.hunks.iter().find(|h| h.covers(line, total)).cloned()
    }

    /// Apply `hunk`'s reverse to buffer `idx`: pure deletions reinsert,
    /// pure additions drop, changes swap old content back. Returns
    /// false when the buffer has no HEAD content to restore from.
    fn restore_hunk_in(&mut self, idx: usize, hunk: &Hunk) -> bool {
        let Some(path) = self.buffers[idx].path.clone() else {
            return false;
        };
        let Some(repo) = &self.git else { return false };
        let Some(head) = repo.head_content(std::path::Path::new(&path)) else {
            return false;
        };
        let head_lines: Vec<&str> = head.lines().collect();
        let (new_first, new_count, old_first, old_count) = hunk.changed_region();
        // only computed when restoring (change/delete); pure adds need
        // none — and a top-of-file add has old_first == 0, so the `- 1`
        // math must stay saturating
        let old: String = if old_count == 0 {
            String::new()
        } else {
            let lo = old_first.saturating_sub(1).min(head_lines.len());
            let hi = (lo + old_count).min(head_lines.len());
            head_lines[lo..hi.max(lo)].join("\n")
        };

        let saved_current = self.current;
        self.current = idx;
        if new_count == 0 {
            // pure deletion: reinsert the old lines at the gap
            let total = self.buf().len_lines();
            if new_first > total {
                let end = self.buf().len_bytes();
                self.buf_mut().insert(end, &format!("\n{old}"));
            } else {
                let at = self.buf().line_start(new_first.saturating_sub(1));
                self.buf_mut().insert(at, &format!("{old}\n"));
            }
            self.cursor = self.buf().line_start(new_first.saturating_sub(1));
        } else if old_count == 0 {
            // pure addition: drop the added lines
            let start = self.buf().line_start(new_first - 1);
            let last = (new_first - 1 + new_count).min(self.buf().len_lines());
            let end = if last >= self.buf().len_lines() {
                self.buf().len_bytes()
            } else {
                self.buf().line_start(last)
            };
            self.buf_mut()
                .delete(strop_core::Range::charwise(start, end));
            self.cursor = self
                .buf()
                .line_start((new_first - 1).min(self.buf().len_lines() - 1));
        } else {
            let start = self.buf().line_start(new_first - 1);
            let end = self
                .buf()
                .line_end((new_first - 1 + new_count - 1).min(self.buf().len_lines() - 1));
            self.buf_mut()
                .delete(strop_core::Range::charwise(start, end));
            self.buf_mut().insert(start, &old);
            self.cursor = start;
        }
        self.current = saved_current;
        // the cursor field belongs to the driven pane; only the origin
        // buffer's own view moves when it is current
        if self.current == idx {
            self.clamp_cursor();
            self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
        } else if let Some(pane) = self.panes.iter_mut().find(|p| p.buffer == idx) {
            pane.cursor = self.buffers[idx].line_start(self.buffers[idx].len_lines() - 1);
        }
        true
    }

    /// `Space g u`: reset a hunk to HEAD's content. From the hunk
    /// surface it restores the origin buffer's hunk (0010 §2).
    pub(crate) fn undo_hunk(&mut self) {
        match self.hunk_surface_target() {
            HunkTarget::Fresh { buffer, hunk } => {
                if self.restore_hunk_in(buffer, &hunk) {
                    self.message = "hunk reset".into();
                }
            }
            HunkTarget::Stale => self.message = "buffer changed — reopen the hunk preview".into(),
            HunkTarget::NotASurface => {
                let Some(hunk) = self.hunk_under_cursor() else {
                    self.message = "no hunk here".into();
                    return;
                };
                if self.restore_hunk_in(self.current, &hunk) {
                    self.message = "hunk reset".into();
                }
            }
        }
    }

    /// `Space g s`: stage a hunk (git apply --cached). From the hunk
    /// surface it stages the origin buffer's hunk.
    pub(crate) fn stage_hunk(&mut self) {
        match self.hunk_surface_target() {
            HunkTarget::Fresh { buffer, hunk } => self.stage_hunk_in(buffer, &hunk),
            HunkTarget::Stale => self.message = "buffer changed — reopen the hunk preview".into(),
            HunkTarget::NotASurface => {
                let Some(hunk) = self.hunk_under_cursor() else {
                    self.message = "no hunk here".into();
                    return;
                };
                self.stage_hunk_in(self.current, &hunk);
            }
        }
    }

    fn stage_hunk_in(&mut self, idx: usize, hunk: &Hunk) {
        let Some(path) = self.buffers[idx].path.clone() else {
            return;
        };
        // staging reads the *disk* file's hunk; save first so the two agree
        if self.buffers[idx].dirty {
            let _ = self.buffers[idx].save();
        }
        let Some(repo) = &self.git else { return };
        let Ok(rel) = std::path::Path::new(&path)
            .strip_prefix(repo.workdir())
            .map(|p| p.to_path_buf())
        else {
            self.message = "buffer not under workdir".into();
            return;
        };
        match repo.stage_hunk(&rel, hunk) {
            Ok(()) => {
                self.hunks_epoch = u64::MAX;
                self.message = "hunk staged".into();
            }
            Err(e) => self.message = format!("stage failed: {e}"),
        }
    }

    /// `Space g p`: preview the hunk under the cursor as a diff surface
    /// (0010 §2) — a readonly buffer you can move in; `q` closes,
    /// `Space g u`/`g s` still act on the file.
    pub(crate) fn preview_hunk(&mut self) {
        let Some(hunk) = self.hunk_under_cursor() else {
            self.message = "no hunk here".into();
            return;
        };
        let origin = HunkOrigin {
            buffer: self.current,
            epoch: self.buffers[self.current].epoch,
        };
        self.open_diff_surface("hunk", "hunk", vec![hunk], Some(origin));
    }

    /// What a `Space g u`/`g s` from the current buffer should act on:
    /// the hunk surface's origin when fresh, a refusal when the origin
    /// buffer has moved on, and the cursor's own hunk otherwise.
    fn hunk_surface_target(&self) -> HunkTarget {
        let Some(super::Surface::Diff { hunks, origin, .. }) = self.surface() else {
            return HunkTarget::NotASurface;
        };
        let Some(origin) = origin else {
            return HunkTarget::NotASurface; // commit delta: nothing to undo
        };
        let Some(hunk) = hunks.first() else {
            return HunkTarget::NotASurface;
        };
        match self.buffers.get(origin.buffer) {
            Some(b) if b.epoch == origin.epoch => HunkTarget::Fresh {
                buffer: origin.buffer,
                hunk: hunk.clone(),
            },
            _ => HunkTarget::Stale,
        }
    }

    pub(crate) fn feed_git_pending(&mut self, c: char) {
        self.pending.clear();
        match c {
            'u' => self.undo_hunk(),
            's' => self.stage_hunk(),
            'p' => self.preview_hunk(),
            'l' => self.open_log(false),
            'h' => self.open_log(true),
            'b' => self.toggle_blame_gutter(),
            'y' => self.yank_permalink(),
            'o' => self.open_permalink(),
            _ => {
                self.message =
                    "Space g: l log · h file history · b blame · y/o permalink · u/s/p hunk".into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::editor::Key;
    use crate::editor::Surface;
    use strop_core::Buffer;

    /// A git repo with one committed file, edited in-memory.
    fn fixture() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.t"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
                .unwrap();
        }
        std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(root)
            .output()
            .unwrap();
        let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
        e.discover_git();
        (dir, e)
    }

    #[test]
    fn gutter_tracks_live_edits() {
        let (_d, mut e) = fixture();
        e.refresh_hunks();
        assert_eq!(e.sign_at(1), None, "clean buffer has no signs");
        e.feed_text("G");
        e.feed_text("ofn c() {}");
        e.feed_text("<esc>");
        e.refresh_hunks();
        assert_eq!(e.sign_at(3), Some('+'), "added line signs +");
        assert_eq!(e.sign_at(1), None);
    }

    #[test]
    fn hunk_nav_and_undo() {
        let (_d, mut e) = fixture();
        e.feed_text("Go");
        e.feed_text("fn c() {}");
        e.feed_text("<esc>");
        e.feed_text("gg");
        e.jump_hunk(true);
        assert_eq!(e.buf().line_of(e.cursor) + 1, 3, "]c lands on the hunk");
        e.undo_hunk();
        assert_eq!(e.buf().rope.to_string(), "fn a() {}\nfn b() {}\n");
    }

    #[test]
    fn space_g_namespace_dispatches() {
        let (_d, mut e) = fixture();
        e.feed_text("G");
        e.feed_text("o");
        for c in "fn new() {}".chars() {
            e.feed(Key::Char(c));
        }
        e.feed(Key::Esc);
        e.feed_text(" gp"); // Space, g, p
        assert!(
            matches!(e.surface(), Some(Surface::Diff { .. })),
            "Space g p opens the hunk surface (buffer: {})",
            e.buf().rope
        );
    }

    /// The hunk surface is a real readonly buffer you can move in, and
    /// ` g u` from it restores the origin buffer (0010 §2).
    #[test]
    fn hunk_surface_moves_and_undoes() {
        let (_d, mut e) = fixture();
        e.feed_text("Go");
        e.feed_text("fn c() {}");
        e.feed_text("<esc>");
        e.feed_text("]c"); // like the tape: jump onto the hunk first
        e.feed_text(" gp");
        assert!(e.buf().readonly);
        assert!(e.buf().rope.to_string().contains("fn c() {}"));
        // motions work on the hunk surface
        e.feed_text("j");
        e.feed_text("j");
        assert_eq!(e.buf().line_of(e.cursor), 2);
        // undo acts on the origin file buffer, not the surface
        e.feed_text(" gu");
        let text = e.buffers[0].rope.to_string();
        assert_eq!(text, "fn a() {}\nfn b() {}\n", "hunk restored: {text}");
        e.feed_text("q");
        assert_eq!(e.current, 0);
    }

    /// A stale preview refuses honestly: edits after opening it change
    /// the epoch, and applying the stored region would cut wrong.
    #[test]
    fn stale_hunk_surface_refuses() {
        let (_d, mut e) = fixture();
        e.feed_text("Go");
        e.feed_text("fn c() {}");
        e.feed_text("<esc>gg]c gp");
        // edit the origin buffer: the epoch moves, the preview goes stale
        e.panes[0].buffer = 0; // ensure a pane points at the file
        e.buffers[0].insert(0, "// touched\n");
        e.feed_text(" gu");
        assert!(
            e.message.contains("buffer changed"),
            "stale preview must refuse: {}",
            e.message
        );
    }
}
