//! Read-only lifecycle projection and retained terminal failures.
//!
//! Discovery, live servers and refusals already have authoritative owners.
//! Only a terminal `Failed` event needs separate retention: admitting it
//! tears down the server, bindings and placements, but its actionable reason
//! must remain available after later, unrelated messages.

use std::path::{Path, PathBuf};

use strop_core::id::DocumentId;
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

/// How many times one workspace scope's crashed server is restarted
/// without being asked. A count, not a time window: the decision must
/// replay identically from a trace, and wall-clock time does not.
pub(crate) const RESTART_BUDGET: u8 = 3;

/// One workspace scope's automatic-restart ledger.
pub(crate) struct Restart {
    pub language: String,
    pub root: PathBuf,
    pub target: Filesystem,
    /// Restarts spent; an explicit request (`gd`, `K`, …) resets it.
    pub attempts: u8,
    /// Documents the dead server served — project files and the
    /// library files routed to it — reopened on the replacement.
    pub documents: Vec<DocumentId>,
}

/// What a crashed server's restart needs, captured before teardown
/// erases its bindings.
pub(crate) struct RestartPlan {
    language: String,
    root: PathBuf,
    target: Filesystem,
    /// A served document inside the root: its discovery finds the same
    /// workspace again.
    representative: DocumentId,
    documents: Vec<DocumentId>,
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

    /// A server that had finished its handshake and then died is a
    /// crash, and crashes restart. One that never became ready would
    /// fail the same way again: its refusal stands until the user acts.
    /// Remote servers ride their connection's own recovery.
    pub(super) fn lsp_restart_plan(&self, server: ServerId) -> Option<RestartPlan> {
        let owner = self.lsp_servers.iter().find(|owner| owner.id == server)?;
        if !owner.ready || owner.target != Filesystem::Local {
            return None;
        }
        let served = |id: &DocumentId| {
            self.lsp_state
                .bindings
                .get(id)
                .is_some_and(|binding| binding.server == server)
        };
        let inside = |id: &DocumentId| {
            self.lsp_state.bindings.get(id).is_some_and(|binding| {
                binding.server == server && binding.path.starts_with(&owner.root)
            })
        };
        let mut documents: Vec<DocumentId> = self
            .lsp_state
            .bindings
            .keys()
            .copied()
            .filter(served)
            .chain(
                self.lsp_state
                    .jump_contexts
                    .iter()
                    .filter(|(_, context)| context.server == server)
                    .map(|(id, _)| *id),
            )
            .collect();
        documents.sort();
        documents.dedup();
        let current = (!self.docs.is_empty()).then(|| self.current());
        let representative = current
            .filter(inside)
            .or_else(|| documents.iter().copied().find(inside))?;
        Some(RestartPlan {
            language: owner.language.clone(),
            root: owner.root.clone(),
            target: owner.target.clone(),
            representative,
            documents,
        })
    }

    /// Spend one restart from the scope's budget and rediscover the
    /// workspace; returns the status line. The failure stays recorded
    /// (and `LSP!` lit) until the replacement reports Ready.
    pub(super) fn lsp_restart(&mut self, plan: RestartPlan, name: &str) -> String {
        let RestartPlan {
            language,
            root,
            target,
            representative,
            documents,
        } = plan;
        let index = self
            .lsp_state
            .restarts
            .iter()
            .position(|restart| {
                restart.language == language && restart.root == root && restart.target == target
            })
            .unwrap_or_else(|| {
                self.lsp_state.restarts.push(Restart {
                    language,
                    root,
                    target,
                    attempts: 0,
                    documents: Vec::new(),
                });
                self.lsp_state.restarts.len() - 1
            });
        let restart = &mut self.lsp_state.restarts[index];
        if restart.attempts >= RESTART_BUDGET {
            restart.documents.clear();
            return format!(
                "lsp: {name} failed again after {RESTART_BUDGET} restarts — not restarted; \
                 repeat a request to retry — :explain for details"
            );
        }
        restart.attempts += 1;
        restart.documents = documents;
        let attempt = restart.attempts;
        self.lsp_attach_document(representative);
        format!(
            "lsp: {name} failed — restarting ({attempt}/{RESTART_BUDGET}) — :explain for details"
        )
    }

    /// The replacement for a crashed server is placed: every document
    /// the dead one served opens on it, library files included.
    pub(super) fn lsp_reopen_restarted(
        &mut self,
        language: &str,
        root: &Path,
        target: &Filesystem,
    ) {
        let Some(restart) = self.lsp_state.restarts.iter_mut().find(|restart| {
            restart.language == language && restart.root == root && restart.target == *target
        }) else {
            return;
        };
        for document in std::mem::take(&mut restart.documents) {
            if self.docs.get(document).is_some() {
                self.lsp_did_open_document(document);
            }
        }
    }

    /// The user asked again: the scope's budget is theirs to refill.
    pub(super) fn lsp_refill_restarts(&mut self, language: &str, target: &Filesystem) {
        self.lsp_state
            .restarts
            .retain(|restart| !(restart.language == language && restart.target == *target));
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
