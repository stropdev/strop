//! strop-containers: attach to an *existing* container on the local Docker
//! engine, browse/read its filesystem (0037 DC1a), and run supervised
//! in-container programs (0037 DC1b, hardened for 0056 AR07).
//!
//! # Capability boundary
//!
//! The engine conversations are: probe (`docker info`), discovery
//! (`docker ps` + `docker inspect`), filesystem reads (`docker cp … -`
//! tar streams) and supervised exec. There is deliberately **no** write,
//! delete, create, stop/restart, provisioning or install surface:
//! unsupported operations are refused with typed errors
//! ([`ContainerError::CapabilityRefused`]), never silently approximated.
//!
//! # Ownership and identity
//!
//! - Attaching never transfers lifecycle ownership: nothing here creates,
//!   stops, restarts or removes a container, and no local path is ever
//!   touched — a container path is not aliased to an analogous host path.
//! - The engine is the **local** Docker CLI only (argv arrays, never shell
//!   strings). Remote engines are DC5; no SSH daemon inside the container
//!   is required or used.
//! - The probe pins the selected connection: every invocation carries
//!   the probed CLI context, so a `docker context use` elsewhere cannot
//!   redirect strop's traffic between probe and exec.
//! - Identity is the canonical 64-hex inspect id plus the incarnation
//!   (`State.StartedAt`), never a container name alone. A name that
//!   re-resolves to a different id is a stale-identity refusal; a
//!   container that restarts between inspect and read is detected by a
//!   cheap `started_at` re-check before every read, and execution
//!   revalidates id + incarnation again at admission.
//!
//! # Process policy
//!
//! Every `docker` invocation runs through `strop_core::process`
//! supervision: an [`OwnedProcess`](strop_core::process::OwnedProcess)
//! process group, the caller's [`CancelToken`], a wall-clock deadline and
//! bounded pipe retention. Directory listings stream the tar archive
//! through an incremental parser that retains only direct-child metadata,
//! so a subtree's bulk bounds the transfer, never the memory; a listing
//! whose retained metadata overflows its bound is refused, and file reads
//! truncate by explicit `max` semantics — nothing partial is ever
//! presented as complete.
//!
//! Execution additionally runs the in-container program under a fixed
//! POSIX sh supervisor whose stdin is the lifetime lease: local close or
//! client death ends the whole session group in-container (TERM, bounded
//! grace, KILL), with nonce-marked launch/exit/termination records. See
//! [`ExecSpec`] for the honest limits (a `setsid`-ing descendant escapes;
//! zombie reaping belongs to the container's init; distroless images
//! without a shell get a typed refusal).

mod engine;
mod error;
mod exec;
mod identity;
mod read;
mod tar;

pub use engine::{engine, inspect, list_running, revalidate, Captured, EngineRef};
pub use error::ContainerError;
pub use exec::{AdmittedExec, ExecRecord, ExecSpec, LaunchCause, SessionKey};
pub use identity::{ContainerIdentity, ContainerRef};
pub use read::{list_dir, read_file, DirEntry, DirEntryKind};
