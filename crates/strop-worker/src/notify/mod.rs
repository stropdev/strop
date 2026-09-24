//! Filesystem notification backend: Linux inotify through a thin direct
//! binding (0058 §2 amendments 2026-09-14/16; WK02 notify family).
//!
//! ## Backend decision
//!
//! A direct `libc::inotify_*` binding rather than the `notify` crate:
//!
//! - **Overflow honesty.** `IN_Q_OVERFLOW` arrives in-band on the instance
//!   it invalidated; this adapter turns it into a per-subscription
//!   rescan obligation (`Event::NotifyOverflow`) with no intermediate
//!   event-enum translation. The `notify` crate surfaces it only as
//!   `EventKind::Other` + a `Rescan` flag on an event with no path, after
//!   crossing its own abstraction.
//! - **Bounded flow.** The `notify` crate's recommended watcher queues
//!   through an unbounded channel; the wire contract (WK11) requires every
//!   queue bounded. Here the kernel queue (per instance,
//!   `/proc/sys/fs/inotify/max_queued_events`), the per-subscription
//!   pending queue, the pending-rename table and every emitted batch all
//!   have explicit caps, and hitting one latches a conservative rescan
//!   obligation — the staleness signal is never silently dropped.
//! - **Dependency weight / static builds.** inotify is four syscalls; the
//!   binding adds zero dependencies (`libc` is already in the tree) and is
//!   trivially musl-static, where `notify` pulls mio/walkdir/filetime and
//!   friends to do the same registration walk this module does anyway
//!   (inotify has no recursive mode on any backend).
//! - **Rename semantics.** Cookie pairing, parent/name guard watches and
//!   the explicit `Ambiguous` class are Strop contract, not backend
//!   defaults; owning the classification keeps the wire honest.
//!
//! ## Shape
//!
//! One inotify instance (fd) **per subscription**: overflow invalidates
//! exactly one baseline, watch descriptors are never shared or ambiguous
//! across subscriptions, and cancellation is one `close(2)`.
//!
//! Each subscription registers **parent/name guard coverage** of the scope
//! root (replacement/rename of the root itself) plus tree coverage of the
//! root (recursive when asked). inotify is natively flat; recursion is a
//! bounded walk here, with unreadable/over-cap subtrees recorded as
//! explicit coverage gaps that invalidate the baseline.
//!
//! Events are hints. Rename pairs come only from kernel cookies; anything
//! unpaired, self-moved or queue-lost is `NotifyKind::Ambiguous` plus a
//! rescan obligation — never a guessed relocation.
//!
//! Remote and container namespaces are refused typed
//! ([`NotifyError::UnsupportedNamespace`]); the relay is a later slice.

mod manager;
mod sys;
#[cfg(test)]
mod tests;

pub use manager::{NotifyManager, Subscribed};

use std::path::PathBuf;

use strop_worker_protocol::{Capability, Refusal};

/// Hard bounds for one manager. Defaults are production values; tests use
/// tiny caps to exercise overflow/exclusion paths deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyConfig {
    /// Live subscriptions per manager (worker session).
    pub max_subscriptions: usize,
    /// Kernel watches one subscription may hold; exceeding this excludes
    /// the rest of the tree and invalidates the baseline (typed, honest).
    pub max_watches_per_subscription: usize,
    /// Buffered hints per subscription awaiting publication; a full queue
    /// latches a rescan obligation rather than dropping staleness.
    pub max_pending_hints: usize,
    /// Hints per `Event::Notify` batch (bounded wire messages).
    pub max_hints_per_event: usize,
    /// Read cycles per drain per subscription (work bound against a
    /// continuously-written kernel queue).
    pub max_read_cycles: usize,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            max_subscriptions: 64,
            max_watches_per_subscription: 8192,
            max_pending_hints: 4096,
            max_hints_per_event: 256,
            max_read_cycles: 64,
        }
    }
}

/// A typed notify outcome. Admission/freshness failures map onto the
/// wire's [`Refusal`] vocabulary via [`NotifyError::refusal`]; the wire has
/// no I/O-fault refusal variant, so backend faults ride `Limit` with the
/// precise message — typed and truthful, never a guessed success.
#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// Remote/container namespaces are out for this backend; the relay
    /// slice owns them. Maps to `Refusal::Capability { Notify }`.
    #[error("native notify watches only the local namespace; remote/container scopes need the worker relay")]
    UnsupportedNamespace,
    /// A notify scope must be absolute; relative paths never resolve here.
    #[error("notify scope must be an absolute path")]
    RelativeScope,
    /// `Limits::max_subscriptions` reached for this worker session.
    #[error("subscription bound reached ({0} live)")]
    SubscriptionLimit(usize),
    /// The kernel refused a watch registration. Fatal when nothing of the
    /// scope could be covered; partial coverage instead invalidates the
    /// baseline with a rescan obligation.
    #[error("watch registration failed for {}: {source}", path.display())]
    Register {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `inotify_init1` failed (fd/accounting exhaustion).
    #[error("inotify init failed: {0}")]
    Init(#[source] std::io::Error),
    /// `poll(2)` over the live subscription fds failed.
    #[error("inotify poll failed: {0}")]
    Poll(#[source] std::io::Error),
    /// No live subscription with this id in the session.
    #[error("unknown subscription {id}")]
    UnknownSubscription { id: u64 },
    /// The named generation predates the current one: a reinstalled
    /// subscription invalidates the old identity, so late events or
    /// retires against it are refused.
    #[error("stale subscription {id} generation; the current generation is {current}")]
    StaleGeneration { id: u64, current: u64 },
}

impl NotifyError {
    /// The wire-level refusal for this outcome (WK02 vocabulary).
    pub fn refusal(&self) -> Refusal {
        match self {
            NotifyError::UnsupportedNamespace => Refusal::Capability {
                capability: Capability::Notify,
            },
            NotifyError::UnknownSubscription { id } => Refusal::UnknownHandle {
                message: format!("subscription {id}"),
            },
            NotifyError::StaleGeneration { current, .. } => {
                Refusal::StaleSubscription { current: *current }
            }
            other => Refusal::Limit {
                message: other.to_string(),
            },
        }
    }
}
