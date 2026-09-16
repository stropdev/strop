//! Per-project warm-attach status rows (0063 §2): every warm-up
//! decision point records an outcome on the workspace-symbols glue;
//! non-healthy outcomes publish as pinned picker rows — informational
//! chrome that filtering never hides, late source appends never bury,
//! and nothing consumes as a symbol candidate.

use std::path::{Path, PathBuf};

use strop_picker::{Item, Kind, Payload};

use super::super::Editor;

/// One scope project's warm-attach outcome family (0063 §2): mirrors
/// the `AttachDecision` labels plus the ambiguous-marker cold state.
/// Healthy families are recorded to supersede a stale row, never
/// rendered — only non-healthy outcomes earn a picker row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectStatusKind {
    /// The marker carries no language evidence (package.json): the
    /// project stays cold until a document pins a language.
    Ambiguous,
    Attached,
    NoServer,
    Cancelled,
    TrustRequired,
    TrustError,
    NotExecutable,
    SpawnFailed,
    RemoteIo,
}

impl ProjectStatusKind {
    pub(crate) fn from_decision(decision: &super::super::lsp::attach::AttachDecision) -> Self {
        use super::super::lsp::attach::AttachDecision as D;
        match decision {
            D::Attached => Self::Attached,
            D::NoServer => Self::NoServer,
            D::Cancelled => Self::Cancelled,
            D::TrustRequired { .. } => Self::TrustRequired,
            D::TrustError { .. } => Self::TrustError,
            D::NotExecutable { .. } => Self::NotExecutable,
            D::SpawnFailed { .. } => Self::SpawnFailed,
            D::RemoteIo { .. } => Self::RemoteIo,
        }
    }

    /// Healthy outcomes clear a stale row instead of rendering one.
    fn healthy(self) -> bool {
        matches!(self, Self::Attached | Self::Cancelled)
    }

    /// The chip label, ≤ 7 cells (the chip slot's width): quiet and
    /// secondary-colored like any unknown kind label.
    fn badge(self) -> &'static str {
        match self {
            Self::Ambiguous => "cold",
            Self::Attached | Self::Cancelled => "",
            Self::NoServer => "no srv",
            Self::TrustRequired => "trust",
            Self::TrustError => "trust!",
            Self::NotExecutable => "no exec",
            Self::SpawnFailed => "spawn",
            Self::RemoteIo => "remote",
        }
    }
}

/// The recorded outcome for one scope project: the family plus the
/// actionable reason (explain_decision's convention) rendered on the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectStatus {
    pub kind: ProjectStatusKind,
    pub reason: String,
}

impl ProjectStatus {
    /// A refusal outcome with its actionable reason: the status-line
    /// wording for NoServer, explain_decision's convention otherwise.
    pub(crate) fn from_decision(
        decision: &super::super::lsp::attach::AttachDecision,
        name: &str,
    ) -> Self {
        let reason = match decision {
            super::super::lsp::attach::AttachDecision::NoServer => {
                format!("no language server for {name}")
            }
            other => super::super::explain::explain_decision(other),
        };
        Self {
            kind: ProjectStatusKind::from_decision(decision),
            reason,
        }
    }

    /// An ambiguous marker (package.json): cold until a document pins
    /// the language — the only outcome discovery never reports.
    pub(crate) fn ambiguous(marker: &str) -> Self {
        Self {
            kind: ProjectStatusKind::Ambiguous,
            reason: format!(
                "{marker} is ambiguous — open a file in this project to attach its server"
            ),
        }
    }

    /// Attached, already live, or in flight: clears a stale row.
    pub(crate) fn healthy() -> Self {
        Self {
            kind: ProjectStatusKind::Attached,
            reason: String::new(),
        }
    }
}

impl Editor {
    /// Record one scope project's warm-attach outcome (0063 §2) and
    /// re-publish the status rows: the latest outcome per project wins,
    /// only non-healthy outcomes earn a row. Rows are the picker's
    /// pinned tail — filtering never hides them, later source appends
    /// never bury them, and nothing consumes them as symbol candidates.
    pub(crate) fn record_project_status(&mut self, root: PathBuf, status: ProjectStatus) {
        {
            let Some(glue) = self.picker.as_mut() else {
                return;
            };
            if glue.picker.kind != Kind::WorkspaceSymbols {
                return;
            }
            match glue
                .project_statuses
                .iter_mut()
                .find(|(path, _)| *path == root)
            {
                Some((_, existing)) => *existing = status,
                None => glue.project_statuses.push((root, status)),
            }
        }
        let rows = self
            .picker
            .as_ref()
            .map(|glue| {
                glue.project_statuses
                    .iter()
                    .filter(|(_, status)| !status.kind.healthy())
                    .map(|(path, status)| project_status_row(path, status, &self.cwd))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.set_pinned(rows);
        }
        self.request_picker_ranking();
    }
}

/// One project status row in the symbol-row shape (0063 §2): a quiet
/// status chip, `name  path · reason` text, and a payload whose accept
/// browses the project root — never a symbol location. Unknown chip
/// labels render SECONDARY; no kind_color family is needed.
fn project_status_row(root: &Path, status: &ProjectStatus, cwd: &Path) -> Item {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let shown = root
        .strip_prefix(cwd)
        .map(strop_picker::display_path)
        .unwrap_or_else(|_| std::borrow::Cow::from(root.display().to_string()))
        .into_owned();
    Item {
        badge: Some(status.kind.badge().into()),
        text: format!("{name}  {shown} · {}", status.reason),
        payload: Payload::ProjectStatus(root.to_path_buf()),
    }
}
