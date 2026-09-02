//! Git working surface (M2, 0001 pillar 3.1): hunks between HEAD and the
//! live buffer, refreshed on edit epochs; hunk nav and the hunk verbs.

use strop_git::{Hunk, HunkKind, Repo, Sign};

use super::Editor;

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
        self.hunks_epoch = u64::MAX; // force refresh
    }

    /// Recompute hunks when the buffer changed since the last diff.
    /// libgit2 in-memory diff — no process spawn per keystroke (0001 §3).
    pub fn refresh_hunks(&mut self) {
        let Some(repo) = &self.git else { return };
        let epoch = self.buf().epoch;
        if epoch == self.hunks_epoch {
            return;
        }
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let content = self.buf().rope.to_string();
        self.hunks = repo.hunks(std::path::Path::new(&path), &content);
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

    /// `Space g u`: reset the hunk under the cursor to HEAD's content.
    pub(crate) fn undo_hunk(&mut self) {
        let Some(hunk) = self.hunk_under_cursor() else {
            self.message = "no hunk here".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let Some(repo) = &self.git else { return };
        let Some(head) = repo.head_content(std::path::Path::new(&path)) else {
            self.message = "untracked — nothing to restore from".into();
            return;
        };
        let head_lines: Vec<&str> = head.lines().collect();
        let (new_first, new_count, old_first, old_count) = hunk.changed_region();
        // only computed when restoring (change/delete); pure adds need none
        let old: String = {
            let lo = old_first.saturating_sub(1).min(head_lines.len());
            let hi = (old_first - 1 + old_count).min(head_lines.len());
            head_lines[lo..hi].join("\n")
        };

        if new_count == 0 {
            // pure deletion: reinsert the old lines at the gap
            let total = self.buf().len_lines();
            if new_first > total {
                let end = self.buf().len_bytes();
                self.buf_mut().insert(end, &format!("\n{old}"));
            } else {
                let at = self.buf().line_start(new_first - 1);
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
        self.clamp_cursor();
        self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
        self.message = "hunk reset".into();
    }

    /// `Space g s`: stage the hunk under the cursor (git apply --cached).
    pub(crate) fn stage_hunk(&mut self) {
        let Some(hunk) = self.hunk_under_cursor() else {
            self.message = "no hunk here".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        // staging reads the *disk* file's hunk; save first so the two agree
        if self.buf().dirty {
            let _ = self.buf_mut().save();
        }
        let Some(repo) = &self.git else { return };
        let Ok(rel) = std::path::Path::new(&path)
            .strip_prefix(repo.workdir())
            .map(|p| p.to_path_buf())
        else {
            self.message = "buffer not under workdir".into();
            return;
        };
        match repo.stage_hunk(&rel, &hunk) {
            Ok(()) => {
                self.hunks_epoch = u64::MAX;
                self.message = "hunk staged".into();
            }
            Err(e) => self.message = format!("stage failed: {e}"),
        }
    }

    /// `Space g p`: preview the hunk under the cursor in a floating card.
    pub(crate) fn preview_hunk(&mut self) {
        match self.hunk_under_cursor() {
            Some(hunk) => self.hunk_preview = Some(hunk),
            None => self.message = "no hunk here".into(),
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
            'b' => self.blame_line(),
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
    use strop_core::Buffer;

    /// A git repo with one committed file, edited in-memory.
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
        let path = root.join("f.rs");
        std::fs::write(&path, "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        let mut e = Editor::new(Buffer::open(path.to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.discover_git();
        (dir, e)
    }

    #[test]
    fn gutter_tracks_live_edits() {
        let (_d, mut e) = fixture();
        e.refresh_hunks();
        assert_eq!(e.sign_at(1), None);
        e.feed_text("j"); // line 2
        e.feed_text("cc");
        for c in "fn b2() {}".chars() {
            e.feed(Key::Char(c));
        }
        e.feed(Key::Esc);
        e.refresh_hunks();
        assert!(e.sign_at(2).is_some(), "changed line must carry a sign");
    }

    #[test]
    fn hunk_nav_and_undo() {
        let (_d, mut e) = fixture();
        e.feed_text("Go") // last line, open below
            ;
        for c in "fn d() {}".chars() {
            e.feed(Key::Char(c));
        }
        e.feed(Key::Esc);
        e.feed_text("gg"); // top
        e.refresh_hunks();
        e.jump_hunk(true); // ]c equivalent
        assert_eq!(
            e.buf().line_of(e.cursor),
            3,
            "jumps to the added line 4 (0-based 3)"
        );
        e.undo_hunk();
        e.refresh_hunks();
        assert_eq!(e.sign_at(4), None);
        let got = e.buf().rope.to_string();
        assert_eq!(got, "fn a() {}\nfn b() {}\nfn c() {}\n", "got: {got:?}");
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
        assert!(e.hunk_preview.is_some(), "Space g p opens the hunk card");
    }
}
