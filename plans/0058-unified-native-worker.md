# 0058 — Unified native worker: local first, the same protocol remotely

Status: **authorized standalone implementation and verification handoff**, after
[0056 architecture](0056-architecture-prerequisites.md) and
[0057 core verification](0057-core-verification-and-assurance.md), before completion.
Those two foundation releases have already been dispatched. **Do not stop, fold
this work into, or silently change the candidate being verified by that agent.**
This document records a future release contract, not a shipped worker or passed proof.

```text
0054 filesystem -> 0055 TUI terminal -> 0056 architecture -> 0057 core verification
    -> 0058 unified native worker + assurance migration
    -> 0059 completion -> 0060 debugger -> 0061 GUI (+ 0062 distribution)
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

Cache garbage collection respects live executable/worker leases and concurrent client
versions. Keep cleanup bounded and scoped by receipts, never glob-delete unrelated
files. Disk full, read-only/noexec cache, wrong architecture, corrupt artifact, failed
rename/sync, concurrent cache replacement and interrupted launch have explicit outcomes.
No guarantee assumes a hostile same-principal process or compromised host reports truth.

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
