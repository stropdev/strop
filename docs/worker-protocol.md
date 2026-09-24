# Worker protocol and migration contract (0058 WK01/WK02)

Status: WK01 contract record + WK02 wire/codec. Normative for every later
WK slice; the codec crate `strop-worker-protocol` is the sign-off by
construction — it implements exactly the families documented here, no
more. Plan of record: [0058 unified native worker](../plans/0058-unified-native-worker.md).

## 1. Accepted baseline identity

The baseline this release migrates is the post-0056 candidate:

- Source: commit `88e3fb1` (main at authoring), workspace version
  `0.34.0`; the 0056 architecture release landed at `de729a7` and the
  0.34.0 gates at `88e3fb1`.
- Lock: `Cargo.lock` at that source identity; the worker-protocol crate
  joins the workspace without disturbing pinned inputs.
- Accepted architecture ledger (0056, landed): AR01 readonly rendering,
  AR02 admitted action/effect API (`editor/api.rs`), AR03 semantic
  presentation/read models, AR04 draft checkpoint/recovery, AR05
  resource-binding/operation outcomes, AR06 bounded work/queues, AR07
  selected-context native/SSH/container execution admission, AR08
  trust/privacy classification, AR09/AR10 the bounded UI protocol
  (`strop-ui-protocol`, `strop --ui-stdio`) and Rust driver, AR11/AR12
  install/release identity, AR13–AR16 identity exhaustion, ownership
  splits, pinned inputs and integration gates.
