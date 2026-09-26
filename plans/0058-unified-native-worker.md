# 0058 — Unified native worker: local first, the same protocol remotely

Status: **implementation in progress; not release-qualified**. The
local/SSH/container worker routes, protected Store, session credits and
PTY cutover have native evidence; `docker compose run --build --rm model`
exhausts the WorkerSession and WorkerDeploy bounds and semantic mutants.
`verification/check.py --release` still refuses `WPERF-FULL`
and `WPLAT-NATIVE`. Worker-native retirement serializes registration
with collection and its generalized lock/snapshot induction now
passes; full native-platform and product-performance gates do not.
WorkerSession, WorkerDeploy and WorkerCacheGC have symbolic TLAPS
inductions over their TLC models; none proves Rust refinement,
filesystem effects or a live peer. The original 0057
archive has 75 claims and dirty inputs; the separate clean `a05d84f`
Linux x86_64 pre-worker snapshot has 74 claims and six raw passing
gate logs under
`verification/baseline/0057-linux-evidence.json`. The omitted FS-STORE
claim belongs to this worker cutover, not that clean pre-worker binary.
Neither archive qualifies macOS/aarch64 target execution; those hosts
are unavailable on this workstation. Scoped local warm-handshake and
worker Health control-frame observations are in
`verification/measurements/`; §10 still needs remote deployment,
editor input/render, LSP and terminal-load measurements on every native
target profile.

```text
0054 filesystem -> 0055 TUI terminal -> 0056 architecture -> 0057 core verification
    -> 0058 unified native worker + assurance migration
    -> [deferred last:] 0059 completion -> 0060 debugger -> 0061 GUI (+ 0062 distribution)
```

The user explicitly chose deployment of a native worker: development hosts normally
permit the required writes, while restricted production machines may forbid executing
uploaded Python just as they forbid a binary. Interpreter-based portability is not
worth maintaining a second filesystem/process policy engine indefinitely. This is
permission for a deliberate deployment flow, not blanket permission to write to or
execute on every reachable host. No claimed “10x” performance improvement precedes
measurement.

## 1. Decision and non-negotiable boundaries

**Use one Rust worker implementation and one versioned service protocol for local,
SSH and supported running-container operations.** Ship the worker as a noninteractive
mode of the existing release binary, provision that matching artifact where authorized,
and retire Strop's Python filesystem/save/process-supervisor implementations.

The default product path launches the local worker too; it is not a test-only
server behind a production local bypass. The editor's user-resource filesystem and
owned service execution clients consume the same commands, framing, checked admission
and outcomes irrespective of transport. Native effects occur in the chosen namespace.

```text
editor: buffers / grammar / history / preview / views / recovery authority
                    |
        one typed worker client and protocol
          /                 |                    \
 local child stdio     OpenSSH stdio      selected container exec stdio
          \                 |                    /
             same release's native worker
          checked context + shared Rust executor
                    |
             native filesystem / processes
```

The worker is **not a second editor**, UI server or general RPC/plugin platform.
Input, text mutation, syntax, preview and rendering stay in the editor; no RPC per
keystroke, no UI-thread wait, and no remote round trip needed to edit already loaded
text. The worker owns admitted native I/O/process handles, not unsaved document truth.

Worker lifetime is explicitly leased to its client session. It has no public listener,
root installation, system service, always-running reconnect daemon or cross-editor
session sharing. A leased worker can handle successive operations/services without
being respawned for every request. Connection loss and uncertain writes retain honest
semantics; introducing a native process does not create exactly-once external effects.

## 2. Entry, integration ownership and current evidence

Entry requires completed **AR01–AR16 and VF01–VF20**, their actual source/lock/artifact
identities and the final filesystem/terminal implementation. Reconcile the final
crate/module layout first: source was being split during research, so old filenames
are anchors to responsibilities, not an instruction to recreate obsolete modules.

One worker integration owner controls protocol/context/identity/outcome contracts,
all caller migration and the final proof manifest. Independent deployment, native
executor and evidence work may proceed only after those contracts are written.
Shared-file mutations are coordinated. Skip project-wide validation during concurrent
edits; the integration owner runs the final complete gates on one candidate.

The inspected implementation motivates, but does not complete, this change:

- `strop-remote/src/filesystem/mod.rs` assembles `protected.py` plus the filesystem
  observation/mutation/dispatch bundle; `save/protocol.rs` assembles a related bundle.
- The remote filesystem bundle owns path/lock validation, metadata, publication and
  outcome classification. The native `strop-fs` executor implements overlapping policy.
- The filesystem Python capability path uses Linux `renameat2` through `ctypes` and
  `/proc` identity; “Python exists” is not its complete portability contract.
- `strop-remote/src/exec/{run,python,spec,supervisor}.rs` still uses Python to supervise
  ordinary remote native programs. Replacing filesystem scripts alone leaves Python
  mandatory for Git/LSP/process execution.
- Native prepared capabilities and remote helper capabilities have different lifetime
  domains. A local process nonce must not become a reconnect-stable host identity by
  serialization; copying the old struct unchanged is not a correct protocol design.

The research probe executed the exact 29,095-byte filesystem Python bundle locally:
rename preserved bytes, and a destination created after preparation caused a conflict
while preserving both files. This was not SSH/lease/power-loss or alternative-worker
performance evidence. 0057 must verify what actually ships in its baseline even if
this later release will replace it.


### Filesystem notifications and invalidation are owned here

Amendment (2026-09-14): filesystem notifications, reconciliation, guarded
reload, Git/catalog invalidation and their verification fold into **this
plan**; there is no separate filesystem-notification plan. The unified
worker is the natural owner: one notification source per filesystem session
— local inotify-style watches, remote worker relays, container mounts —
normalized into the existing generation-stamped catalog. Requirements:

- Notification delivery is advisory freshness, never authority: a confirmed
  rename or save result cannot be erased by a late watch event
  ([0054](0054-unified-filesystem-workspace.md) keeps this precedence).
- Guarded reload uses the existing admission/receipt vocabulary — the same
  checked operations, conflicts and recovery as any other filesystem
  mutation; dirty buffers overlay and are never silently clobbered.
