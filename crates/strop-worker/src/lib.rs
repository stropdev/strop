//! The unified native worker's in-process kernel consumers (0058 WK03).
//!
//! This crate is the shell the worker entrypoint (WK04) wires to its protocol
//! transport: handler-facing code that runs beside the files and processes it
//! owns. It holds no editor state and no transport handles.
//!
//! - The filesystem/save/observation kernel is [`strop_fs`]: pure handler
//!   functions driven in-process under one admitted
//!   [`strop_fs::ExecutionContext`], whose incarnation binds prepared
//!   authority to this worker's lifetime.
//! - [`exec`] is owned process supervision: the native port of the fixed
//!   supervisor semantics from strop-remote (owned target, readiness,
//!   revoke-before-reap, TERM/grace/KILL, descendant cleanup, nonce-marked
//!   status records), with no interpreter in the loop.
//! - [`notify`] (Linux) is the filesystem-notification backend: one
//!   inotify instance per generation-stamped subscription, parent/name
//!   guard coverage, kernel-cookie rename pairing and honest
//!   overflow/rescan-obligation reporting over the WK02 notify family.
//! - [`serve`] (WK04) is the serving loop the `--worker-stdio` entrypoint
//!   wires to its transport: frames in, admitted dispatch to the
//!   kernel/exec/notify owners above, framed outcomes out. It owns the
//!   session state for one client lease, never a listener or editor state.

pub mod exec;
#[cfg(target_os = "linux")]
pub mod notify;
pub mod serve;