- Verification ledger (0057, the pre-worker baseline WK16 migrates):
  VF01–VF20 as named in `plans/0057-core-verification-and-assurance.md`;
  the worker release rebinds rather than inherits them (0058 §8 mapping
  table is the migration manifest's skeleton).

One worker integration owner controls protocol/context/identity/outcome
contracts, caller migration and the final proof manifest (0058 §2). This
document is that contract's protocol half.

## 2. Caller inventory (current consumers of strop-remote / strop-fs)

Every consumer below migrates to the worker client; WK14 removes the old
paths as a clean cutover.

**Engine I/O and filesystem (strop-engine):**
`editor/filesystem/{mod,prepare,verify}.rs` drive `strop_fs::batch`
prepare/apply/verify on worker threads (`strop-fs-prepare/apply/verify`);
`editor/filesystem/reconcile.rs` is the editor-side guarded-reload
surface (admission/receipt vocabulary, dirty buffers never clobbered);
`editor/filesystem/draft/*` compiles Directory intents;
`editor/io/open.rs`, `editor/directory/{mod,jobs}.rs`,
`editor/document/directory.rs` and `editor/remote_completion/directory.rs`
consume `strop_fs::{list, parent, from_remote, from_container}`.

**Engine remote browsing/save (strop-engine → strop-remote):**
`editor/remote/{mod,chooser,commands,view,follow,save}.rs` use
`RemoteClient` (read-only SFTP), `ReadSelection`/`RemoteWindow`/
`RemoteOffset`/`ReadLimit` ranged reads and `strop_remote::save` for
protected remote save; `editor/picker/preview.rs` resolves ranged
previews; `editor/remote_completion/mod.rs` and
`editor/remote/chooser.rs` use `enumerate_hosts`/`HostSources`.

**Picker/search (strop-picker):** `source/remote.rs` runs `rg` over SSH
through `strop_remote::stream` with bounded argv batches;
`source/{worker,grep}.rs` run local search; `source/{catalog,snapshots}.rs`
are the 0063 ProjectCatalog/snapshot consumers whose invalidation rides
the notify family (§5 below).

**LSP (strop-lsp):** `client/spawn.rs` spawns local servers in the
workspace root and remote servers through strop-remote's one-owned-process
policy (`RemoteCommand` + `command_supervised`, `StdinMode::Relayed`,
`SupervisionKey`); containers go through strop-containers exec.

**Git (strop-git):** `exec.rs` and `remote/mod.rs` run finite Git
commands over endpoints via `RemoteCommand`/`strop_remote::run`;
container Git uses the bounded exec-capture boundary; libgit2 hot paths
stay native (WK10 changes where jobs run, not their algorithm).

**Terminal (strop-terminal/strop):** the 0055 terminal owns local
PTY/emulator semantics; `strop --terminal-helper` is an early-entry edge
(§7). Worker-owned PTY input/resize/close joins the exec family behind
`capabilities.pty`; no remote/container interactive terminal ahead of its
authorized milestone.

**CLI (strop):** `cli.rs` uses `ReadSelection`/`ReadLimit` for
noninteractive remote reads; `ui_stdio.rs` serves the separate AR09
presentation protocol (nested UI-server→worker ownership, never a second
engine).

**Providers themselves:** strop-remote (`client.rs` SFTP read-only codec
+ pool, `transport/wire.rs`, `exec/{run,python,spec,supervisor,stream}.rs`
Python-supervised exec, `filesystem/` protected.py bundle, `save/`
helper.py, `hosts/`, `selection.rs`); strop-fs native executor
(`local/{prepare,execute,verify,outcome,copy}.rs`, `observation.rs`,
`stage.rs`, `batch.rs`, `guard.rs`, `trash.rs`, `listing.rs`). strop-fs
currently depends on strop-remote/strop-containers adapters; WK03 inverts
that direction — the shared kernel must not reach back through clients
(acyclic split, no worker→client→worker cycle).

## 3. Wire shape (WK02, implemented by `strop-worker-protocol`)

One canonical wire for local child stdio, OpenSSH stdio and selected
container exec stdio — no second serializer, JSON-RPC or SFTP variant
per transport.

- **Framing** (`frame.rs`): bounded `Content-Length: N\r\n\r\n` + body,
  the convention strop-ui-protocol/strop-lsp already pin. Header bound
  8192 bytes; body bound 1 MiB. Every violation is a typed `FrameError`;
  corruption poisons the stream and closes it, never resynchronizes by
  guessing.
- **Bodies** (`codec.rs`): one class byte, then either a JSON control
  envelope (`ClientMessage`/`WorkerMessage`) or a binary stream chunk
  `[stream: u64 LE][sequence: u64 LE][flags: u8][bytes…]` with a 256 KiB
  payload ceiling; flags bit 0 is `last` (direction half-close), reserved
  bits are typed violations. Bulk file/VT/process bytes flow only as
  chunks — never giant JSON arrays, base64 copies or full-response
  buffers. Paths/argv/env inside envelopes stay byte-exact (the
  strop-core path_serde convention and bounded byte strings), never
  lossy UTF-8.
- **Handshake** (`message.rs`): `hello` (protocol version, client
  name/version/build/target) answered by `welcome` (protocol, worker
  identity, fresh `Session { incarnation, lease }`, `NamespaceIdentity`,
  `Limits`, `Capabilities`). Version/build/target mismatch is a truthful
  refusal, not a downgrade; a version string is not attestation of a
  compromised host — the trusted-host assumption stays explicit.
- **Request families** (`request.rs`): observation
  (`observe`/`list`/`read`/range), mutation (`prepare`/`apply`/`verify`
  with frozen content streams and typed receipts), exec
  (`exec`/stdin chunks/half-close/`exec_cancel`/exit events),
  notify (`subscribe`/`unsubscribe`/events), lifecycle
  (`health`/`quiesce`/`shutdown`, `cancel` per request).
- **Outcomes**: the wire reuses strop-workspace's pure taxonomy —
  `OperationIntent`, `PreparedOperation`, `StepReceipt`/`StepOutcome`
  (committed/refused/cancelled/unconfirmed stay distinct),
  `VerifiedOutcome`, `Observation`, `DirectorySnapshot`, `FsFailure`.
  Admission/freshness failures are the protocol's own typed `Refusal`
  (`wrong_incarnation`, `wrong_lease`, `unknown_handle`,
  `stale_subscription`, `capability`, `limit`, `busy`,
  `namespace_changed`, `retiring`, `closed`). The two layers never blur.

- **Streams**: every bulk payload has a `StreamId`; chunks are ordered
  per stream; stream handles are session-scoped. Child stdout/stderr are
  data streams, never control messages — a child echoing valid-looking
  JSON cannot forge a committed/exit/control event.

WK04 integration amendments (integration-owner approved 2026-09-24):
`ResultOutcome::Failed { failure }` carries the workspace `FsFailure`
taxonomy for the observation/read/exec-admission families — mutation
failures already ride inside `StepReceipt`, and the two layers never
blur; `Prepared` carries `refused: Vec<OperationRefusal>` alongside the
admitted steps so the review surface keeps in-process parity.

## 4. The incarnation triple and prepared authority

Three identities are kept distinct on the wire (`id.rs`):

1. **Worker session** — `Session { incarnation, lease }`, fresh per
   worker process, stamped on every post-handshake client envelope.
   `guard::Authority` rejects stale stamps with typed refusals: a
   restarted worker rejects old handles, write permits and subscriptions;
   connection loss never replays a mutation; verification of an uncertain
   attempt uses retained before/intended/observed evidence plus newly
   admitted read authority and never revives an old session's write
   capability.
2. **Host/mount namespace** — `NamespaceIdentity`, a native boot/mount/
   container observation plus principal; a changed namespace fails
   closed (`namespace_changed`), never retargets retained authority.
3. **Editor document binding** — `DocumentStamp { document, revision }`,
   carried opaquely on mutation requests and echoed in receipts so a
   late committed outcome reconciles its source exactly once. Workers do
   not manufacture editor permits; the editor checks its captured
   owner/binding/revision before granting authority.

## 5. Filesystem notifications (2026-09-14 / 2026-09-16 amendments)

The notify family is first-class: `subscribe`/`unsubscribe`, `notify`
events, `notify_overflow`, `reconcile_boundary`.

- Events are **hints**: advisory freshness that triggers observation/
  reconciliation — never an authoritative edit, save receipt or write
  log. A confirmed rename/save result cannot be erased by a late event.
- Subscription identity is `Subscription { id, generation }`; reused
  descriptors/inodes cannot resurrect old authority; events carry a
  per-subscription monotone sequence so scan completion
  (`reconcile_boundary`) cannot erase a newer invalidation.
- Overflow, registration failure, disconnect and excluded subtrees are
  explicit (`notify_overflow`, typed refusals); the client invalidates
  the baseline, reobserves and reestablishes coverage before claiming
  freshness. Bounded flow: coalescing produces a conservative rescan
  obligation, never a silently dropped staleness signal.
- Coverage is honest per namespace: `NotifyCoverage::{native, polling,
  on_demand, unsupported}`; a periodic full-tree crawl is not an
  invisible default, and watch/read happens in the actual selected Linux
  namespace — no local fallback for a remote path or guessed bind mount.
- Search invalidation (0063) and optional task reruns consume this
  service; they never install unrelated local-only watchers. Git state
  and the catalog invalidate by observed generation; a missed or
  coalesced event self-heals on the next observation.
- Guarded reload uses the existing admission/receipt vocabulary; dirty
  buffers receive external-change state and are never silently
  clobbered by a stale reload.

## 6. Capability vocabulary and deployment consent

`Capabilities` advertises only admitted native primitives in this
namespace: `observe`, `list`, `read`, `write`, `trash`, `notify`
(coverage-typed), `exec_finite`, `exec_service`, `pty`. An unavailable
capability answers `Refusal::capability` — never a no-op command, a
guessed local execution or a retry through SFTP/shell/Python. Read-only
SFTP browsing survives for hosts that cannot run a worker (WK15).

Deployment consent/policy (WK05–WK08 contract, recorded here so protocol
and deployment agree):

- First deployment requires an explicit authorized worker-using action
  with endpoint/principal-bound consent/policy; browsing a read-only
  SFTP host never uploads or executes code.
- The artifact is the same release's binary in `--worker-stdio` mode,
  bound to the client's exact release/build/target; a private per-user
  cache holds immutable version/target/content-addressed artifacts with
  verified-object activation and lease-safe, receipt-scoped cleanup.
- No root/PATH/rc/image/package-database/noexec-evasion tricks; an
  explicit administrator-provisioned path is validated under the same
  rules and an invalid override never chooses another file.
- Bootstrap is a small fixed audited POSIX path and/or existing SFTP
  upload — no Python prerequisite, no project paths/argv/file contents
  in generated shell source; the deployed binary speaks the handshake
  itself, so shell/interpreter diagnostics cannot impersonate a worker.
- Offline/unauthorized/refused states are truthful typed outcomes
  (install state machine steps 1–6 of the plan), not silent fallbacks.

## 7. Edge owners that must NOT recurse into the worker

These paths create the worker or precede it; they use shared low-level
primitives directly and never depend on a worker whose creation they
enable (0058 §4):

- editor-internal startup and session/bootstrap sequencing;
- the private state/recovery store (AR04 checkpoints, watermarks);
- trace and config loading, and worker diagnostics themselves (private
  bounded stderr, never protocol authority);
- release/cache bootstrap I/O: artifact verification, upload, cache
  activation/cleanup (WK05/WK06) — the worker being deployed cannot
  deploy itself;
- the `--terminal-helper` and `--ui-stdio` early entries and the
  replay/export CLI paths, which run before editor setup;
- `strop update` / installer identity (AR11/AR12).

This is not a loophole: user-file save and admitted service execution
always go through the worker; local document mutation is not filesystem
I/O.

## 8. Wire-shape pins and proof hooks

`cargo test -p strop-worker-protocol` pins: exact frame bytes and JSON
envelope shapes (handshake, requests, refusals, notify events), chunk
binary layout and bounds, typed refusals for header/body/chunk/flags
violations, loopback round-trips (handshake, stamped request, non-UTF-8
chunk payload, arbitrary fragmentation), and stale-incarnation rejection
by `Authority` — the WK17/WK19 model/correspondence work keys on these
exact types.
