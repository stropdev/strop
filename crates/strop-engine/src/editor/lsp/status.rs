//! Read-only lifecycle projection and retained terminal failures.
//!
//! Discovery, live servers and refusals already have authoritative owners.
//! Only a terminal `Failed` event needs separate retention: admitting it
//! tears down the server, bindings and placements, but its actionable reason
//! must remain available after later, unrelated messages.

use std::path::{Path, PathBuf};

use strop_lsp::ServerId;
use strop_workspace::Filesystem;

use super::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspPhase {
    Starting,
    Ready,
    Refused,
    Failed,
}

impl LspPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Refused => "refused",
            Self::Failed => "failed",
        }
    }
}

/// One owner-scoped lifecycle entry. `resource` is a workspace root once
/// discovery completes; for pending/refused discovery it is the trigger path.
#[derive(Debug, Clone, Copy)]
pub struct LspStatus<'a> {
    pub phase: LspPhase,
    pub server: Option<ServerId>,
    pub name: &'a str,
    pub language: &'a str,
    pub resource: &'a Path,
    pub target: &'a Filesystem,
}

/// The reason and complete scope that survive server teardown.
pub(crate) struct LspFailure {
    pub server: ServerId,
    pub name: String,
    pub language: String,
    pub root: PathBuf,
    pub target: Filesystem,
    pub reason: String,
}

impl LspFailure {
    fn serves(&self, language: &str, root: &Path, target: &Filesystem) -> bool {
        self.language == language && self.root == root && self.target == *target
    }
}

impl Editor {
    /// Lifecycle state for presentation, derived from the same tables used
    /// for routing. No message text participates in this projection.
    pub fn lsp_statuses(&self) -> impl Iterator<Item = LspStatus<'_>> {
        let pending = self.lsp_state.attach.pending.keys().map(|key| LspStatus {
            phase: LspPhase::Starting,
            server: None,
            name: &key.language,
            language: &key.language,
            resource: &key.path,
            target: &key.target,
        });
        let servers = self.lsp_servers.iter().map(|server| LspStatus {
            phase: if server.ready {
                LspPhase::Ready
            } else {
                LspPhase::Starting
            },
            server: Some(server.id),
            name: &server.name,
            language: &server.language,
            resource: &server.root,
            target: &server.target,
        });
        let refused = self.lsp_state.attach.refused.keys().map(|key| LspStatus {
            phase: LspPhase::Refused,
            server: None,
            name: &key.language,
            language: &key.language,
            resource: &key.path,
            target: &key.target,
        });
        let failed = self.lsp_state.failures.iter().map(|failure| LspStatus {
            phase: LspPhase::Failed,
            server: Some(failure.server),
            name: &failure.name,
            language: &failure.language,
            resource: &failure.root,
            target: &failure.target,
        });
        pending.chain(servers).chain(refused).chain(failed)
    }

    /// Retain one current failure per scope. A retry for the same resource
    /// replaces its old reason rather than growing an unbounded event log.
    pub(super) fn record_lsp_failure(
        &mut self,
        server: ServerId,
        name: &str,
        reason: &str,
    ) -> bool {
        let Some(owner) = self.lsp_servers.iter().find(|owner| owner.id == server) else {
            return false;
        };
        let failure = LspFailure {
            server,
            name: name.to_owned(),
            language: owner.language.clone(),
            root: owner.root.clone(),
            target: owner.target.clone(),
            reason: reason.to_owned(),
        };
        self.lsp_state.failures.retain(|existing| {
            existing.server != server
                && !existing.serves(&failure.language, &failure.root, &failure.target)
        });
        self.lsp_state.failures.push(failure);
        true
    }

    /// A Ready event retires both this server's failure and an older server
    /// failure for the same resource. Other workspaces keep their failures.
    pub(super) fn clear_lsp_failure(&mut self, server: ServerId) {
        let Some(owner) = self.lsp_servers.iter().find(|owner| owner.id == server) else {
            return;
        };
        let language = owner.language.clone();
        let root = owner.root.clone();
        let target = owner.target.clone();
        self.clear_lsp_failure_scope(&language, &root, &target);
        self.lsp_state
            .failures
            .retain(|failure| failure.server != server);
    }

    pub(super) fn clear_lsp_failure_scope(
        &mut self,
        language: &str,
        root: &Path,
        target: &Filesystem,
    ) {
        self.lsp_state
            .failures
            .retain(|failure| !failure.serves(language, root, target));
    }
}
