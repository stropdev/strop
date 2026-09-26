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
- The immutable 0057 archive has 75 claims but recorded dirty inputs.
  A second clean Linux x86_64 snapshot at `a05d84f` has 74 claims and
  six raw passing gates (`verification/baseline/0057-linux-evidence.json`);
  it is local profile evidence, not macOS/aarch64 or full VF20
  publication. FS-STORE is the additional worker-cutover claim.

One worker integration owner controls protocol/context/identity/outcome
contracts, caller migration and the final proof manifest (0058 §2). This
document is that contract's protocol half.

## 2. Caller inventory and cutover ownership

The 0.34.0 baseline named in §1 used a Python supervisor for SSH commands
and protected saving. That was an implementation baseline, not a runtime
option: the old supervisor, interpreter selector, filesystem/save bundles
and their Rust carriers are no longer shipped. There is no Python helper
fallback.

- `strop-engine::editor::io`, `namespace`, `directory` and
  `remote_completion` own resource binding and read-only browsing. Local
  observations, listings and reads use the local worker; SSH keeps
  read-only SFTP browsing/ranged access when no worker can run, while
  admitted endpoints use `RemoteWorker`. Container browsing needs no
  deployment; an explicitly admitted `BoundWorker` routes its reads.
- `editor::remote::save` owns the document's protected write permit,
  frozen Store attempt and `:remote verify`. `:remote edit` deploys/
  admits a verified worker and then checks the displayed snapshot
  against its actual Store baseline. `:remote worker [URI]` admits the
  same endpoint worker without granting a file write permit. No
  worker means no remote mutation route.
- `strop-picker::source::remote` streams native `rg` output through
  `Worker::exec`, never shell interpolation or an analogous local path.
  Starting a remote search is itself the explicit worker-using action;
  deployment takes place on the source thread, not input→render.
- `strop-git::exec`, `strop-git::remote` and `strop-git::container`
  issue finite Git commands on an already-admitted, endpoint-bound
  worker. Native local libgit2 hot paths remain native.
- `strop-lsp::Client::spawn` uses an admitted worker for every
  local, SSH and container server. The editor supplies its local worker
  lease; missing non-local authority refuses typed, never via a shell,
  Python or direct local process fallback. Project-command trust
  still precedes every launch.
- `editor::containers::ContainerWorkers` records a selected Docker
  engine and canonical id/StartedAt incarnation. `:container-worker`
  authorizes private-cache deployment (or a configured preinstalled
  artifact), without changing readonly browsing or granting writes.
- `strop-terminal` owns VT interpretation and history; the local
  worker owns the PTY, child and stream. SSH/container interactive
  terminals remain disabled even when their worker transport could
  technically carry PTY frames.
- The `strop --ui-stdio` presentation server remains a distinct,
  bounded editor presentation protocol; it is not a second executor.

## 3. Wire shape (WK02, implemented by `strop-worker-protocol`)

One canonical wire for local child stdio, OpenSSH stdio and selected
container exec stdio — no second serializer, JSON-RPC or SFTP variant
per transport.

- **Framing** (`frame.rs`): bounded `Content-Length: N\r\n\r\n` + body,
  the convention strop-ui-protocol/strop-lsp already pin. Header bound
  8192 bytes; body bound 1 MiB. The decoder calls the same-source Verus
  header/body/completeness gates before slicing; the proved bound on
  ready-frame indices assumes the codec supplied its scanned header
  offset and parsed body length. The codec's byte scanner is tested,
  not formally proved. Every violation is a typed `FrameError`;
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
  with frozen content streams and typed receipts), read-only
  `verify_recovered` for a frozen `StepOutcome::Unconfirmed` receipt
  under an exact boot/mount/principal witness (an acknowledged commit
  cannot enter recovered verification), exec (`exec`/stdin chunks/
  half-close/`exec_cancel`/`exec_resize`/exit and `exec_input` events —
  the PTY family included), notify (`subscribe`/`unsubscribe`/events),
  scoped cache maintenance (`collect_cache { context }`), lifecycle
  (`health`/`quiesce`/`shutdown`, `cancel` per request).
- **Outcomes**: the wire reuses strop-workspace's pure taxonomy —
  `OperationIntent`, `PreparedOperation`, `StepReceipt`/`StepOutcome`
  (committed/refused/cancelled/unconfirmed stay distinct),
  `VerifiedOutcome`, `Observation`, `DirectorySnapshot`, `FsFailure`.
  Admission/freshness failures are the protocol's own typed `Refusal`
  (`wrong_incarnation`, `wrong_lease`, `unknown_handle`,
  `stale_subscription`, `capability`, `limit`, `busy`,
  `namespace_changed`, `retiring`, `closed`). The two layers never blur.
  `CacheCollected` reports exact removed object addresses, removed
  receipt count, kept objects and an optional typed `FsFailure`; a
  failure after any unlink is `Incomplete` with partial retirements,
  never an empty success or an automatic retransmission.