- Git state and the project catalog (0063's discovery consumer) invalidate
  by observed generation, not by trust in watch completeness; a missed or
  coalesced event self-heals on the next observation.
- Verification rides the WK16–WK19 proof/correspondence additions: watch
  loss, event storms, reconnect replays and generation-skew cases extend the
  existing model registry and negative controls.

Amendment (2026-09-16, per the external review handoff §9 “Filesystem
notification obligations retained inside 0058”, 2026-09-15): the 2026-09-14
ownership assignment above stands; this amendment deepens it into the detailed
contract the handoff requires. The same filesystem foundation serves local and
admitted Linux SSH/container execution: the worker beside the files owns native
watching; the editor owns application of observations to documents, search
indexes, Directory buffers and Git state.

Required protocol properties:

- **Events are hints.** Invalidation triggers observation/reconciliation; it is
  never an authoritative edit, save receipt or total write log.
- **Subscription identity.** Bind each subscription to the client/worker/context
  incarnation, the logical scope and a subscription generation. Reused
  descriptors or inodes cannot resurrect old authority.
- **Replacement and rename.** Watch parent/name coverage as well as relevant
  objects; reconcile atomic replacement and ambiguous rename. Never silently
  relocate a document from a guessed rename pair.
- **Loss and partial coverage.** Overflow, registration failure, disconnect and
  excluded subtrees are explicit outcomes. Invalidate the relevant baseline,
  reobserve and reestablish coverage before claiming freshness.
- **Bounded flow.** Bound native-to-service queues, retained dirty scopes and
  client publication. Coalesce invalidation into a conservative rescan
  obligation rather than silently dropping the only sign of staleness.
- **Ordering/reconciliation.** Subscription installation, the initial scan and
  later events have a defined reconciliation boundary; scan completion cannot
  erase a newer invalidation.
- **Document publication.** Clean-buffer reload is checked against the expected
  document/binding/observation. A dirty buffer is preserved and receives
  external-change state; newer edits are never cleared by a stale reload.
- **Namespace.** Watch/read in the actual selected Linux filesystem namespace.
  No local fallback for a remote path or a guessed container bind mount.
- **Unsupported filesystems.** Report native, polling or on-demand coverage
  honestly. A periodic full-tree crawl is not an invisible default.

Model and verification obligations: extend the existing TLA+/TLC model for
subscription lifetimes, loss, reconciliation and publication, reusing 0057's
applicable TLAPS/Verus obligations. The model must cover two clients, restarted
workers, reordered/duplicate observations, overflow during rescan, save versus
external write and dirty-buffer races. Name the OS/filesystem assumptions and
provide real Linux traces. The model must correspond to production handlers: a
proof of a toy watcher does not establish editor reload correctness.

Backend note: Linux inotify through a small backend adapter remains the initial
direction. The chosen maintained `notify` crate or thin native adapter must be
inspected at implementation time for actual overflow reporting, queue bounds,
recursive registration and static-build compatibility. Do not pin a dependency
from a stale handoff, and never expose its event enum as Strop's wire contract.

Consumer note: search invalidation (0063) and optional task reruns consume this
service; they never each install unrelated local-only watchers. Protocol
additions remain capability/version checked and preserve control/cancellation
progress under data pressure.

## 3. Required release ledger

All WK requirements are mandatory. A mode flag, a remote upload, a passing protocol
smoke or a Rust port without consumer/assurance cutover is not completion.

| ID | Required result |
| --- | --- |
| WK01 | Recorded accepted AR/VF baseline, complete current capability/caller/platform inventory and one owned worker/assurance migration contract. |
| WK02 | One bounded versioned protocol, checked identity/admission and typed operation/stream/outcome semantics for real local, SSH and container clients. |
| WK03 | Shared production Rust filesystem/save/observation and owned-execution kernels; acyclic dependency split, no local/Python/native policy twins. |
| WK04 | Real local worker mode and client integration, leased startup/shutdown, current local workflow parity and no input/render regression. |
| WK05 | Exact release/target worker artifacts and compatibility manifest, static Linux builds, all retained supported-platform coverage and no GUI/runtime dependency. |
| WK06 | Authorized private deployment/cache, verified-object activation, concurrent-version/lease-safe cleanup, offline/manual provisioning and truthful refusal. |
| WK07 | Direct OpenSSH bootstrap and worker transport with existing authentication/configuration, native arguments and no Strop Python prerequisite. |
| WK08 | Selected-engine/container-incarnation worker deployment/exec/cleanup, correct user/cwd/PIDs, shellless/preinstalled cases and no elevation or lifecycle takeover. |
| WK09 | Complete current filesystem/read/save/operation parity through the worker, preserving metadata, conflict/uncertainty/reconciliation and capability limits. |
| WK10 | Current finite commands, native-library Git work, LSP and admitted services through common worker ownership, preserving efficient implementations, streams, readiness and borrowed/owned semantics. |
| WK11 | Physical queue/stream/work bounds, priority for control/outcomes, fair cancellation and correct service settlement/retirement under sustained load. |
| WK12 | Completed terminal/PTY input, geometry, lifecycle and privacy integration with the worker; emulator/editor semantics remain engine-owned. |
| WK13 | Correct editor/source/binding/history/recovery/trace integration across worker loss, late outcomes, relocation and protocol/capability changes. |
| WK14 | Clean removal of all obsolete Strop Python helpers/supervision/configuration/wire paths and every migrated caller; no write fallback or compatibility twins. |
| WK15 | Complete actual TUI/help/settings/error journeys and native local/SSH/container capability matrix, including restricted hosts and SFTP-only reading. |
| WK16 | Claim-by-claim 0057 assurance migration manifest and live ledger updates; preserve historical baseline evidence without inheriting it as a worker proof. |
| WK17 | Updated/calibrated TLA+/TLC and required generalized TLAPS worker/deployment/session/effect safety, with explicit limits and no weakened inherited claims. |
| WK18 | Same-source Verus proofs for shipped worker/shared kernels and their decoder/caller premises, audited trust changes and proof erasure. |
| WK19 | Real client↔worker↔OS↔editor correspondence, instrumented production Loom, framing/native-byte/fault campaigns and attributable seam mutants. |
| WK20 | Exact-candidate full quality/model/proof/core-assurance/native/artifact gates, measured performance, complete docs/evidence and downstream handoff. |

WK01–WK20 must close before [0059 completion](0059-nonblocking-code-completion.md)
starts. Required behavior/platform/proof reductions need explicit user approval and
an impact/re-entry record in [0028](0028-roadmap-and-review.md); deployment difficulty
is not permission to use an unverified fallback or omit an existing workflow.

## 4. WK02–WK04: one real service, not two implementations

### Responsibility and dependency split

Keep the 0056 admitted editor API and existing `strop-workspace` source/operation
contracts canonical. Factor the final native filesystem/save/lease logic into the
lowest shared layer needed by the worker; transport selection must not sit inside
that executor. In particular, the existing `strop-fs` dependency on remote/container
adapters must not produce a worker→client→worker cycle or a recursive local spawn.

Name modules by responsibility: protocol/admission, client/transport, native fs/save,
process/PTY lifecycle and deployment. Prefer existing crates/modules; introduce a
small crate only where dependency direction or proof ownership actually requires it.
No universal service framework, duplicated command table or independent worker history.

Local development, deterministic exploration and remote deployment execute **the same
Rust handler bodies**. Transport tests must cross the actual codec even on localhost.
A direct executor call remains useful as a lower-level test, not proof of the shipped
local worker path. Do not add an in-process product bypass merely to make a latency
benchmark pass; fix buffering/batching/copies at the proper boundary instead.

The native executor retains platform-qualified behavior. “Shared” does not mean
replace stronger local/remote guarantees with their weakest common denominator, or
branch on a display path containing `ssh:`. Native context and capabilities are data.

### Worker entrypoint and local launch

Implement a real internal `strop --worker-stdio` command, distinct from 0056's
`strop --ui-stdio`. Parse and enter worker mode before TUI setup, session restoration,
normal editor tracing/config loading, terminal ownership or update logic. Stdout is
protocol-only; diagnostics are private bounded stderr and never protocol authority.

Local clients launch the matching installed executable, not an unrelated `strop`
found through PATH or a mutable project executable. Runtime construction/configuration
is pure; process creation occurs on the existing job boundary. Workers are shared
only across explicitly compatible owners/context within that editor session, bounded
in number and retired after their final owner/service closes.

Run ordinary local user-resource I/O and owned execution through this client, not only
a special `:remote` route. Startup establishes the real worker handshake/capabilities;
a successful spawn alone does not mean ready. Worker failure is visible and cannot
silently switch the local product back to a second filesystem implementation.

Editor-internal startup, private state/recovery store, trace/config and release/cache
bootstrap I/O cannot recursively depend on a worker whose creation they enable.
They remain explicit edge owners using shared low-level primitives where appropriate.
Record these boundaries in WK01/WK16; this is not a loophole for user-file save or
service execution to bypass the worker. Local document mutation is not filesystem I/O.

### Protocol contract

Reuse the released bounded framing/codec conventions from 0056. Specify one canonical
wire format and typed request/event/result families; do not invent a second JSON-RPC,
SFTP variant or serializer for local versus remote. Native path/argv bytes retain
exact values, without lossy UTF-8 conversion. Large file/VT/process payloads stream
as bounded bytes rather than giant JSON arrays, base64 copies or full response buffers.

The handshake binds protocol/version, release/build/target, frame/resource limits,
execution principal, host/namespace identity and **fresh worker/session incarnation**.
Advertised capability means the actual relevant native primitive/profile was admitted,
not only that this binary was compiled for Linux. A version string is not cryptographic
attestation of a compromised remote host; the trusted-host assumption stays explicit.

Document request IDs, sequence/stream IDs, lease IDs, offset/length units, terminal
outcomes and overflow retirement. Distinguish worker incarnation from stable host/
mount/container incarnation and from DocumentId/buffer revision. Workers do not
manufacture editor permits; the editor checks its captured owner/binding/revision
before granting authority or updating a source.

Minimum protocol families: bounded observation/list/read/range/follow; prepare/apply/
verify/save with frozen byte streams and typed receipts; current native-library jobs/
subscriptions and admitted finite/service exec;
process input/half-close/output/exit/cancel; existing local terminal/PTY input/resize/
close; explicit lease/health/quiesce/shutdown. An unavailable capability returns a
typed refusal, not a no-op command or a guessed local execution.

Prepared authority belongs to its admitted context. A restarted worker cannot accept
an old live handle or write permit. Verification of an uncertain completed attempt
uses retained before/intended/observed evidence plus newly admitted read authority;
it does not revive an old session's write capability. A changed host/container or
namespace fails closed. Connection loss never automatically replays a mutation.

## 5. WK05–WK08: deployment is part of the product

### Artifact and platform contract

Use the existing release binary's worker mode as the default artifact; Linux workers
are static, rustls-only, with no Python, OpenSSL or remote compiler/package-manager
requirement. Worker mode does not initialize GUI/TUI facilities. A distinct reduced
artifact requires measured justification and the same source/target/provenance gates;
it is not a separately implemented worker or permission to leave packaging unfinished.

Bind deployment to the **exact client release/build and target** in the existing
0056 release catalog. Bootstrap from a verified local artifact; if the remote target
differs from the client's, obtain that matching artifact through the established
local release pipeline and transfer it over SSH. Never download `latest` on the
remote or assume the local machine's CPU/ABI matches the destination.

Required coverage includes every retained shipping local TUI platform and previously
supported remote capability profile. Exercise Linux x86_64/aarch64, glibc-based and
musl/Alpine environments, and the shipped macOS targets. WSL uses the Linux worker.
Inventory other claimed POSIX remote-save support before cutover; add the native
artifact/evidence needed or obtain explicit approval for a named support change.
A broad prior “Python works on POSIX” statement is not silently narrowed to two hosts.
Native Windows workspace/process/ConPTY support remains outside the initial WSL envelope.

### Authorized provisioning and cache ownership

Use the completed trust/admission and install/catalog contracts. First deployment
requires an explicit authorized worker-using action and endpoint/principal-bound
consent/policy; simply browsing a read-only SFTP host does not upload or execute code.
Previously authorized deployment may quietly reuse a compatible verified cache.

For containers, `:containers` / selecting one remains read-only attach
without provisioning (0037 DC2). `:container-worker` on an attached
buffer is the explicit worker-using action: it records consent for that
exact engine, canonical ID, StartedAt and principal. A matching local
release artifact is uploaded only to the selected principal's private
cache; `STROP_CONTAINER_WORKER_PATH` selects an explicitly preinstalled
verified in-container object without a write, including shellless images.
Absent/mistyped overrides refuse, never choose another path. Git/LSP
use only the admitted lease; SFTP/tar read-only browsing remains available
when deployment is refused.
Show the selected target, artifact version, destination and precise refusal reason.

A private per-user Strop cache holds immutable version/target/content-addressed worker
artifacts. Do not modify a user's existing TUI installation, PATH, shell rc files,
project tree, global `/tmp` name, container image or package database. No root/chmod-
away-policy/mount/remount/memfd execution trick to evade a noexec or upload restriction.
An explicit administrator-provisioned artifact/path is supported and validated under
the same compatibility/ownership rules; an invalid override never chooses another file.

Required install state machine:

1. Capture endpoint/principal/context and resolve an authorized native cache root.
   Validate ownership, privacy, native spelling and parent/symlink policy.
2. Reuse an exact verified existing artifact, or create a uniquely owned private
   stage with bounded length/hash and metadata before accepting upload bytes.
3. Transfer bytes through the existing authenticated provider; verify complete
   bytes/provenance, target, executable mode and the intended object before activation.
4. Atomically publish the immutable cache entry under declared filesystem guarantees.
   Concurrent editors/installers cannot overwrite a different version or run a partial
   stage. Failure retains the old entry and removes only positively owned staging.
5. Bind verification to the object executed using the platform's safe mechanism and
   private-directory/identity guarantees. Hashing one file then blindly executing a
   replaceable path is not sufficient evidence.
6. Check the real handshake and capture the lease; activation, execution and readiness
   are separate outcomes. Never report installed/ready merely because upload finished.

Correspondence correction: the deployment provider's handshake is a
short-lived *probe* whose process is stopped before the editor's actual
worker connection. Its `Session.lease` cannot be used as the active
client's cache lease. The real connection must register its own
handshake lease against the verified object before that worker becomes
visible as ready, and every later reconnect must register its fresh
session before admitting work. A failed registration closes/refuses the
connection. An administrator-provisioned object outside Strop's cache
needs no cache record. Old/stale records remain conservative keep
evidence until ownership and cross-client retirement are proved;
inventing liveness from the probe's receipt is forbidden.

A cached worker also re-observes its final executable name before
Welcome. Linux refuses an unlinked `current_exe` (`(deleted)`) or a
final path naming another inode; an invalid/private-directory or
owner-exec observation refuses rather than accepting an unleased
session. On macOS, the same path/owner/mode checks apply, but this
Linux `/proc/self/exe` inode correspondence is not a macOS theorem;
native macOS execution remains a release gate.

Cache garbage collection respects live executable/worker leases and concurrent client
versions. Keep cleanup bounded and scoped by receipts, never glob-delete unrelated
files. Disk full, read-only/noexec cache, wrong architecture, corrupt artifact, failed
rename/sync, concurrent cache replacement and interrupted launch have explicit outcomes.
No guarantee assumes a hostile same-principal process or compromised host reports truth.

GC exclusion implementation: scanning `leases/` and later unlinking
an object is not atomic with another client's handshake record.
The old standalone deployment-side collector could list no lease for
X, let another worker register X and send Welcome, then unlink X from
its stale snapshot; a second scan does not close the race. That code
and its policy-only tests were removed. The selected native worker now
owns `CollectCache { context }` after its live SSH/container handshake,
before editor readiness. It accepts only the context recorded in its
own private executable receipt, holds an exclusive OS-backed lock on
a persistent, owner-private per-cache lockfile from the complete
bounded lease snapshot through every scoped object/receipt unlink,
and keeps its own executable plus **every** recorded lease. An invalid
lease refuses before deleting anything; partial unlink reports exact
removed objects and an `Incomplete` outcome. Preinstalled objects
outside the managed cache are never GC targets.

Every cache-executed worker acquires the same lock before verifying
the final path/inode and matching release receipt, registering its own
session lease and sending Welcome. The lock releases after its record
is synced. `File::lock` provides OS-backed cross-process exclusion on
Unix and unlocks with the handle on crash; only cooperating Strop
processes and the selected same-principal OS are trusted
([Rust File locking contract](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock)).
The lockfile inode is never unlinked/recreated. A concurrent publisher
is not *live* until its actual worker registers under this lock; if
collection wins first, final-path admission refuses instead of
publishing a removed object as ready. A killed worker's stale lease
remains conservative keep evidence, never aged out.

`WorkerDeploy.tla` proves the atomic scoped-`Collect` abstraction, not
an implementation refinement. `WorkerCacheGC.tla` now expands native
admission and collection into `LockWelcome`/`Welcome`,
`Acquire`/`ObserveLeases`/`Retire`/`Release`, crash and concurrent
publication transitions. TLC exhausts 35,897 two-client/two-context/
two-build states; four named mutants break unlocked Welcome, ignored
leases, foreign-context retirement and unchecked final-path admission,
with concurrent-worker/blocked-Welcome/crash witnesses reached.
TLAPS discharges 447 obligations for `Init => Inv`,
`Inv /\ [Next]_vars => Inv'` and the derived snapshot-completeness,
live-object, stale-record and selected-context safety properties.
The proof quantifies over arbitrary nonempty client/context/digest
sets **disjoint from the non-value sentinel**; real client IDs,
endpoint contexts and content addresses satisfy that typed premise.
Another 34 obligations prove conditional retirement properties, and
a matched foreign-context mutation fails its own scoped theorem.
This proves model safety, not OS lock behavior, Rust refinement,
filesystem durability or native macOS/arm execution. WDEP-GC has
Linux real-worker correspondence and is native-tested; the other
platforms remain WPLAT-NATIVE. Shellless/preinstalled and
changed-context refusals remain honest under the cutover.

Offline remote hosts work via client upload. If both ends are offline, a matching
locally cached or explicitly preinstalled verified artifact is required; absence is
a useful refusal, not a remote network/install attempt. Source payloads/credentials
never appear in command arguments, bootstrap logs or deployment receipts.

### SSH: small bootstrap, existing security boundary

Keep the configured system OpenSSH, host-key policy, agent/askpass, aliases, ProxyJump,
connection ownership and strict namespace semantics. Reuse owned connection pooling;
never kill a master Strop did not create. Do not introduce custom authentication,
encryption, a public port or a second interpretation of ssh configuration.

Bootstrap must work **without Python**. Use a small fixed audited POSIX bootstrap and/or
existing SFTP upload operations for deployment, not another shell filesystem engine.
Keep project paths/argv/file contents out of generated shell source. The actual worker
receives data through the protocol. Where shell strings are unavoidable for SSH exec,
use one tested native argument encoding/quoting boundary and controlled entrypoint.

Launch the deployed binary directly into worker mode, not through the old Python
supervisor. Shell/interpreter diagnostics cannot impersonate a worker handshake.
Restricted/SFTP-only accounts continue to offer their honest read capabilities;
worker execution refusal does not enable an alternate write path or bypass policy.

### Containers: selected context is load-bearing

Reuse the completed 0056 execution admission: EngineRef/context, canonical container
ID + StartedAt, selected user/cwd, namespace identity and namespace-local PIDs.
Deployment, stat/readiness, execution and cleanup all use that captured context.
A renamed/restarted container or changed Docker default cannot retarget a request.

Use the provider's scoped file-transfer/exec facility or a verified preinstalled
worker. Respect the selected principal and cache policy; a daemon API's wider access
is not authorization to write as root or chown/chmod the container into compliance.
No container restart/rebuild/removal, image modification, privileged helper or bind
mount is introduced to manufacture success.

Exercise automatic deployment on an authorized writable container and a genuinely
shellless/Python-free preinstalled-worker image. Missing shell, writable/executable
location, safe transfer capability, architecture support or permission is classified
truthfully. No claim that every distroless container can be auto-provisioned.
The native worker itself must not need shell utilities to supervise ordinary programs.

## 6. WK09–WK13: preserve the whole existing product

### Filesystem and saving

Migrate the actual user-resource paths, including local and remote save—not merely
Directory commands. Required families include listing/metadata/native identity,
complete/ranged reads and follow behavior, source fingerprints, create/copy/rename/
move/Trash/restore/remove where currently supported, protected save and verification.
Retain the exact 0054/0040 capability distinctions; no new remote Trash, cross-host
copy, recursive deletion or Windows-local backend is implied by shared code.

The shared executor preserves descriptor-relative operations, native spelling and
reserved names, cooperative lock domains, final-component/ancestor policy, metadata/
xattr preservation, no-clobber primitives, private stages and qualified durability.
Select policy through admitted typed capabilities, not ad-hoc remote/local string
checks. Existing local workflows must not become unnecessarily readonly to simplify
unification; stronger policies are not silently weakened either.

Keep read-only SFTP browsing/range access on hosts that cannot run a worker. Worker-
capable namespaces use the common worker path for the migrated operations. Capability
selection is explicit and stable for an admitted operation; an upload/worker/save
failure never retries through SFTP, shell commands, Python or a local path.

Apply/Save, partial batch outcomes, cancel-before-effect, committed and unconfirmed
remain distinct. Persist/retain frozen attempt evidence before launch. A late committed
receipt reconciles the source once even if the originating view closed; an uncertain
outcome preserves dirty text and never grants automatic retry or rollback.

Prepare and commit still revalidate actual observations at their native boundary.
A stable protocol does not make a stale inode/hash/parent fact valid. Cooperative
locking is not universal CAS against nonparticipants; native Rust does not change
filesystem, power-loss or network-partition guarantees by itself.

### Native programs and long-lived services

Migrate the actual current native/SSH/container exec consumers, finite commands,
Git/LSP streams and implemented shell/filter/task paths. Preserve trust, executable
selection, cwd/environment/native argv, readiness, deadlines and accurate diagnostics.
Keep native library implementations native: in particular, preserve libgit2 hot paths
rather than spawning `git` to fit an exec-only protocol. Worker ownership changes
where those jobs run, not their algorithm into a shell pipeline. Expose current
library-backed work and native observations through bounded typed requests/events;
do not move parser/preview work out of the editor or add per-keystroke process spawn.
No generic command from a request is executed merely because it is well formed;
its editor/service authority and selected context must already be admitted.

The worker owns process/pipe/reader/writer/wait state. Child stdout/stderr are framed
**data streams**, never worker control messages. Close unrelated descriptors and
prevent children inheriting the protocol authority channel. A child that echoes the
old public nonce or valid-looking JSON cannot forge a committed/exit/control event.
Document the compromised-host/same-principal trust limit instead of calling a public
nonce authentication.

Keep actual leased group supervision from the established contract: owned target,
readiness, revoke-before-final-reap ordering, TERM/grace/KILL where appropriate,
normal-exit descendant cleanup and final outcome precedence. Half-closing a child's
stdin differs from revoking its lease; killing local ssh/docker alone is not evidence
of remote descendant cleanup. Borrowed targets are not killed as owned launches.
Remote partition/escaped-session/supervisor-death limitations remain explicit.

Native macOS qualification found a real supervisor fault: after a recorded
exit, XNU can return `EPERM` for `kill(-pgid, SIGKILL)` when only the
unreaped group leader remains. [XNU `killpg1`][xnu-killpg] filters
zombies, then returns `EPERM` with no signalable member; `EPERM` can
also mean a *live* inaccessible descendant. Never broadly forgive
the errno or reap first (that drops the PGID reservation). On macOS
only, while the leader remains unreaped, accept `EPERM` **solely** if
the bounded group-member enumeration contains exactly that known
zombie leader; fail closed on any other member, refusal, or truncation.
The host's system `libproc` is an explicit new macOS runtime trust
dependency; the native artifact gate must permit only that system
library and exercise direct supervisor plus framed exec/PTY exits on
both Intel and Apple Silicon. Linux retains its existing `ESRCH` rule.
[XNU's group-list API][xnu-proc-list] includes live and zombie lists.

[xnu-killpg]: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c
[xnu-proc-list]: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c

### Bounds, scheduling and terminal

Bound frames, admitted requests, input/output chunks, retained snapshots, service
slots, queues, cancellation and final retirement by the actual registered budgets.
Reserve capacity/fairness for control, cancellation and terminal outcomes. Do not
truncate protocol/VT bytes to fit a generic capture buffer, or put EOF behind a full
queue. Backpressure occurs off input; publish-versus-park and close races are covered.

Do not respawn a worker per keystroke, LSP message or terminal packet. Preserve ordered
synchronization barriers and coalescing of genuinely superseded unsent data; never
coalesce accepted mutations or outcomes. Existing headless settle distinguishes
finite work/barriers from a healthy long-lived worker/LSP/terminal.

The completed 0055 terminal remains a real current consumer. Worker-owned local PTY/
process handles, input/resize/close and VT bytes use the common service contract;
emulation, history/projection, controlling-view decisions and presentation stay in
the engine. Preserve local outer-terminal behavior and no local input-latency cliff.
This does not enable SSH/container interactive terminals ahead of their authorized
milestones: unsupported capabilities remain disabled even if transport primitives
could carry them. Native Windows still renders the WSL-owned terminal later in 0061.

### Editor identity, recovery and privacy

Maintain DocumentId/source binding/revision/focus/permit distinctions through the
protocol. Worker restart, host reconnect, relocation and out-of-order results cannot
retarget a buffer or revive authority. LSP sync, source-backed collections, history,
review receipts and recovery records see the same reconciled source event.

The editor retains unsaved data and recovery policy independently of worker life.
Verify uncertain effects after reconnect with fresh namespace evidence without
restoring old write leases. An executable-cache receipt is not a source save receipt,
and an applied reply is not a durable draft checkpoint.

Classify native paths, environment/argv, file/terminal/LSP bytes and errors before
recording them. Payload-free metadata capture stays payload-free across frames,
bootstrap and worker diagnostics. Native-free replay launches no worker, uploads
nothing, invokes no process/file effect and cannot acquire trust from recorded data.
Migration must not put sensitive remote content into local temporary artifacts.

## 7. WK14–WK15: clean cutover and visible capability behavior

Use a complete caller/config/source inventory and language-server references for
exported APIs. Migrate all actual filesystem/save/exec consumers, including containers
and noninteractive Git/LSP—not just the first demonstration route.

Remove obsolete Strop Python sources/bundles, interpreter discovery and overrides
(including `STROP_REMOTE_PYTHON` if still present), old command-spec/bootstrap/status
nonce machinery, legacy decoders, fixture copies and docs/help/settings that advertise
them. Replace legitimate behavior tests with the corresponding worker contract tests;
delete tests that only pin old wiring, strings or source structure.

This does **not** remove Python as a user tool or the runtime of pyright/debugpy/user
programs. A project's Python configuration is unrelated to Strop's deleted helper
interpreter selector. Do not accidentally rename or delete it in a broad codemod.

No old/new selector, deprecated alias, automatic Python fallback or two permanently
maintained full-write implementations survive the release. Historical baseline proof
artifacts may remain immutable evidence; old helper source is not retained in the
shipping tree merely to keep an obsolete proof job green. Existing public commands
(:remote edit/verify, :w, Directory actions, service/terminal controls) keep their
behavior and discovery; a worker mode is infrastructure, not a new editing language.

Surface useful states through the existing help/settings/explain/error owners:
not authorized to deploy; cache not writable/executable; target artifact unavailable;
wrong build/protocol; native capability refused; worker starting/ready/retiring;
transport lost; operation outcome unconfirmed. Do not confuse readiness with upload,
worker identity with a host-key guarantee, or no Python with unrestricted host access.

## 8. WK16: explicit migration of the already-running verification programme

**0057 owns the pre-worker baseline. 0058 owns every assurance delta introduced by
this cutover.** If the other agent is already proving Python helpers, let it complete
those actual-source claims. Do not tell it to skip a boundary because Rust is planned,
or change its frozen source/target inventory under an active proof run.

At entry, import the completed claim inventory and exact baseline evidence. For every
claim record one of: unchanged with justified dependency closure; transferred with
new correspondence; strengthened/reproved; new; or retired **implementation-specific**
claim with its successor and unchanged user promise. A path rename alone is not proof
transfer. The final candidate still runs the full applicable gates, not just changed rows.

Required migration mapping:

| 0057 area | Required worker-release action |
| --- | --- |
| VF01 / VF20 inventory, source and release evidence | Add worker/protocol/target/bootstrap/cache identities, migration status and current-candidate gates; preserve the old qualified baseline as history. |
| VF02–VF04 mutation/input/projections/history | Preserve proofs and independent byte/geometry oracles; requalify real editor publication after worker outcomes, restart and relocated bindings. |
| VF05 LSP/services | Rebind request/sync/binding/retirement claims to worker streams and process leases; prove no reordering or bound bypass. |
| VF06 local filesystem | Replace local bypasses with the common executor/protocol and prove caller/decoder/observer premises; retain all native guarantees. |
| VF07 SSH/SFTP | Preserve SFTP-only claims; add direct worker bootstrap/framing/context mapping without conflating transport identity with file identity. |
| VF08 Python supervisor/codec | Supersede its implementation mapping with actual Rust supervisor, framing, lease and child-stream isolation claims; do not retain a false Python trust dependency or claim an automatic theorem transfer. |
| VF09 Python save/filesystem | Rebind the same baseline/metadata/commit/verify/dirty-state promises to shared Rust executor and actual worker→editor correspondence. |
| VF10–VF11 containers/processes/queues | Reprove captured context/incarnation and actual in-namespace lifetime; adapt Loom to the shipped coordinator and preserve native cleanup evidence. |
| VF12 terminal | Requalify worker-owned PTY/data/control boundaries through the actual TUI while preserving emulator/input/privacy semantics. |
| VF13 recovery | Update worker-loss/unknown-effect/source-baseline interactions and persistence mapping; no worker cache or lease becomes recovered write authority. |
| VF14 UI-stdio | Preserve the separate presentation protocol/server; show nested UI-server→worker ownership/backpressure/recovery without another engine. |
| VF15 privacy/trust | Cover bootstrap/cache/handshake/frames/child diagnostics and native-free replay; remove obsolete helper-config authority without weakening admission. |
| VF16 install/publication | Extend artifact verification/owner receipts to remote/container cache activation and leased cleanup; keep TUI/GUI install identity distinct. |
| VF17–VF19 formal and runtime evidence | Update models, theorem/kernel targets, premises, trust set, calibration and real-code mapping; explicitly discharge new deployment/session/codec/effect obligations. |

Carry the original observable guarantees forward even when no Python implementation
remains. New proof-friendly Rust code is useful only if production executes it and
its real caller establishes the premises. TCB shrinkage is recorded precisely; OS,
OpenSSH, Docker, allocator/serializer/native libraries and filesystem assumptions
are not erased by changing the language.

## 9. WK17–WK19: required proof and correspondence additions

### Human semantics and TLA+/TLC

Extend the existing normative contracts/models rather than creating an unrelated
worker “toy model.” Cover worker/session incarnation, authority preparation/consume,
framing/stream ownership, fair bounded service admission, cancel/half-close/shutdown,
unknown committed effects and deployment/cache activation/retirement.

Model both client and worker decisions, actual observer facts and effects. Different
clients/builds/leases/resources must be reachable in finite instances. Include concurrent
cache installers, worker death between effect and reply, late old-session frames,
container restart/default-context changes, half-closed/blocked pipes and ID exhaustion.
Retain existing remote-save/process/workspace/ChangePlan/editor models and calibrated
limits where still semantically applicable.

Positive TLC configurations exhaust their declared bounds. Exact semantic mutants
must fail the named property, with witnesses showing success/refusal/uncertainty/
recovery paths are live. Missing tools, parse errors, OOM/timeouts or a mutant that
never applied are failed gates, not successful calibration.

### Generalized TLAPS

Preserve the baseline UI/recovery/resource-effect safety theorems, adapting their
abstraction mapping where worker behavior changes. Required worker additions prove:

- admitted effects/receipts belong to the correct client/worker/context incarnation;
  stale handles/frames cannot authorize a new mutation or foreign cleanup;
- one consumed action has one consistent terminal-outcome classification, without
  confusing a lost reply with a pre-effect refusal or automatic retry permission;
- activated executable identity was completely verified under the stated deployment
  assumptions, and cache cleanup cannot retire another live lease/version's artifact;
- stream/session closure and post-effect reconciliation preserve the relevant source,
  recovery evidence and authority invariants.

Prove initialization, inductive preservation and the named observable consequences
for arbitrary finite admitted sets and repeated transitions/restarts; no two-worker/
two-crash cap advertised as generalized. State actual storage/host/process premises.
Liveness/fairness and dead-peer assumptions remain separate from inductive safety.

Proof-engineering amendment (2026-09-25): TLAPS 1.5 cannot translate
`state' = [state EXCEPT !.field[key] = value]` when `state` is a nested
record. Normalize the **same** WorkerSession/WorkerDeploy transition
relation into explicit full-state record constructors with one-level
function updates, not a separate easy-to-prove model. Before relying
on any theorem, require the original bounded TLC state counts,
attributed mutants and reachability witnesses to agree, then discharge
init/induction/observable consequences with the pinned backend.
Equal finite counts are correspondence evidence, never a proof of
general equivalence or a substitute for the TLAPS gate.

WorkerSession now uses that full-state normalization. A fresh stream
registration initializes all per-stream status fields, matching the
native `StreamRegistration::new`; TLC still reaches exactly 232,371
joint, 92,621 two-client Store and 70,733 two-client stream states,
with all seven semantic mutants and reachability witnesses intact.
The pinned TLAPS gate proves 274 WorkerSession obligations, including
initialization, 19-action-plus-stutter induction and the safety
corollary for symbolic positive bounds and arbitrary nonempty sets.
A matched clean Commit theorem proves two obligations; the stale-
commit mutant fails its `NoStaleCommit` step (one of two obligations).
This proof is model safety only; it does not verify OS effects or the
caller facts supplied to the same-source Rust kernels below.

WorkerDeploy uses the same normalization (18-field StateRecord with
one-level function updates, the record-set spelling of Targets and an
explicit published-object premise in VerifyObject matching the native
final-path verifier). TLC reaches exactly the same 396,985-state
two-client graph and 929-state two-target graph with all seven semantic
mutants and witnesses intact. The pinned TLAPS gate proves 565
WorkerDeploy obligations: initialization, 15-action-plus-stutter
induction over arbitrary nonempty Clients/Contexts/Digests with
MUTATION=0, and the consent/activation/context/cleanup/lease safety
corollary. A matched scoped-Collect theorem proves ten obligations;
the cross-context cleanup mutant fails its `OwnedCleanup` step. Like
WorkerSession, this proves model safety, not host durability or refinement.

The worker release's same-source Rust proof boundary now additionally
covers framing header/body/slice admission, the observed Unconfirmed
receipt and namespace gate for read-only recovery, terminal delivery
after cancellation settles, 64-lowercase-hex content addresses,
protocol/version/target admission, matching-receipt activation and
live-lease keep decisions. Production codec, worker, deployment and GC
call the verified functions directly. Decoder scanning, source
attestation, receipt provenance, provider write/rename durability and
global lease liveness are not established by those pure proofs.

### Same-source Verus

Prove the shipped pure admission/transition code for framing bounds/length arithmetic,
identity generation/exhaustion, worker/context/lease freshness, source/save/operation
preconditions and receipt classification, budgets/deadlines and deployment/cache
verify/activate/retire decisions. Reuse transferred shared kernels; do not rewrite
proved local policy into a second remote implementation.

Include decoder-to-kernel and caller/observer premise obligations. Preserve the existing
edit geometry and required batch composition proof; a worker migration cannot drop
unrelated core targets. Audit all new/removed trusted bodies and assumptions. No
external_body around the whole executor/deployment function that assumes its safety,
no assume(false), omitted proof or silent fallback to tests for a required theorem.
Normal Cargo builds execute the same source with ghost code erased.

### Actual implementation correspondence and calibration

Use the released Rust harness/driver. Local and remote campaigns cross the actual
client codec, real worker mode, admitted native executor and editor outcome handlers.
Check native bytes, metadata, process/descriptor lifetime and source/history/draft
state with independent expectations—not the same executor called twice as its oracle.

Map real traces to model transitions, including admitted stuttering; replay bounded
model schedules/counterexamples against production. Document tested versus formally
proved refinement. Loom uses the real queue/admission/publish/wakeup/close coordinator
with instrumented synchronization and recorded exploration bounds, not a copied loop.

At minimum, attributable seam mutants must expose: wrong worker/context accepted;
missing source/base guard; length/budget overflow; child output accepted as control;
early process reaping; omitted wakeup/cancel; forgotten directory sync; successful
ack before commit; corrupted/partial artifact activation; wrong executed object;
cache cleanup ignoring a live lease; default Docker context used; and lost reply
clearing dirty state. Pure-policy proofs plus mocked syscalls cannot catch all these.

Native tests exercise actual local/SSH/container OS effects, including syscall failures
before and after publication. Real process kills are not power-loss proofs; a proof
of a policy kernel is not a proof of the filesystem/SSH/engine implementation.

## 10. WK20: integrated evidence and required release gates

Record baseline versus candidate on the same build profile/hardware/fixtures. Measure
cold deployment, warm launch/reuse, artifact bytes, worker count/RSS, transfer and
retirement high-water marks, input→frame, LSP request/sync latency and terminal
input/output under load. Use p50/p95/p99/max plus deterministic count/byte/work bounds.
Do not hide a new local IPC regression in a remote-only benchmark or claim speedups
from interpreter removal without measuring them.

Required real scenarios include:

- Local open/edit/save/undo/review/Directory operations, dirty relocation, sustained
  search/LSP traffic and terminal interaction through the normal worker product path.
- SSH deployment/reuse, offline remote, wrong target/version, untrusted host and
  restricted/SFTP-only accounts; no remote Python present on capable worker fixtures.
- Linux x86_64/aarch64 glibc/musl and retained macOS/local/remote profiles, with actual
  target execution rather than cross-compilation alone called platform evidence.
- Writable and preinstalled shellless containers, selected nondefault EngineRef,
  restarts/reused names, chosen user/cwd and real program/descendant cleanup.
- Non-UTF-8/control-character native filenames, wrong logical aliases, replaced
  parents/locks, metadata/xattr drift, occupied destination and partial batch outcomes.
- Huge/fragmented/malformed frames, sustained VT/service output, blocked input/output,
  worker/editor/ssh/docker death, cancel versus exit and commit versus reply loss.
- Concurrent clients/builds deploying and retiring cache entries, corrupt/wrong-object
  activation, noexec/read-only/disk-full conditions and positively owned cleanup only.
- Readonly/protected source distinctions, pending/unconfirmed save reconciliation,
  source restore after worker loss, private capture and native-free replay.

Drive the actual TUI at the established wide/narrow/split geometries. Verify worker
startup/refusal/retirement UX and all current editor families—not only protocol JSON.
UI-stdio/headless parity, container/SSH suites and native terminal evidence supplement,
not replace, the local product path.

On the **same final source/lock/proof inputs and worker artifacts**, require:

- `docker compose run --build --rm test` (fmt, locked workspace/all-target clippy and
  locked tests, including preserved differential/behavioral gates);
- `docker compose run --build --rm model`, `verify`, `tlaps` and `core-assurance` with
  the updated complete registry, required obligations and semantic negative controls;
- required-mode native SSH/container/PTY/WSL/platform and artifact/deployment lanes;
- package/static/provenance/publication checks tying the uploaded/executed worker
  digest/build/target to the release candidate and current assurance manifest.

PR and tag publication require that evidence, not an older 0057 green badge or
model-only tag job. Missing required tools/hosts/fixtures fail instead of skip.
Proof runs/mutants use private work/target copies. Archive raw targets, theorem
assumptions, bounds, calibration failures, native platforms and code/artifact hashes.

## 11. Execution order and downstream contract

1. Accept the completed 0056/0057 candidate and inventory all current native/Python
   consumers, capabilities, platforms and claims. Agree protocol/context/receipt and
   deployment contracts before concurrent implementation.
2. Factor shared Rust decisions/native executors and implement actual worker mode.
   Make local normal operation, codec and lifecycle real first; no remote-only port.
3. Add verified artifact/bootstrap/cache flows, direct SSH and captured-container
   transports. Exercise the same protocol and handlers in every context.
4. Migrate filesystem/save/read/exec/LSP/Git/terminal and editor/recovery/privacy
   integrations with complete observable parity. Remove old Python paths as a clean
   cutover, not a released dual-backend transition.
5. Complete the assurance migration, new generalized/shared-kernel obligations,
   calibrated runtime/Loom/native/deployment evidence and exact-candidate full gates.
6. Update actual user docs/help/settings/changelog/release catalog and the WK ledger,
   remove disposable probes/scaffolding and authorize completion only after WK20.

[0059 completion](0059-nonblocking-code-completion.md) consumes the requalified worker/
LSP/input/source baseline and adds its own query/index/acceptance claims.
[0060 debugger](0060-debugger-workflow-and-architecture.md) consumes native worker
execution/streams/context/leases, not a new Python supervisor or installer.
[0061 GUI](0061-gui-windows-and-wsl.md) consumes the WSL engine's worker-backed services
through the separate UI protocol; it does not deploy another remote engine.
[0062 distribution](0062-distribution-and-wsl-onboarding.md) packages the exact worker-
capable backend and extends existing deployment identities for Windows/WSL onboarding.
All C01–C09, DBG01–DBG16, UI01–UI18 and PKG01–PKG14 scope remains unchanged.

Not authorized here: completion/DAP/GUI implementation, new remote interactive PTY
capabilities, native Windows workspaces, daemon/reconnect persistence, third-party
worker plugins, arbitrary deployment hooks, privilege escalation or a new remote
filesystem protocol replacing SSH/SFTP wholesale. Their absence does not excuse
missing WK behavior or proof obligations.

## 12. Research references and evidence limits

- [SSH connection protocol](https://www.rfc-editor.org/rfc/rfc4254): authenticated
  multiplexed channels and remote execution; preserve the existing security boundary.
- [OpenSSH extensions](https://raw.githubusercontent.com/openssh/openssh-portable/master/PROTOCOL):
  rename/fsync primitives do not supply Strop's whole conditional-save transaction.
- [Rsync process architecture](https://rsync.samba.org/how-rsync-works.html): an on-demand
  remote executable over SSH need not be a permanent daemon; not a transaction oracle.
- [Zed remote deployment](https://zed.dev/docs/remote-development): matching static Linux
  worker artifacts and upload-over-SSH precedent; its reconnect daemon is not adopted.
- [Ansible module architecture](https://raw.githubusercontent.com/ansible/ansible-documentation/devel/docs/docsite/rst/dev_guide/developing_program_flow_modules.rst)
  and [Kitty bootstrap](https://raw.githubusercontent.com/kovidgoyal/kitty/master/docs/kittens/ssh.rst):
  shipping scripts is a legitimate deployment approach, not a reason to duplicate
  Strop's transaction policy after the user chose native unification.
- [Mosh](https://mosh.org/#techinfo) and [9P](https://9p.io/magic/man2html/5/intro): useful
  state/identity/session lessons, not substitutes for admitted filesystem effects.
- [0035](0035-remote-workflow-roadmap.md), [0036](0036-remote-workspace-execution.md),
  [0037](0037-devcontainers-and-workspace-contexts.md), [0040](0040-remote-editing-and-saving.md),
  [0054](0054-unified-filesystem-workspace.md), [0055](0055-embedded-terminal-tui-and-gui.md),
  [0056](0056-architecture-prerequisites.md), [0057](0057-core-verification-and-assurance.md)
  and [0002](0002-build-and-release.md): retained product/identity/evidence contracts.

No worker implementation, performance comparison or new formal proof was performed
for this handoff. The preceding bounded Python probe and source/primary-document
research are evidence for the decision only; WK01–WK20 require the real implementation.

## Landed slices

### WK01/WK02 contracts and wire: landed (2026-09-17)

`docs/worker-protocol.md` records the accepted AR/VF baseline identity,
the full strop-remote/strop-fs caller inventory, the protocol type
families, capability vocabulary, deployment consent/policy contract and
the edge-owner no-recursion list. `crates/strop-worker-protocol` is the
one bounded versioned wire: Content-Length framing (8 KiB header /
1 MiB body), class-tagged bodies (JSON control; binary 256 KiB stream
chunks), incarnation-keyed handshake (Session{incarnation,lease},
NamespaceIdentity, limits, capabilities incl. NotifyCoverage), typed
families (observe/list/read, prepare/apply/verify, exec, subscribe —
notify is first-class with overflow/reconcile-boundary events), the
workspace outcome taxonomy reused without duplication, and
guard::Authority enforcing prepared-authority semantics (restarted
workers reject old sessions/handles/subscriptions; no mutation replays
after connection loss). Evidence: 31 codec tests incl. loopback
round-trips, stale-incarnation rejection, wire-shape pins, bound
refusals.

### WK03 kernel extraction: landed (2026-09-17)

strop-fs is now the shared production kernel: pure handlers driven
in-process under one admitted `ExecutionContext` whose NamespaceView is
data (Native/Remote/Container), never adapter imports — the
strop-fs → strop-remote/strop-containers edge is gone (cargo tree
evidence). Receipts bind principal+incarnation to the calling context,
so a restarted worker/editor cannot apply an old session's prepared
operation. The exec supervisor is ported to Rust in strop-worker
(setsid launch with pgid reservation, typed launch classification —
chdir/not-found/not-executable never disguised as exits, half-close vs
revoke, TERM/grace/KILL, group reaping). WK14 later removed the
test-only Python-compatible nonce record; typed `exec_exit` owns
terminal status on the worker wire. Porting found and fixed two real
deadlocks (handshake fd retention, grandchild pipe inheritance).
Evidence at landing: strop-fs 15, strop-worker 14 tests green.

### WK04 local worker mode: landed (2026-09-17)

`strop --worker-stdio` serves the protocol (early entry before CLI/TUI,
protocol-only stdout, bounded private stderr); `strop-worker-client`'s
Worker lease spawns the matching executable (current_exe, never PATH),
readiness is the handshake (version/target identity check, typed
mismatch), retirement at last-owner close, cancellation through the
lease, kill = typed failure with bounded captured stderr — never a
silent fallback. The engine's local list/prepare/execute/verify/observe
flow through the client (editor/namespace.rs), with Buffer::from_read
taking observation evidence explicitly. Wire amendments ratified:
ResultOutcome::Failed{FsFailure} (domain failures never blur with
protocol refusals), Prepared{steps,refused} (review-UI parity), and an
additive Prepare environment override (the per-session trash root must
cross). Deferred per plan: `:w` save parity (WK09 Store intent), wire
exec env/PTY requests (WK10/WK12). Evidence: engine open/edit/save/undo/
Directory parity through the real codec, worker-kill smoke
(stale incarnation fails closed, no file created), worker stdio
integration tests.

### S7 filesystem notifications: landed (2026-09-17)

The full stack: strop-worker's NotifyManager is a thin direct
inotify(2) binding (zero new dependencies; the `notify` crate was
rejected on evidence: it degrades IN_Q_OVERFLOW to a pathless Rescan
flag and queues through an unbounded internal channel) — one fd per
subscription, generation-stamped identity, parent/name guard watches,
cookie-paired rename classification with an explicit Ambiguous class
(never a guessed relocation), and every loss/partial-coverage outcome
latching a conservative rescan obligation over bounded queues. The
editor subscribes the scope root at service start (never at
construction, never under replay): hints land in a bounded NotifyQueue
as AR06-legal coalescing wake hints; clean documents reload through a
guarded path (binding + revision + disk-stamp re-checked at
publication, own-save echoes dropped, mid-flight hints re-observed
once); dirty documents gain external-change state ("changed on disk;
the buffer keeps your edits (:w! forces)") that only a confirmed save or
guarded reload clears; directory buffers reobserve; Git signs revoke
and recompute lazily; overflow/lease loss triggers the conservative
rescan; dead incarnations refuse stale-generation events and resubscribe
fresh. Picker freshness (0063's residual): SourceWorker caches per scope
— retained SymbolIndex/catalog entries stat-validate on reuse, hinted
subtrees rescan, cancellation restores the snapshot; no subscription
means byte-identical pre-0058 full scans (honest degradation, no
invisible crawl). Model: specs/Notify.tla — eleven invariants
(HintsNeverAuthority, StaleIdentityNeverActs, AmbiguousNeverRelocates,
FreshRequiresCoverage, CoalesceNeverLosesStaleness, ScanNeverErasesNewer,
DirtyNeverClobbered, StaleReloadNeverClears, CoverageHonest,
BoundedQueue, TypeOK), nine kept mutants each killed by exactly its
named property, fifteen witnesses, exhaustive main check in ~5s
(two-client calibration documented; the model has no cross-client
action); chained into specs/gate.sh via notify-gate.sh. Evidence:
strop-worker notify 15 real-kernel integration tests, engine
subscription/reload/dirty/overflow/staleness tests, picker incremental
e2e through the real pipeline.

### WK05/WK06 artifacts and deployment: deployed; worker-owned GC native-tested on Linux

The release catalog carries the worker compatibility manifest (protocol
version, minimum editor version, per-target artifact facts), generated
from pinned source constants in the release workflow — `verify` rejects
worker-fact drift. `strop-worker-deploy` owns the deploy/cache state
machine: Decide (compatibility → Compatible/Fallback) → StageLocal
(hash-verified, 256 MiB bound) → ResolveCacheRoot (private 0700,
symlink-free, foreign entries refused — never chmod-into-compliance) →
CheckCache (verified-hit reuse without fresh consent; first upload to
an endpoint requires explicit consent) → Upload (uniquely-owned staging
via authenticated providers) → VerifyTransfer (read-back hash+size) →
Publish (atomic, content-addressed objects/<sha256>) → VerifyObject
(re-hash at the final path + owner-exec mode — verification binds to
the executed object) → receipt → Probe (short-lived real handshake,
identity mismatch refuses) → retire the probe → connect the actual
worker, which writes its own session-specific cache lease before
Welcome and before editor readiness is published. A verified probe
is not itself a live editor worker. Interruption removes only
positively-owned staging and reports honestly
(PublishedNotReady ≠ Probed ≠ live ready); offline with no local
supply is a typed refusal with zero endpoint contact. The provider
trait admits only
put/get/rename/chmod/stat/readdir — no PATH/rc/image/glob-delete
operation is representable. The obsolete unlocked deployment-side GC
and policy-only tests were deleted. The actual admitted worker now
collects scoped old objects/receipts over the framed protocol, under
the same OS lock that guards new worker lease registration. Its own
private matching release receipt binds the selected context. Real
two-worker processes, Python-free SSH and container provider/editor
journeys verify the current object and every live lease survive while
an old unleased object retires. The generalized lock/snapshot
induction now passes; native macOS/arm execution remains a separate
WPLAT-NATIVE release gate.
Evidence: hermetic deployment/interruption and real worker cache
fixtures, plus catalog wire-shape pins in tests/release-catalog.sh and
tests/install.sh.

### WK07 SSH worker transport: landed (2026-09-24)

OpenSSH bootstrap and worker transport are in strop-remote: ONE fixed
audited POSIX discovery line (uid/uname/cache root — no Python, no
PATH/rc/image mutation) with a single tested quoting boundary; the
transport spawns `ssh … --worker-stdio` and readiness is the framed
handshake (shell text can never impersonate a Welcome), every reconnect
a fresh exec + fresh incarnation. Deploy rides WK06's state machine
through SftpDeployProvider — a dedicated SFTP connection over the
existing v3 codec (extended for lstat/mkdir/write/setstat/rename with
posix-rename@openssh.com when advertised, bounded read-back, poisoned
closes). Handshake identity binds the exact build target
(TARGET_TRIPLE captured at build time; `connect_deployed` admits
cross-platform workers while version+protocol bind to the release).
Consent flow: browsing never deploys — read-only arms ride the SFTP
path byte-identically until the first reviewed filesystem mutation,
which IS the authorized worker-using action (recorded on the receipt);
restricted/SFTP-only hosts get a typed refusal, never an alternate
write path. The engine's SSH workspace I/O routes through RemoteWorker
where admitted (the namespace-translation boundary is typed and single).
Evidence: worker_ssh 4/4 over real sshd (deploy→handshake→
read/write/notify parity, consent gating + quiet reuse,
interrupted-deploy cleanliness, SFTP-only refusal), remote_ssh suite,
110 strop-remote lib tests. Found and handled: a trace file inside a
watched directory self-hints every record — subscriptions suppress
under tape record/replay (the product-level fix belongs to the
trace/notify owners).

The separate `ssh-pythonfree` image uses the pinned Alpine builder
without installing Python, adds only OpenSSH, and runs the same
deploy→handshake→read/write/notify parity fixture over a real local
sshd. `! command -v python3` gates its build; `python`, `python2`
and `python3` were also absent in the executed image. The CI gate
repeats this distinct remote-host portability check.

### WK08 container worker deploy: landed (2026-09-24)

Container worker deployment/exec/cleanup rides the captured AR07
context end to end. strop-containers gained scoped transfer/exec
primitives: shell-free lstat off the tar stream's first header
(symlinks reported, never followed), tar-in writes stamped with the
selected principal's numeric uid/gid (the daemon's root extraction
never manufactures root-owned caches), image-platform inspection, and
an admitted shell-free worker channel for the preinstalled case.
strop-worker-deploy's ContainerProvider implements the DeployProvider
over those primitives — consent-gated deploy, verified-object
activation handshake, lease-aware cleanup — and hands the engine a
Worker lease that re-admits the incarnation on every connect. Gated
live-engine evidence (STROP_CONTAINER_TESTS=1):
deploy+handshake+read/write/notify inside a real container with the
cache object proven principal-owned 0500; lease close reaps the
in-container worker; a restarted container is a typed stale-incarnation
refusal at provider and lease; a FROM-scratch preinstalled worker
serves byte-identical reads with zero deploy writes; shell-less deploy
and read-only rootfs are truthful classified refusals. The lane caught
and pinned two real bugs: the daemon reports a missing sh on stdout
under -i (AR07 consulted only stderr), and ranged worker reads
announced the full file size while streaming the range. The full
namespace-dispatch migration of editor consumers stays with WK09 per
plan sequencing.

### WK17/WK18 proof and correspondence: partial

The normalized WorkerSession and WorkerDeploy TLC graphs and matched
negative controls remained intact while TLAPS discharged 274 and 565
inductive obligations respectively. The shipped Verus kernels include
framing, session admission, recovered-Store receipt/namespace admission,
terminal delivery, deploy catalog/content-address/activation and
lease-aware keep decisions. The recovered verifier takes its
`Unconfirmed` premise from the actual `StepReceipt`, not the request
name: a real worker regression first reproduced a committed receipt
wrongly entering recovered verification, then confirmed refusal. The
deploy receipt-to-activation path likewise reissues mismatched receipt
facts and refuses on a post-write loss; hermetic provider failures and
a real OpenSSH/Python-free lane exercise their separate I/O premises.

The locked workspace gate exposed a native read-stream fault:
an exact-length range enqueued `last=true` and looped once more to
enqueue a second terminal marker. The client's first marker retired
the stream; the second could poison its session as an unknown stream.
`read_streaming` now returns after that final chunk, and the loopback
journey checks a live health request before its next read. This is a
real codec/worker/client correspondence fix, not a preview exception.

The inventory checker re-pinned the real `worker-serve` evidence after
this fix. `remote-save` and `worker-recovery` also own the shared
`serve/fs.rs` file; their pins needed an explicit reviewed `--force`
for this read-only-stream change, which did not alter either boundary's
Store or recovery decision. `worker-protocol` needed two reviewed
forces for unrelated core-module edits: rustfmt's declaration ordering
and the shared cache-record module registration. Neither changes the
protocol boundary; live-lease evidence is in `worker-serve` and
`worker-deployment`.

The cache cutover adds only `Worker::collect_cache` and
`CacheGcOutcome` in `strop-worker-client/src/lib.rs`; existing Store
and recovery client methods and receipts do not change. It also adds
the distinct `CollectCache`/`CacheCollected` protocol variant with
Standard scheduling class in `strop-worker-protocol/src/request.rs`,
without changing the exec request/result or its supervision class.
`remote-save`, `worker-recovery` and `exec-supervision` own those
shared files but their named behavior/evidence did not change; the
reviewed `check.py --stamp --force` for these three rows rebinds only
this additive unrelated source drift. The new cache behavior has
separate `worker-deployment`/`worker-protocol` evidence.

File-size discipline: `strop-worker-client/src/lib.rs` had crossed the
800-line ceiling, and `connection.rs` was at 822 lines. The unchanged
exec/PTY client API and its incarnation-bound controls now live in
`exec.rs`; inbound result/event/chunk routing and its 64-chunk budget
live in `connection/reader.rs`. The root modules retain their API and
connection state; LSP references identify every `exec`, `exec_pty`
and control caller. Real loopback exec, saturated PTY and large read
journeys passed after the move. `remote-save`, `worker-recovery` and
`worker-deployment` only lose unrelated exec code from their shared
`lib.rs`; `exec-supervision`, `worker-streams` and `worker-serve`
have their new owning source files registered without changes to the
named test bodies. Reviewed `--force` re-pins those six rows for a
code-only extraction; no safety claim or kill was transferred solely
by the new path.

The real OpenSSH journey reproduced another correspondence defect:
deployment had persisted the short-lived probe session's lease while
the editor's subsequent live worker had no matching cache record.
The cached worker now records its own lease before Welcome and removes
it on orderly teardown; the editor's SSH/container admission waits
for this actual connection before publishing ready. Reconnects mint
new records, while an interrupted worker may leave a conservative
stale record. Probe cleanup and real SSH/container lease assertions
are new correspondence evidence; later worker-owned collection is
separate from this live-lease admission repair.

The real-binary test then exposed a related gap: an object unlinked
after exec but before Hello could still send Welcome because the
Linux `current_exe` basename gained `(deleted)` and bypassed the
cache-lease branch. It now refuses the missing/wrong final inode
before Welcome, and pre-handshake Error/Bye reaches the waiting
client rather than timing out after the worker already refused.
`exec-supervision` and `worker-streams` share the client connection
source; their reviewed forced pins record only this pre-Welcome
routing edit, not a change to post-Welcome exec/stream behavior.

Native retirement now runs only after the selected worker's live
handshake, before its SSH/container editor lease is published. The
worker verifies its own context, executable receipt and final inode,
locks the persistent private cache, observes all bounded lease records
and unlinks only unpinned content addresses with same-context receipts.
The actual two-worker/two-build test preserves the second live
executable; other cases cover foreign context, corrupt records,
cross-process lock exclusion and a deleted object before Welcome.
The real SSH editor journey seeds a second, old object and its exact
endpoint receipt between two separate `:remote worker` admissions,
then checks retirement without granting a file-write permit. The
container editor similarly seeds an old object before
`:container-worker`; direct SFTP/ContainerProvider journeys cover
their transports. TLC checks the refined two-client
lock/snapshot/retire graph and four semantic negative controls. TLAPS
proves the 447-obligation generalized induction, its derived safety
and three additional conditional retirement theorems; the matched
foreign-context negative proof fails as required. Those model proofs
do not establish OS lock behavior or native macOS/arm execution.

### WK20 scoped static worker, UI and TUI measurements: local Linux only

On the WSL2 Ryzen 9950X3D, clean pre-worker `a05d84f` and the
sampled pre-macOS-fix stripped 47,373,648-byte x86_64 musl worker (`sha256
67f7e15dec483ddc4926808b4352824b75fb7d470969f022a5ada2a98c86b130`)
each ran eight warmups and 64 real Hello/Welcome launches (pre-worker
protocol 1; native worker protocol 2). Baseline/current readiness p50
was 0.731/0.808 ms, p95 0.866/0.954 ms, p99/max 1.115/1.459 ms;
the binaries were 46,226,704/47,373,648 bytes. The sampled worker
completed eight warmups and 64 serial Health requests through
real framed IPC: write+flush to result p50 0.203 ms, p95 0.275 ms,
p99/max 0.320 ms. Neither protocol-different warm launches nor these
samples establish a speedup.
`verification/bench_worker.py` and
`verification/bench_worker_roundtrip.py` pin raw samples, artifact
digests, RSS/threads and request bytes in `verification/measurements/`.

The same clean pre-worker and sampled worker artifacts each ran eight
warmups and 64 real `--ui-stdio` committed-text actions after the
editor opened a 10,000-line file through one live worker at 120×40.
`verification/bench_ui_input_frame.py` checks that each complete
semantic-view frame visibly contains the next edit on line 5000.
Baseline/current write+flush→view p50 was 0.226/0.235 ms, p95
0.297/0.330 ms, p99/max 0.427/0.352 ms. The higher current p95
is recorded, not called a no-regression result. Raw samples, framed
bytes and observed worker/editor RSS and thread counts are archived.

On the same two static artifacts, the real 120×30 TUI opened the
10,000-line worker-backed file and completed eight warmups plus 64
single-character edits. The opt-in
`terminal_editor::native_terminal_input_to_painted_frame_samples`
waits until the VT100-decoded **cell grid** displays each exact edit:
baseline/current key-write→paint p50 1.065/1.047 ms, p95
1.865/1.626 ms, p99/max 1.913/2.850 ms. These Linux Docker-on-WSL2
samples do not establish no regression at the higher current p99.

The schema-5 diagnostic freeze checks artifact, fixture, method and
raw-percentile bindings, but records a dirty worktree; `--check`
refuses release qualification. LSP sync/request, terminal output under
load, cold/warm SSH/container deployment and transfer/retirement
high-water marks remain unmeasured. PR native CI run
[`36243016093`](https://github.com/stropdev/strop/actions/runs/36243016093)
found macOS exec/PTY `Lost` exits and missing nested `nvim` on both GNU
runners. After provisioning that test fixture, run
[`36244126275`](https://github.com/stropdev/strop/actions/runs/36244126275)
passed native x86_64 and aarch64 GNU Store, worker client and editor
journeys. Both macOS runners still failed the direct supervisor test:
their zombie-only `kill(-PGID)` returned `EPERM`. The guarded `libproc`
repair above has not yet passed native Mac CI, and the passing GNU run
predates this source change. Mac/arm final-candidate qualification
and full performance remain blocked.

The same-source proofs do not verify OS effects, exact receipt
provenance, a global liveness oracle or platform performance.
`WDEP-GC` now has a serialized native caller, two-worker/SSH/container
OS correspondence, exhaustive bounded model and generalized TLAPS
induction. `WPERF-FULL` and `WPLAT-NATIVE` still require product-path
measurements and actual native target execution. WK16–WK20 and VF20
remain open; 0059 must not start on partial evidence.
