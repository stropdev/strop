//! Permalinks (0011 §5): revision-pinned source locations → GitHub/GitLab
//! URLs, yanked via OSC52 or opened with the platform opener. On a commit
//! surface the link pins THAT commit (0014 wave 4).

use std::path::{Path, PathBuf};

use super::git_memory::Surface;
use super::{Editor, Mode};

impl Editor {
    /// `Space g y`: permalink for the cursor line (or visual range) —
    /// SHA-resolved, remote-prioritized (0001 pillar 3.3).
    pub(crate) fn yank_permalink(&mut self) {
        match self.build_permalink() {
            Ok(url) => {
                self.set_register(None, url.clone(), false);
                self.osc52 = Some(url);
                self.message = "permalink copied".into();
            }
            Err(e) => self.message = e,
        }
    }

    /// `Space g o`: open the permalink in the browser.
    pub(crate) fn open_permalink(&mut self) {
        let url = match self.build_permalink() {
            Ok(url) => url,
            Err(e) => {
                self.message = e;
                return;
            }
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

    pub(crate) fn build_permalink(&self) -> Result<String, String> {
        let Some(repo) = self.git.as_ref() else {
            return Err("not a git repository".into());
        };
        // on a commit-diff surface the file is the surface's label —
        // the buffer itself is virtual and has no path
        let surface_label = match self.surface() {
            Some(Surface::Diff { label, .. }) => Some(label.clone()),
            _ => None,
        };
        let path = surface_label
            .as_deref()
            .or(self.buf().path.as_deref())
            .map(str::to_string);
        let Some(path) = path else {
            return Err("no file for this buffer".into());
        };
        let path = path.as_str();
        let abs = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            repo.workdir().join(path)
        };
        let Ok(rel) = abs.strip_prefix(repo.workdir()) else {
            return Err(format!("{} is outside the repo", abs.display()));
        };
        let (a, b) = if self.mode == Mode::Visual || self.mode == Mode::VisualLine {
            (
                self.buf().line_of(self.anchor()) + 1,
                self.buf().line_of(self.head()) + 1,
            )
        } else {
            let l = self.buf().line_of(self.head()) + 1;
            (l, l)
        };
        let remotes = repo.remotes();
        if remotes.is_empty() {
            return Err("no remote configured".into());
        }
        if strop_git::memory::pick_remote(repo).is_none() {
            return Err(format!("unsupported remote URL: {}", remotes[0].1));
        }
        // the location is revision-pinned (0014): on a commit surface it
        // links THAT commit's file, on a working buffer it links HEAD
        let revision = match self.surface() {
            Some(Surface::Diff {
                commit: Some(cf), ..
            }) => strop_git::GitRevision::Commit(cf.sha.clone()),
            _ => strop_git::GitRevision::Head,
        };
        let loc = strop_git::SourceLocation {
            revision,
            path: rel.to_path_buf(),
            lines: Some((a.min(b), a.max(b))),
        };
        strop_git::memory::permalink(repo, &loc).ok_or_else(|| "no HEAD commit".into())
    }
}
