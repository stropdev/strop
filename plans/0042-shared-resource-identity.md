# 0042 — Shared resource identity (handoff S1, slices 1–2)

Status: adopted from [0041](0041-handoff-adoption-and-roadmap.md) after the review
session (user direction, 9 Sep 2026). This is the identity slice only: one shared
definition of remote/workspace identity, every consumer migrated, superseded
definitions deleted. No capability snapshots, no execution routing changes, no
container work — those are later slices with their own plans.

## 1. Why this slice first

The 0.19.1 field report demonstrated the cost of per-consumer path semantics:
the read path followed symlinked ancestors while the save path refused them,
and `:LINE` parsing existed locally but not remotely. Today three crates
independently encode "local vs one remote endpoint": `FileTarget` (editor),
`FsTarget`/`DocPath`/`Workspace` (LSP), `RepoTarget` (Git), all over address
types that live inside the transport crate (`strop-remote/src/address`). Identity
data depending on the transport inverts the layering the handoff requires and
blocks containers (0037 DC1), which need the same identity without SSH.

## 2. Decisions

1. **New crate `strop-workspace`.** Pure identity and path contracts. Depends on
   `strop-core` (native path bytes), `serde`, `thiserror`. Never on strop-remote,
   async-lsp, git2, or any I/O. Dependency direction: `strop-remote`,
   `strop-lsp`, `strop-git`, `strop-picker`, and the editor all depend on it.
2. **The address module moves.** `strop-remote/src/address/{endpoint,file,
   location,uri,error}.rs` move to `strop-workspace` unchanged in behavior —
   they are pure: parse/validate/serialize, no subprocess, no filesystem.
   `strop_remote::RemoteEndpoint` and friends become
   `strop_workspace::RemoteEndpoint`; no re-export shim remains in strop-remote
   (clean cutover, every caller migrated in this change).
3. **`Filesystem` supersedes `FsTarget`.** `Filesystem { Local,
   Remote(RemoteEndpoint) }` is the one namespace identity. Same serde wire
   shape as `FsTarget` (variant names `Local`/`Remote`); type rename only.
4. **`ResourceLocation` supersedes `DocPath`.** `{ filesystem: Filesystem,
   path: PathBuf }` — one document path on one filesystem. The serialized field
   keeps `target` via `#[serde(alias = "target")]` so existing traces/sessions
   decode; new writes emit `filesystem`.
5. **Service types keep their shape, change their parts.** `strop-lsp`'s
   `Workspace` (root + binding + LSP URI mapping) and `strop-git`'s `RepoTarget`
   (workdir + provenance) stay service-specific enums, now built on
   `strop_workspace` identity. Their serde shapes do not change.
6. **`FileTarget` stays in the editor.** It is the *unresolved open target*
   (a remote `RemoteLocation` may still be a `~` home query). Resolved identity
   is `ResourceLocation`; the two are not merged.
7. **URI byte math deduplicates into `strop-workspace`.** `remote_file_uri` /
   `decode_uri_path` in `strop-lsp/src/target.rs` overlap `address/uri.rs`; one
   strict percent-codec survives, async-lsp `Url` conversion stays in strop-lsp
   (the crate boundary must not pull in async-lsp).
8. **Workspace registry (slice 2), minimal and real.** The editor gains
   `WorkspaceId` (generational, strop-core arena style) → `WorkspaceContext {
   filesystem: Filesystem, root: PathBuf, incarnation: u64 }`. Local cwd is
   registered at startup; a remote context registers on first use of an
   endpoint+root. The registry is the future home of capability snapshots and
   the explain buffer's workspace section; jobs keep capturing the concrete
   targets they already capture (no behavior change in this slice).

## 3. Explicitly out of scope

No new clocks on messages, no capability model, no `ExecutionBinding`, no
path-mapping translation layer, no container identity. The handoff's §4.2
vocabulary arrives with the consumers that need it. `RemoteWorkspace.tla` gains
no new transitions in this slice — identity moves, semantics do not; the model
work lands with the first behavioral user of incarnation (registry rebinding).

## 4. Tests

- Moved address tests run unchanged under strop-workspace.
- Serde: `Filesystem` and `ResourceLocation` decode the exact JSON 0.19.1 wrote
  (fixture strings recorded from the old types), and round-trip.
- Cross-crate: an LSP diagnostic routed by `ResourceLocation` and a Git query
  routed by `RepoTarget` for the same remote file agree on namespace identity.
- No-local-fallback: remote `ResourceLocation` values never produce a local
  `Path` through any accessor (compile-time by type, plus the existing
  remote-URI tests).

## 5. Gate

`docker compose run --build --rm test` and `model` green; clippy `-D warnings`;
no file past the ~800-line ceiling without a single-pattern justification;
publication order script re-derived (new crate in the workspace graph).
