//! Owned shell request identities (R9 §5): every shell job carries a
//! ticket allocated at registration, and every terminal result —
//! success, failure, cancellation — owns exactly that ticket. Stale,
//! duplicated or superseded results die at the handler's registry
//! check instead of mutating whatever currently occupies the view.

use std::path::PathBuf;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Ticket, WorkerId};

/// Which surface owns a shell request, captured at registration and
/// never re-derived from current editor state at delivery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShellKey {
    /// `:!cmd`: an output request for the document that ran it. The
    /// focus token is the request itself — only the newest display
    /// request may still switch the view when it lands.
    Display {
        origin: DocumentId,
        revision: BufferRevision,
        focus: WorkerId,
    },
    /// `|cmd`: replace `[start, end)` of a document captured at a
    /// revision. The range is already normalized, clamped and
    /// boundary-checked — delivery validates exactly what was
    /// captured, not re-derived arguments.
    Pipe {
        document: DocumentId,
        revision: BufferRevision,
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellIntent {
    pub ticket: Ticket<ShellKey>,
    pub command: String,
    #[serde(with = "strop_core::path_serde")]
    pub cwd: PathBuf,
    /// Pipe only: the exact bytes captured for `[start, end)`.
    pub original: Option<String>,
}

/// What a shell process produced. Both streams survive failure — a
/// failed display still shows its output, a failed pipe reports its
/// stderr — so failures never masquerade as empty successes.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
}

/// The owned terminal shell result: one per registered request.
pub type ShellResult = strop_core::worker::Completion<ShellKey, ProcessOutput>;
