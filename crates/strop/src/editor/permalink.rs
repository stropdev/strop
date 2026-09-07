//! Permalinks (0011 §5, R6, 0033 finding 1): revision-pinned source
//! locations → GitHub/GitLab URLs, yanked via OSC52 or opened with the
//! platform opener. Building is pure — remotes and HEAD come from the
//! cached git context, never a native read on the input path.
//!
//! An SSH-alias remote is the one asynchronous input: OpenSSH's
//! effective configuration (`ssh -G` — Include, wildcard `Host` and
//! `HostName` rules) is evaluated on the IO native worker, and the
//! yank/open completes from the returned hostname. Until OpenSSH
//! answers, nothing is copied and no browser is launched; a failed or
//! unresolved alias is a visible diagnosis, never a guessed URL.

use std::path::PathBuf;

use super::document::Surface;
use super::{Editor, Mode};

/// What the finished URL is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PermalinkIntent {
    /// `space g y`: register + OSC52 payload.
    Yank,
    /// `space g o`: the platform opener.
    Open,
}

/// Frozen pure data for a permalink whose SSH host alias is still
/// unresolved: OpenSSH's answer plus these fields rebuild the URL with
/// no repository handle, no IO, and no state that moved meanwhile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingPermalink {
    pub intent: PermalinkIntent,
    /// The alias exactly as the winning remote spells it.
    pub host: String,
    /// The (name, url) pairs the selection folded over — completion
    /// re-runs the same pure fold against this frozen copy.
    pub remotes: Vec<(String, String)>,
    /// The pinned commit SHA, already resolved (0014): never a branch.
    pub sha: String,
    /// Repo-relative file path, native bytes.
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    /// 1-based line range.
    pub lines: (usize, usize),
}

/// Pure permalink construction: either a ready URL, or an SSH alias
/// the IO worker must ask OpenSSH about.
#[derive(Debug)]
pub(crate) enum PermalinkOutcome {
    Url(String),
    Alias(PendingPermalink),
}

impl Editor {
    /// `Space g y`: permalink for the cursor line (or visual range) —
    /// SHA-resolved, remote-prioritized (0001 pillar 3.3).
    pub(crate) fn yank_permalink(&mut self) {
        self.permalink_action(PermalinkIntent::Yank);
    }

    /// `Space g o`: open the permalink in the browser. The opener
    /// process is fire-and-forget — spawn only, never a wait.
    pub(crate) fn open_permalink(&mut self) {
        self.permalink_action(PermalinkIntent::Open);
    }

    fn permalink_action(&mut self, intent: PermalinkIntent) {
        match self.build_permalink(intent) {
            Ok(PermalinkOutcome::Url(url)) => self.finish_permalink(intent, url),
            Ok(PermalinkOutcome::Alias(pending)) => self.request_ssh_host(pending),
            Err(e) => self.message = e,
        }
    }

    /// Publish a finished URL — the resolution is over, so the
    /// clipboard/register (yank) or opener request (open) may act.
    fn finish_permalink(&mut self, intent: PermalinkIntent, url: String) {
        match intent {
            PermalinkIntent::Yank => {
                self.set_register(None, super::Register::characterwise(url.clone()));
                self.osc52 = Some(url);
                self.message = "permalink copied".into();
            }
            PermalinkIntent::Open => self.request_browser(url),
        }
    }

    /// IO-worker completion: OpenSSH said what the alias means, so the
    /// rebuild is pure from the frozen pending data.
    pub(crate) fn complete_ssh_permalink(&mut self, pending: PendingPermalink, hostname: String) {
        let Some(strop_git::permalink::SelectedRemote::Alias(alias)) =
            strop_git::permalink::pick_remote(&pending.remotes)
        else {
            self.message = format!("permalink remote vanished: {}", pending.host);
            return;
        };
        let Some(remote) = alias.resolved(&hostname) else {
            self.message = format!(
                "SSH host {} has no resolved web hostname; check ssh -G",
                pending.host
            );
            return;
        };
        let url =
            strop_git::permalink::permalink(&remote, &pending.sha, &pending.path, pending.lines);
        self.finish_permalink(pending.intent, url);
    }

    /// Pure: every input (workdir, remotes, HEAD sha) is cached git
    /// context — no libgit2 handle, no IO, no spawn (R6). An alias
    /// remote comes back unresolved: evaluation is IO-worker work.
    pub(crate) fn build_permalink(
        &self,
        intent: PermalinkIntent,
    ) -> Result<PermalinkOutcome, String> {
        let Some(context) = self.git.as_ref() else {
            return Err("not a git repository".into());
        };
        let file = match self.surface() {
            Some(Surface::Diff {
                commit: Some(commit),
                ..
            }) => commit.current.clone(),
            Some(Surface::Diff {
                origin: Some(origin),
                ..
            }) => self
                .docs
                .get(origin.buffer)
                .and_then(|document| document.buf.path.clone())
                .ok_or("no source file for this diff")?,
            _ => self.buf().path.clone().ok_or("no file for this buffer")?,
        };
        let abs = if file.is_absolute() {
            file
        } else {
            context.workdir().join(file)
        };
        let rel = abs
            .strip_prefix(context.workdir())
            .map_err(|_| format!("{} is outside the repo", abs.display()))?
            .to_path_buf();
        let (a, b) = if self.mode == Mode::Visual || self.mode == Mode::VisualLine {
            (
                self.buf().line_of(self.anchor()) + 1,
                self.buf().line_of(self.head()) + 1,
            )
        } else {
            let l = self.buf().line_of(self.head()) + 1;
            (l, l)
        };
        if context.remotes.is_empty() {
            return Err("no remote configured".into());
        }
        let selected = strop_git::permalink::pick_remote(&context.remotes)
            .ok_or("no supported web remote configured")?;
        // the location is revision-pinned (0014): on a commit surface it
        // links THAT commit's file, on a working buffer it links HEAD;
        // index/worktree content is local state, HEAD pins it
        let sha = match self.surface() {
            Some(Surface::Diff {
                commit: Some(cf), ..
            }) => cf.sha.clone(),
            _ => context.head_sha.clone().ok_or("no HEAD commit")?,
        };
        Ok(match selected {
            strop_git::permalink::SelectedRemote::Web(web) => PermalinkOutcome::Url(
                strop_git::permalink::permalink(&web, &sha, &rel, (a.min(b), a.max(b))),
            ),
            strop_git::permalink::SelectedRemote::Alias(alias) => {
                PermalinkOutcome::Alias(PendingPermalink {
                    intent,
                    host: alias.alias,
                    remotes: context.remotes.clone(),
                    sha,
                    path: rel,
                    lines: (a.min(b), a.max(b)),
                })
            }
        })
    }
}
