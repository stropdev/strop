//! The dive chain (0010 §3, 0011): commit browser → changed files →
//! file delta, blame card → browser at that commit, sidebar focus.
//! Enter dives, q unwinds through ReturnPoints (by document id).

use std::path::Path;

use strop_git::Hunk;

use super::git_memory::{diff_surface_text, hunk_stats, CommitFiles, Surface};
use super::Editor;

impl Editor {
    /// Enter with the blame gutter on: dive into the cursor line's
    /// commit, positioned at its sha (0011 §3). An unloaded or edited-
    /// stale gutter falls back to the single-line card; with the gutter
    /// off, Enter stays inert in normal mode.
    pub(crate) fn dive_from_blame(&mut self) {
        if self.buf().readonly || self.buf().path.is_none() {
            return;
        }
        let key = self.blame_key();
        match self.blame_gutters.get(&key) {
            None => {}
            Some(_) if self.blame_gutter_for(self.current()).is_some() => {
                let line = self.buf().line_of(self.head());
                match self.blame_gutters.get(&key).and_then(|g| g.lines.get(line)) {
                    Some(bl) if bl.is_uncommitted() => self.message = "uncommitted line".into(),
                    Some(bl) => {
                        let sha = bl.sha.clone();
                        self.open_log_at(&sha);
                    }
                    None => {}
                }
            }
            Some(_) => self.blame_line(), // still loading (or stale): card
        }
    }

    /// Enter on a surface line dives deeper (0001 pillar 3.2).
    pub(crate) fn dive(&mut self) {
        let line = self.buf().line_of(self.head());
        match self.surface().cloned() {
            Some(Surface::CommitLog { rows, .. }) => {
                let Some(sha) = rows.get(line).and_then(|r| r.sha.clone()) else {
                    return;
                };
                let Some(repo) = &self.git else { return };
                match strop_git::memory::show_stat(repo.workdir(), &sha) {
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
                    Ok(diff) => {
                        // the delta carries its commit + siblings: the
                        // sidebar and `]f`/`[f` navigate them (0011 §4)
                        let commit = CommitFiles {
                            sha: sha.clone(),
                            files: files.clone(),
                        };
                        self.open_delta(
                            "delta",
                            &file.path.display().to_string(),
                            diff.hunks,
                            None,
                            Some(commit),
                        );
                    }
                    Err(e) => self.message = e,
                }
            }
            _ => {}
        }
    }

    /// Tab on a commit diff: hop focus between the file sidebar and
    /// the diff content (tuicr's model, 0011 §4).
    pub(crate) fn toggle_sidebar_focus(&mut self) {
        let Some(Some(Surface::Diff {
            commit: Some(_),
            sidebar_focus,
            ..
        })) = self.docs.get_mut(self.current()).map(|d| &mut d.surface)
        else {
            self.message = "tab: no file sidebar here".into();
            return;
        };
        *sidebar_focus = !*sidebar_focus;
    }

    pub(crate) fn sidebar_focused(&self) -> bool {
        matches!(
            self.surface(),
            Some(Surface::Diff {
                sidebar_focus: true,
                ..
            })
        )
    }

    /// `]f` / `[f`: next/previous file of the same commit (0011 §4).
    /// Rewrites the diff surface in place — the surface keeps its
    /// return point; only the file it shows changes.
    pub(crate) fn commit_file_step(&mut self, forward: bool) {
        let Some(Surface::Diff {
            commit: Some(cf),
            label,
            ..
        }) = self.surface().cloned()
        else {
            self.message = "]f/[f: file navigation needs a commit diff".into();
            return;
        };
        if cf.files.len() < 2 {
            self.message = "single-file commit".into();
            return;
        }
        let Some(cur) = cf
            .files
            .iter()
            .position(|f| f.path.display().to_string() == label)
        else {
            self.message = "current file not in commit".into();
            return;
        };
        let n = cf.files.len();
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        let file = cf.files[next].clone();
        let Some(repo) = &self.git else { return };
        match repo.commit_file_diff(&cf.sha, &file.path) {
            Ok(diff) => self.load_commit_delta(&cf, &file.path, diff.hunks),
            Err(e) => self.message = e,
        }
    }

    /// Swap the current diff surface to another file of the same
    /// commit: surface data and buffer text in place, cursor to top.
    pub(crate) fn load_commit_delta(&mut self, cf: &CommitFiles, path: &Path, hunks: Vec<Hunk>) {
        let (added, deleted) = hunk_stats(&hunks);
        let label = path.display().to_string();
        let text = diff_surface_text(&label, &hunks);
        let idx = self.current();
        self.doc_mut(idx).buf.replace_all_system(&text);
        if let Some(Some(Surface::Diff {
            label: slot,
            hunks: hunk_slot,
            added: add_slot,
            deleted: del_slot,
            ..
        })) = self.docs.get_mut(idx).map(|d| &mut d.surface)
        {
            *slot = label.clone();
            *hunk_slot = hunks;
            *add_slot = added;
            *del_slot = deleted;
        }
        // the highlighter follows the file the surface now shows
        self.doc_mut(idx).highlighter = strop_syntax::Highlighter::for_path(&label);
        self.set_head(0);
        self.view_mut().view_top = 0;
        let pos = cf
            .files
            .iter()
            .position(|f| f.path == path)
            .map_or(0, |i| i + 1);
        self.message = format!("{label} · {pos}/{}", cf.files.len());
    }
}
