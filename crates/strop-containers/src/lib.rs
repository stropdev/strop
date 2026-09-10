//! strop-containers: attach to an *existing* container on the local Docker
//! engine and browse/read its filesystem (0037 DC1a — browse/read).
//!
//! # Capability boundary (read-only by construction)
//!
//! This crate exposes exactly four engine conversations: probe (`docker
//! info`), discovery (`docker ps` + `docker inspect`), and filesystem reads
//! (`docker cp … -` tar streams). There is deliberately **no** write, delete,
//! exec-of-arbitrary-argv, LSP or Git surface: those are DC1b policy
//! decisions, not accidental omissions. Unsupported operations are refused
//! with typed errors ([`ContainerError::CapabilityRefused`]), never silently
//! approximated.
//!
//! # Ownership and identity
//!
//! - Attaching never transfers lifecycle ownership: nothing here creates,
//!   stops, restarts or removes a container, and no local path is ever
//!   touched — a container path is not aliased to an analogous host path.
//! - The engine is the **local** Docker CLI only (argv arrays, never shell
//!   strings). Remote engines are DC5; no SSH daemon inside the container
//!   is required or used.
//! - Identity is the canonical 64-hex inspect id plus the incarnation
//!   (`State.StartedAt`), never a container name alone. A name that
//!   re-resolves to a different id is a stale-identity refusal; a container
//!   that restarts between inspect and read is detected by a cheap
//!   `started_at` re-check before every read.
//!
//! # Process policy
//!
//! Every `docker` invocation runs through `strop_core::process`
//! supervision: an [`OwnedProcess`](strop_core::process::OwnedProcess)
//! process group, the caller's [`CancelToken`], a wall-clock deadline and
//! bounded pipe retention. Output that overflows its bound is refused
//! (listings) or truncated by explicit `max` semantics (file reads), never
//! presented as complete when it is not.

mod engine;
mod error;
mod identity;
mod read;
mod tar;

pub use engine::{engine, inspect, list_running, revalidate, EngineRef};
pub use error::ContainerError;
pub use identity::{ContainerIdentity, ContainerRef};
pub use read::{list_dir, read_file, DirEntry, DirEntryKind};
