//! Filesystem notifications, editor half (0058 §2 amendments 2026-09-14/16;
//! specs/Notify.tla correspondence). The worker beside the files owns
//! native watching; the editor owns application of observations to
//! documents, Directory buffers, Git signs and the search baseline.
//!
//! - **Events are hints.** A hint triggers observation/reconciliation —
//!   never an authoritative edit, save receipt or relocation. An
//!   `Ambiguous` rename invalidates; only an observation-confirmed
//!   outcome may change a binding.
//! - **Identity.** A subscription's authority is its (session, scope,
//!   generation) triple; events stamped by a superseded identity are
//!   dropped, never acted on (StaleIdentityNeverActs).
//! - **Bounded flow.** Worker events land in [`NotifyQueue`], a bounded
//!   coalescing buffer; the event-loop wake is a pure hint (AR06-legal:
//!   the state lives here). Queue/hint-set bounds and native overflows
//!   coalesce into a conservative rescan obligation — the only sign of
//!   staleness is never silently dropped (CoalesceNeverLosesStaleness).
//! - **Publication.** A clean buffer reloads only through a guarded
//!   job: document/binding/revision re-checked at completion, published
//!   only when the fresh observation differs from the buffer's baseline
//!   (a confirmed own-save receipt is never erased by a late hint). A
//!   dirty buffer is preserved and gains external-change state
//!   (DirtyNeverClobbered); a stale in-flight reload never clears newer
//!   edits (StaleReloadNeverClears). Loss/overflow/exclusions invalidate
//!   the baseline and reobserve before freshness is claimed again.

mod queue;
mod reconcile;
mod subscribe;

#[cfg(test)]
mod tests;

pub(crate) use queue::{NotifyQueue, Record, ReloadKey, SubscribedScope};
pub(crate) use subscribe::NotifyState;