- **Same-source admission proof** (`strop-core::worker::session`):
  Verus checks the real `classify_session` function for arbitrary
  `u64` incarnation/lease values and refusal precedence through
  quiesce/close. The worker
  calls that function before handle lookup. The proof assumes the
  handshake's identity facts are honest; it does not verify SSH, Docker,
  OS effects, deployment, frame parsing or save durability.

- **Streams**: every bulk payload has a `StreamId`; chunks are ordered
  per stream; stream handles are session-scoped. Child stdout/stderr are
  data streams, never control messages — a child echoing valid-looking
  JSON cannot forge a committed/exit/control event.
- **Consumer-driven streams (protocol v2)**: file reads, non-PTY exec
  stdout/stderr and live PTY output start with 32 chunk credits per
  stream, below the client's 64-chunk inbound bound. Only the producer
  parks when spent; the client returns session-stamped `stream_credit`
  as chunks leave the consumer queue. Control/cancel/health remain live.
  Dropping an unfinished file payload cancels its request; dropping
  exec or PTY output sends `stream_abandon`, so the worker drains the
  child without retaining bytes or waiting for credits. Process
  revocation remains explicit through its pinned exec/PTY control.
  Late credits/abandonment for finished streams are no-ops; malformed
  credits poison the session. An older v1 worker fails the handshake.
- **Exec settlement (WK11/WK12)**: the worker keeps an admitted exec's
  cancellation owner alive through its output pumps and publishes
  `exec_exit` only after stdout/stderr terminal chunks reach the actual
  writer. A popped chunk is not proof of delivery. Control frames,
  especially `ReadOpened`/`ExecStarted`, precede all queued data so
  a chunk cannot arrive before its stream exists. The client binds
  stdin/resize/revoke to the admitting connection; a new
  incarnation cannot inherit a reused numeric exec ID.


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
   When executing a verified cache object, the worker rechecks its
   matching private receipt and final executable, acquires the stable
   owner-private cache lock, then writes its own `LeaseRecord` before
   Welcome. The deployment probe is another, short-lived session and
   cannot grant the editor a live lease. Reconnect records its new
   session. A killed process may leave a stale record; the collector
   keeps every recorded lease rather than inferring liveness by age.
   If the managed executable was unlinked after exec but before Hello,
   or (on Linux) its final cache path no longer names the running
   inode, no live lease or Welcome is issued. The client receives the
   pre-handshake protocol refusal directly, not a timeout that might
   be mistaken for a usable worker.
   On live SSH/container admission, the selected worker runs native
   `collect_cache` before the editor publishes ready; a preinstalled
   administrator object outside the managed cache is not a target.
   The worker binds the requested context to its own receipt. Its
   OS lock excludes a new Welcome from lease snapshot through scoped
   retirement. Malformed leases refuse before deletion; concurrent
   publishers that lose the race fail their final-path handshake rather
   than claiming a removed object is ready. This is a cooperating
   same-principal OS-lock contract, not a crash-durability or hostile
   peer guarantee; native platform qualification remains open.
2. **Host/mount namespace** — `NamespaceIdentity` captures the Linux
   boot ID, mount-namespace id and effective principal, separately
   from the per-process worker session. A recovered Store verifies
   through fresh *read-only* authority only if this witness matches;
   stale prepared writes still refuse. An unattested platform can
   verify in its live session but cannot claim cross-restart recovery.
   Containers additionally bind their selected Docker engine, canonical
   id and StartedAt at the deployment provider.
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

- First deployment requires an explicit worker-using action (`:remote
  edit`, `:remote worker`, remote search or `:container-worker`) with
  endpoint/principal-bound consent. Read-only browsing never deploys;
  a restricted/SFTP-only host remains readable without a worker.
- The verified artifact is a matching native binary in `--worker-stdio`
  mode, bound to the declared version/protocol/target and exact
  content digest. The private cache holds content-addressed objects;
  a version string or handshake alone is not host attestation.
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
  activation and probe cleanup (WK05/WK06) — the worker being deployed
  cannot deploy itself; later scoped garbage collection runs only in
  the admitted native worker;
- the `--ui-stdio` early entry and native-free replay/export CLI paths,
  which run before editor setup;
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
