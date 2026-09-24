# 0057 — Whole-core verification and assurance before further features

Status: **dispatched, separate mandatory verification release** immediately after
[0056 architecture closure](0056-architecture-prerequisites.md). No new theorem,
model run or implementation is claimed by this document. The user explicitly wants
strong proved boundaries across the editor, local/remote filesystems, containers,
the deployed Python helpers and their interaction with the editor—not only protocols.

The implementation sequence is:

```text
0054 unified filesystem -> 0055 terminal -> 0056 architecture closure
    -> 0057 whole-core verification
    -> 0058 native worker + assurance migration
    -> [deferred last:] 0059 completion -> 0060 debugger -> 0061 GUI (+ 0062 distribution)
```

The user has dispatched architecture and this verification release. Complete the
actual pre-worker candidate, including its shipped Python helpers. The subsequently
authorized [0058 worker](0058-unified-native-worker.md) follows this release and owns
its native cutover and assurance changes; **do not retarget or interrupt this run**.
Completion remains unimplemented and waits for both this baseline and the worker
release. See §14 for the explicit handoff to an agent already working on verification.

## 1. Meaning of “verified core”

**One release candidate, explicit boundary contracts, mechanized claims over actual
production decisions, and concrete evidence that real effects obey those decisions.**
A passing wire model, a few proved arithmetic functions or a green ordinary suite
alone is insufficient.

“Verified” is always qualified by the claim, admitted domain, source revision,
proof method, assumptions and runtime evidence. We do not claim to prove the OS,
CPython, Ropey, OpenSSH, Docker, a GPU driver, a terminal emulator or arbitrary user
programs. Those are named trusted/effect boundaries with required conformance and
native evidence, not a blanket exclusion for the code that actually does the work.

Every load-bearing owned boundary must have a proved admission/decision contract
where specified below, and no promised effect may rely on an unexamined observation,
parser, wrapper or helper. An unproved connection stays labelled unproved. If a
required proof or runtime correspondence fails, fix the source/contract and rerun;
do not relabel it as optional or postpone it to completion/debugger/GUI.

The architecture release remains responsible for implementing correct APIs, recovery,
lifecycles, supervision and the UI server with its existing test/model/proof gates
green. This separate release owns the **expanded** whole-core assurance programme,
including new TLAPS and core-assurance lanes. It is not permission for architecture
to land untested code or regress existing proofs.

## 2. Candidate freeze, scope and ownership

Start from a recorded completed 0056 candidate and the final filesystem/terminal
implementation—not stale research snippets or partially edited source. Capture the
commit/tree, lockfile, feature/platform matrix, embedded helper sources, model/proof
inputs and toolchain pins. New product features wait while the core is qualified.

Verification may expose defects or require narrow extraction of pure production
kernels, checked wrappers or instrumentable synchronization. Those fixes are part
of reaching this release's acceptance. If a fix changes an architecture contract,
coordinate it with the 0056 owner, update its acceptance and re-freeze the candidate.
Do not keep proving an obsolete tree or introduce a second implementation solely
because it is easier to verify.

One assurance integration owner maintains a machine-readable boundary/claim inventory
under `verification/` and a readable guarantee ledger derived from it. The inventory
selects required targets; incomplete path filters cannot silently omit a boundary.
Each row identifies:

- observable promise and explicit exclusions;
- authoritative input/state, owner/namespace/revision and admission boundary;
- actual production types/symbols and every caller/effect path relying on the claim;
- theorem/model/proof target, parameters/bounds and caller-established premises;
- parser/observer/executor/foreign-runtime trust assumptions;
- trace/refinement/native cases, calibration mutants and raw result artifacts;
- status: specified, bounded-model-checked, proved kernel/theorem, correspondence-
  tested, native-tested, blocked or superseded. These are not interchangeable.

The inventory covers **all current code that can mutate text/state, grant authority,
perform I/O, preserve/recover data, publish async results or report an outcome**.
A new boundary missing from the inventory is a gate failure. Ordinary formatting
helpers need not receive ceremonial proofs, but they cannot hide an effect/admission
boundary under a harmless name.

## 3. Required verification-release ledger

All VF requirements belong to this release. The domain chapters below specify what
must be proved and what must be checked at the actual effect boundary. No single
percentage, test count or “all protocols green” substitute satisfies the ledger.

| ID | Required result |
| --- | --- |
| VF01 | Complete whole-core boundary/guarantee inventory, reviewed human semantics and exact candidate/source/tool binding. |
| VF02 | Proved production text-mutation geometry/authorization and batch-map composition, with concrete byte/history/revision/anchor correspondence. |
| VF03 | Verified input/preview/mutation ownership and readonly presentation contract across real current TUI/field/terminal/UI-server routes. |
| VF04 | Source-backed collection/projection/history/workset integrity, including protected rows, grouped edits, relocation and partial outcomes. |
| VF05 | LSP/background-service source binding, synchronization/publication/cancellation and boundedness assurance beyond message framing. |
| VF06 | Local filesystem/save/operation authority, native-path/parent/lock observations, no-clobber publication and truthful failure/receipt behavior. |
| VF07 | SSH/SFTP/remote-resource identity, wire/read/follow lifecycle and no local fallback, with real authenticated transport evidence. |
| VF08 | Exact Rust↔embedded Python supervisor/codec/lease correspondence using the actual shipped source, not a mock or trusted opaque helper box. |
| VF09 | Remote save and filesystem Python effects through editor reconciliation: baselines, metadata, locks, commit/verify/cancel/uncertain outcomes and preserved drafts. |
| VF10 | Actual running-container context/incarnation/exec/PID/source/lease/cleanup assurance and capability refusals. |
| VF11 | Native process and bounded-queue/service coordination: ownership, cancellation, wakeup, deadlines, retirement and shutdown. |
| VF12 | Completed terminal session/input/PTY/effect/projection boundaries, with native/VT evidence and explicit third-party emulator limits. |
| VF13 | Generalized draft/cohort recovery safety, actual persistence/restore/cleanup correspondence and no implicit source overwrite or restored authority. |
| VF14 | Full UI-stdio/client/view/input/effect ordering and stale-data authority, native stdio/WSL closure and semantic parity with the real engine. |
| VF15 | Configuration/trust/privacy/identity admission across serialization, capture, replay, recovery and all relevant callers. |
| VF16 | Current TUI install/update/release identity, artifact verification, activation/rollback/cleanup and public outcome correspondence. |
| VF17 | Calibrated TLA+/TLC models and required generalized TLAPS safety theorems for the load-bearing state/effect boundaries. |
| VF18 | Same-source Verus proof programme over the required shipped kernels and their checked admission, with audited trust and proof erasure. |
| VF19 | Attributable model/kernel/seam mutants, production correspondence, real synchronization Loom, byte/fuzz/fault and native campaigns. |
| VF20 | Required exact-candidate PR/tag gates, complete raw assurance evidence and an explicit qualified-core handoff to future features. |

VF01–VF20 must be complete before 0058 worker starts; 0059 completion follows both.
An exception requires
explicit user approval, exact claim/impact/re-entry scope and a roadmap record.
“Difficult to prove”, a missing tool or an unavailable test host is a blocker—not
permission to silently replace a required theorem with tests or shrink the claim.

## 4. VF02–VF05: editor mutation is a first-class proof target

### Mutation gateway and actual bytes

The current anchors are `strop-core/src/buffer/mutation.rs`, `editmap.rs`, history,
`editor/transact` and the admitted action/view boundary closed in 0056. Verify the
**actual** PreparedReplacements/UserEdit/SystemEdit publication path and consumers.

Required proved decisions/geometry include:

- accepted user edits belong to the correct live buffer incarnation/base revision
  and have the required write authority; generated-view update authority cannot
  be used as unrestricted source-write authority;
- ranges are well-formed, UTF-8-boundary-admitted, in-bounds and non-overlapping
  under the documented ordering/affinity; validation failure grants no mutation;
- checked arithmetic prevents revision/ID/range overflow from becoming authority;
- prepared payload/geometry correspondence is preserved through sorting and apply;
- **batch anchor mapping composition** left open in 0045 is proved against the real
  validated-batch application order, not only a single-edit helper;
- byte-level splice/plan semantics preserve untouched bytes and insert exactly the
  admitted replacement bytes, with explicit Ropey/runtime assumptions.

Run all actual mutation routes against independent concrete observations: typing,
operators, Visual/block/multicursor, Ex, undo/redo, shell filtering, existing LSP
changes, search replacement, collection edits, Git discard and system projections.
Check text, dirty/saved state, revisions, history/grouping, selections/marks and
syntax/service publication together. A rejected whole batch changes none of them.
A low-level proof does not excuse a caller bypassing history, readonly checks or
service invalidation.

Distinguish in-memory validation/publication atomicity, per-file disk publication,
multi-document partial outcomes and crash recovery. A normal undo group is not a
power-loss transaction. Do not resurrect an old blanket NoPartialCommit claim when
the actual model excludes crash/durability behavior.

### UI input, preview and readonly rendering

Prove the production routing/admission policy selects one valid input owner and
that stale source/view/session identities cannot authorize an edit. Exercise the
actual TUI, fields, pending operators, clipboard, completed terminal and UI-server
input paths—not a fixture that calls the mutation kernel directly and skips routing.

Preview and execution consume the same resolver/validated target. Check resolver
fidelity against pinned Neovim corpora with named divergences and intermediate
composition/cancellation checkpoints; do not claim a complete formal proof of Vim
from those differential cases.

Repeated readonly presentation without model/viewport changes cannot start native
work, alter source/history/cursor ownership, consume a proposal or mutate a binding.
Use the actual renderer/query surfaces, effect observations and compile-time access
restrictions. A function signature alone does not exclude hidden interior mutation.

The byte↔UTF-16↔logical-cell mappings used by current sources/fields/projections
are checked/proved at their pure boundaries and exercised through real input.
Future native GUI pixel/IME adaptation is not falsely included in this baseline.

### Collections, generated views and history

Prove the pure source/projection admission/mapping contract: writable spans resolve
to exactly their source/base, protected headers/gaps do not become source text,
repeated/overlapping excerpts do not duplicate a mutation and coordinates cannot
cross into another source or namespace.

Exercise grouped apply/undo/redo/save through real source documents and receipts.
Use multiple sources, independently edited members, readonly/remote sources, source
rename/deletion and lost/late replies. Rejection retains the group evidence; a
partial outcome is identified per member rather than hidden behind a success count.
Search inclusion, collection promotion and review act on the same defined set.
Generated text is never an independent writable copy of the authoritative source.

### LSP and background service integration

Model/prove the owned binding/request/synchronization decisions: same revision in
two documents is not identity, epoch zero is valid, close/reopen changes incarnation,
required didOpen/didChange ordering precedes requests and late/cancelled results
cannot publish into a new owner. Bound actual unsent snapshots, requests and
retirement—not only the visible popup/request slot.

Real server and controlled-peer campaigns cover startup/failure/restart, Unicode
changes, source/view relocation, external headers, diagnostics/navigation, requests
held behind synchronization and cancellation that the peer ignores. This verifies
the current LSP/service core before completion increases traffic; the completion
provider/query semantics themselves remain 0059 feature work.

## 5. VF06–VF10: filesystem, remote and container authority

A proved policy fed invented facts is not a proved filesystem boundary. Each claim
must include its path/metadata observer, checked admission, effect executor and
editor reconciliation. Core models and real native/fault campaigns cover the same
before/after boundaries.

### Local files and operations

Use the final `strop-fs` guard/observation/stage/local/batch modules and normal buffer
save path from the completed architecture candidate. Prove the pure authority/
plan/outcome decisions; test actual descriptor/path/metadata inputs and filesystem
calls that supply those facts.

Required cases include native-byte names, non-UTF-8/controls, case/normalization
aliases, final/ancestor symlinks, replaced parents, lock identity/link count/owner,
reserved namespaces, missing parent chains, occupied destinations, different mounts,
metadata preservation and operation cancellation before/after publication.

Create/copy/rename/move/remove/Trash and save behavior must preserve their stated
no-clobber, ownership, content and metadata guarantees. A plain rename/exists check
is not a no-replace proof. Qualified cooperative locking is not universal CAS against
nonparticipants. Keep counterexamples to unclaimed guarantees separate from bug
mutants, and never weaken the source policy to make a stronger false claim pass.

Verify the real editor side too: DocumentId-stable relocation, dirty descendants,
in-flight saves, unconfirmed outcomes, old LSP/search/projection replies, close/view
changes and recovery records. A committed native mutation is reconciled once even
if its view is stale; an uncertain result does not authorize retry, rollback or a
write to the old path.

### SSH/SFTP and remote filesystem reads

Preserve/extend SftpWire, RemoteRead, RemoteProcess and RemoteWorkspace models with
explicit actual-code mappings. Check request identity, packet/length bounds, native
filename decoding, read completeness, range/tail/follow state, connection/binding
incarnation, cancellation/backpressure and final outcome ownership.

Use real SSH/SFTP fixtures with private host keys and exact namespace/resource
assertions. Unknown/mismatched host identity, unsupported extension, partial/failed
read, replaced files/directories and delayed old streams produce named outcomes.
Remote paths cannot be reinterpreted as local files or a different endpoint's data.
OpenSSH/cryptography remain trusted dependencies; base64 and correlation nonces
are not secrecy or authentication mechanisms.

### The Python supervisor/editor boundary is mandatory

Inspect and execute the **exact shipped helper bundle**, including:

- `strop-remote/src/exec/spec.rs` and the embedded source in
  `exec/supervisor.rs`;
- `protected.py` plus `save/helper.py`, as actually concatenated by save/protocol;
- the final `filesystem/{main,observe,mutate}.py` bundle and its Rust codec/callers;
- the equivalent completed container lease helper and its admission/result bridge.

Record bundle/source digests and protocol versions in the candidate evidence.
Do not test a simplified Python copy, only one concatenated fragment, or a mock
returning “written.” The code that ships must be the code the campaign exercises.

The assurance chain is explicit:

```text
editor owner + frozen source/authority
 -> Rust checked request/bytes and selected execution context
 -> actual Python decode/admission/observer
 -> actual child or filesystem effect and lease/durability boundary
 -> framed/status response and native process outcome
 -> Rust checked decoding/identity/outcome classification
 -> editor binding/draft/history/recovery/UI reconciliation
```

Prove the owned Rust admission/encoding/decoding/decision kernels where specified,
model the helper's abstract control/effect transitions and systematically check
actual Python correspondence at those boundaries. The Python interpreter/helper
effect driver is **not proved by Verus**; name precisely which parts are modelled,
correspondence-tested or trusted. Do not hide the whole helper under an opaque
trusted function whose assumed postcondition is the property being claimed.

Required cross-language cases:

- canonical binary/JSON lengths, versions, absent versus null fields, trailing/
  duplicate/malformed data, integer limits and complete payload consumption;
- native argv/cwd bytes survive Rust encode → shell transport → Python decode →
  actual exec unchanged, including quotes/newlines/non-UTF-8; no data becomes code;
- selected interpreter/endpoint/user/working directory and fixed helper identity
  remain bound; unusable shims/versions and invalid overrides do not choose another
  runtime silently;
- anchor/worker creation, readiness, stdin lease close, backpressure/hangup, worker
  exit versus cancellation, descendant cleanup and revocation-before-reap follow
  the declared ownership model;
- correlation markers cannot be misrepresented as cryptographic authentication.
  Separate trustworthy control/outcome observations from child/program diagnostics
  according to the actual threat model; test collision/forged-record handling and
  preserve the documented limitation for a worker deliberately echoing a public nonce;
- a malformed/failed/ambiguous helper reply cannot grant a permit, mark text clean,
  update a different binding or turn an uncertain effect into “unchanged.”

### Remote saving and filesystem effects

Use real helper invocations over the actual authenticated transport and editor
handlers. Check baseline content **and** identity/metadata, descriptor-pinned parent
and lock revalidation, exact stored spelling, private staging before receiving bytes,
complete attribute capture/restoration, payload/digest agreement and both pre-commit
and post-commit failures.

Faults must occur at actual semantic boundaries: before/after write, metadata change,
file/directory sync, namespace publication, response delivery and cleanup. Preserve
edited text and attempt evidence across lost acknowledgement or cancellation after
commit. Verify compares before/intended/observed states without blindly repeating
a mutation. Same-size changes, external drift, partial batches and source rename
must not pass by weaker metadata/string comparisons.

Prove the pure commit/authority/outcome classification and model the durable ordering;
exercise CPython/filesystem behavior with real bytes and fault injection. Process
kill tests do not prove physical power-loss durability. OS/filesystem assumptions
and noncooperating-writer limitations remain explicit, not hidden in a helper comment.

### Running containers

Verify actual selected Docker EngineRef/context, canonical container ID + StartedAt,
execution user/cwd, source identity and namespace-local PID interpretation. A changed
default context, restarted container or reused name cannot retarget accepted work.
A container path is not an analogous host path and docker top's host PID is not an
adapter-local PID.

Exercise the real in-container supervisor/lease and program/descendant outcomes.
Killing the local docker client or closing stdin alone is not cleanup proof. Missing
helper/runtime/capability, restricted ptrace/security policy and daemon failures
produce accurate refusals. Tests never restart/remove a user's container or elevate
it to manufacture success; disposable fixtures are uniquely owned and cleaned up.

## 6. VF11–VF16: other load-bearing boundaries

### Processes, queues and service coordination

Prove owned-target/phase/budget/counter decisions and exercise actual process-group,
pipe, reader/writer/wait and cancellation paths. Separate finite admitted work from
long-lived service liveness; settle/shutdown cannot wait forever on a healthy server
or terminal, and a UI destructor cannot wait/reap a child synchronously.

Loom campaigns use the real coordination control flow behind instrumented sync:
publish versus park/unpark, queue capacity/reservation, close/cancel, failure/panic,
one terminal outcome per admitted operation and retirement. Record bounds and any
trusted sync adaptation. No copied “equivalent scheduler” or uninstrumented test
labelled Loom. Native tests retain the OS/descriptor/process boundary that Loom
cannot prove, including held pipes and recycled PID risks.

### Terminal

Verify the completed 0055 implementation as part of the baseline: editor-versus-
child input ownership, escape-prefix handling, ordered key/paste/VT data, one PTY
geometry owner, snapshot/text selection consistency, bounds, child/descendant close,
privacy and host-effect policy. A terminal output/title/OSC message cannot become
unapproved editor mutation, filesystem access, clipboard access or execution.

Use the actual selected emulator and real PTY/outer-terminal journeys, including
nested modal input, Unicode/resize and sustained output. Model/prove Strop's owned
session/admission/ordering decisions; do not claim a theorem over the vendor's
entire VT implementation, fonts or operating system from a grid fixture.

### Recovery and persistence

Verify the real 0056 recovery implementation, not old session-file behavior. Prove
cohort/watermark/publish/retire/restore decisions and generalized safety of repeated
crash/recovery; exercise actual private staging, sync, publication, cleanup and restore.

A durable checkpoint names a complete admitted cohort and source baseline. Cleanup
cannot remove the last necessary recovery evidence; corrupt/unknown/changed source
state fails closed and preserves the draft. Checkpointing is not Save and cannot
clear newer dirty edits. Restore does not overwrite a changed/deleted/rebound file
or deserialize native write/exec authority. Sensitive/remote memory-only policy is
respected throughout capture, on-disk data, UI observations and diagnostics.

### UI server and presentation transport

Prove the actual UI-stdio admission/client-state policy: exact backend/source/view
incarnation, sequence/one-shot apply, valid delta base, bounded data, explicit effects,
correct applied versus durable acknowledgement and safe EOF/resynchronization.
Run complete current-engine journeys through the real server/driver and compare
semantic state/effects with TUI/headless observations. No set-buffer/set-trust mock
backdoor or text-editor-only subset qualifies as current-feature coverage.

Native Windows→WSL stdio is exercised without a GUI. Framing, quoting, startup/user/
distro selection, partial IO, closure and wrong-version/focus/binding outcomes are
real acceptance cases. Native GUI rendering/IME/accessibility remains future work,
not hidden in this baseline's assurance claim.

### Configuration, trust, privacy and IDs

Prove the selected authority/identity/exhaustion kernels and check actual parsers,
constructors and callers. Missing fields, epoch zero, default values, duplicate
keys, overflow and invalid native paths cannot manufacture a valid capability.
Project configuration/restore does not approve itself. A readonly/source reason,
:explain decision and actual accepted/refused action must agree.

Capture classification happens before generic recording and applies to input,
requests, helper traffic, projections/frames, recovery and exported metadata.
Test deliberate secret-bearing fixtures end to end; redacting only the obvious
field is insufficient. Native-free replay cannot execute a helper/process, save,
clipboard effect or network request. Full capture, metadata-only and missing-content
replay claims stay distinct.

### Installation and release

Verify the existing TUI artifact/install/update/catalog foundation from 0056:
exact installed owner/artifact/version, verified bytes before activation, same-target
atomic publication, failure preserving the previous install, scoped cleanup and
truthful rollback/manager routing. A successful download or extracted file is not
a successful activation.

Model/prove the pure ownership/publication decisions and exercise real install/
update interruptions, digest/format errors, changed destinations, disk/permission
failures and activation outcomes. Verify the rendered public version/download facts
against the same immutable release catalog. Checksums, TLS, publisher signatures
and build provenance have different trust meanings. Future Windows installer code
must extend this baseline under 0062; it is not considered already verified here.

## 7. VF17: models and generalized protocol/effect safety

Retain and extend the existing EditorProtocol, ChangePlan, SftpWire, RemoteRead,
RemoteProcess, RemoteWorkspace and RemoteSave models. Add the completed architecture's
recovery, source-binding/filesystem-effect, UI session, terminal/lifecycle and install/
publication boundaries as required by the inventory. Models cover **state/effect
semantics**, not just well-formed messages.

Every model has a reviewed human contract: actors/state domains, authoritative facts,
legal/rejected transitions, linearization points, cancellation/crash semantics,
units/bounds, safety properties, separate progress assumptions, production mapping,
calibration and explicit exclusions. Requirements are independent observable promises,
not a rephrasing of the implementation's current branches.

### TLC

Positive runs exhaust the declared finite instances and all configured properties.
Record constants/state constraints, exploration bounds and deadlock/fairness choices.
Distinct sessions/resources/incarnations and before/after-commit/cancel states must
be reachable where claims require them. Non-vacuity witnesses establish meaningful
success/refusal/uncertainty/recovery and bad-schedule reachability.

Keep attributed counterexamples and targeted mutants. A parser failure, timeout/OOM,
wrong invariant, dead code path or constraint excluding the race cannot count as a
successful calibration. Counterexamples to deliberately **unclaimed** guarantees
are labelled limits, not repaired by claiming the environment cannot do that.

### TLAPS

Required generalized inductive-safety targets include:

- UI/action/publication ownership: correct session/source/base, one-shot apply,
  valid snapshot/delta authority and truthful acknowledgement;
- draft/cohort recovery: complete durable publication, protected recovery evidence,
  safe cleanup and repeated interrupted recovery without mixed cohorts;
- the shared **resource-effect authority/receipt** boundary: no effect before its
  admitted prerequisites, no foreign cleanup, and no stale UI state converting
  a committed or uncertain effect into new authority.

Prove Init => Inv, Inv /\ Next => Inv' and Inv => each named safety property.
Use arbitrary finite admitted resource/session sets and repeated operations/crashes;
a theorem over a hard-coded two-document/two-crash model remains bounded to that
model. Retain finite TLC instances for counterexample discovery. Explain the exact
storage/process assumptions and parameter restrictions.

Liveness is separate: scheduling, peer responsiveness, eventual I/O success and
cessation of crashes are explicit premises where required. Safety does not imply
“always recovers”, universal remote cleanup or real-time UI responsiveness.

## 8. VF18: implementation proofs over shipped Rust

Extend the existing same-source Verus practice. The required kernel set is tied to
VF02–VF16's production symbols, not a new independent executable algorithm in a
proof directory. At minimum it covers:

- buffer/base/write authority, normalized edit geometry and payload association,
  exact splice/untouched-byte policy and batch mapping composition;
- source/projection/selection and resource-binding admission, including protected
  versus writable spans and wrong-owner/stale-result rejection;
- filesystem/save/recovery/installation authority and outcome-classification
  decisions, cohort/watermark ordering and cleanup eligibility;
- UI/service action freshness, consumed/one-shot state, sequence/delta-base checks,
  lease/lifecycle transitions and queue/budget/deadline arithmetic;
- ID generation/exhaustion and decoder-to-kernel bounds/identity premises.

Some mechanisms are already proven under 0045; preserve them, verify their callers
establish the premises and add the unproved composition/consumer obligations.
Plain Cargo builds execute the same source with proof/spec code erased. A proof
around an uncalled function or an external_body wrapper assuming its entire safety
property is not implementation verification.

Use total functions or explicit checked preconditions and prove the documented
postconditions/invariant preservation over admitted inputs. Caller predicates and
observer facts cannot be silently axiomatized. Register every trusted body/axiom/
foreign-function assumption with its precise scope and consumers; audit changes.
No assume(false), omitted proof, altered spec solely to match a bug or automatic
fallback from a required proof to tests.

This does not prove CPython, serde, Ropey or syscalls. Their role and contract must
be narrow and explicit, with VF19 bridging evidence. If a required owned-code
obligation cannot be discharged, it is a release blocker requiring correction or
an explicit user-approved change to that claim.

## 9. VF19: correspondence, faults, concurrency and calibration

Keep the chain explicit:

```text
claim -> model/theorem -> production symbol/guard
      -> actual observed facts -> actual effect -> editor/user-visible outcome
```

For each boundary, run production sequences under controlled schedules/faults and
project admitted observations into the model domains. Check the exported allowed
transition relation/invariants, including documented stuttering. Replay bounded
model schedules/counterexamples through real code; shrink failures into meaningful
regressions. A tested abstraction mapping is not advertised as a universal refinement
theorem. Never use the implementation twice as its own oracle.

Byte/property/fuzz campaigns cover malformed/duplicate/missing/null/version/identity
inputs, Unicode/native bytes, bounds and truncation. State machines generate useful
boundary transitions, not mostly irrelevant random no-ops. Native fault injection
must exercise both before-effect and after-effect/before-ack failures; an injected
error that skips the syscall cannot test a committed-but-unconfirmed outcome.

Loom uses shared production coordination with instrumented synchronization, not a
copied worker algorithm. The Python/helper/native campaigns use the shipped bundles
and real wrappers. Real SSH/container/PTY/storage lanes remain required and isolated;
ordinary test mocks cannot substitute for their namespace/effect guarantees.

Minimum calibration families include:

| Mutant class | Check that must expose it |
| --- | --- |
| Remove buffer/base/readonly/owner guard or skip a source publication side effect | Mutation/input/source contract and real editor state, not only a kernel unit test. |
| Wrong batch order, payload association, anchor mapping or protected projection span | Proved geometry/composition plus byte/history/selection/source correspondence. |
| Wrong observer identity, cross-namespace fallback or malformed reply accepted as fresh | Admission/parser/observer/helper-to-editor campaign. |
| Helper publishes/returns success early, drops metadata/sync, misreads cancel or cleans a foreign stage | Model ordering plus actual Python/native effect/receipt tests. |
| Container context/incarnation ignored; local client exit misread as remote cleanup | Real namespace/lease/descendant test. |
| Completion/wakeup omitted, capacity bypassed, deadline reset or PID reaped before final signal | Loom/model/kernel and actual supervisor correspondence. |
| Premature durable ack, old checkpoint cleanup or mixed cohort restore | TLAPS/model/Verus policy and repeated real persistence/recovery campaign. |
| Duplicated UI apply, wrong delta base or stale focus accepted | UI protocol/real-engine driver and source/effect observations. |
| Secret recorded through a less obvious frame/helper/metadata path | End-to-end capture/export/replay policy tests. |
| Install activation before verification or wrong manager/current-version identity | Policy proof plus actual installation/public-outcome campaign. |

A mutant must apply to the named real seam and fail the expected property/contract.
A stale replacement pattern, syntax/type error, missing solver, crash, timeout or
unrelated invariant failure is a broken gate—not a successful kill. Positive proofs
must check the entire declared target set; an obligation-count floor supplements,
not replaces, target inventory. Zero/subset verification and stale cached output
cannot pass.

## 10. VF20: required release gates and qualified baseline

Reuse the established Gripsack/Strop evidence stack rather than adding a new proof
language/framework: TLA+/TLC, TLAPS, Verus, Loom, existing differential/property/
replay/native tests and exact helper/OS campaigns. Pin tools, solver, Rust/vstd,
Loom, model inputs and supported verification images/checksums together. Preserve
normal build/proof erasure and static/no-OpenSSL release properties.

Required lanes on the exact candidate:

- `docker compose run --build --rm test`: ordinary/differential/state/byte tests,
  fmt, locked all-target clippy and locked test behavior.
- `docker compose run --build --rm model`: calibrated finite TLC safety/progress/
  reachability instances from the complete boundary inventory.
- `docker compose run --build --rm verify`: full declared same-source Verus targets
  plus semantic mutant calibration and trusted-body/target inventory checks.
- `docker compose run --build --rm tlaps`: required generalized safety obligations,
  pinned proof backend and attributable negative controls.
- **`docker compose run --build --rm core-assurance`**: whole-core correspondence,
  actual helper/codec campaigns, instrumented Loom and state/fault/property checks.
  No narrow protocol suite may stand in for the editor/filesystem/helper boundaries.
- Existing required-mode SSH/container/PTY/WSL/storage and artifact/publication lanes
  on the supported platforms. Missing required prerequisites fail rather than skip.

Existing gates remain mandatory while 0056 lands. The **new expanded lanes and
whole-core qualification** are this release's exit gate, not a reason to mix the
verification programme back into architecture implementation or to start completion
while verification is incomplete.

PR and tag workflows require the same applicable full assurance on the exact final
tree/artifacts. Model-only tag checks or a successful proof for an older commit are
insufficient. Run from private work/target copies; generated states/TTrace files,
mutated source and solver output never clobber the active working tree.
Canonical runners use locked dependencies and fresh private proof outputs, including
`cargo verus verify --locked` with the complete registered crate/target selection.
Provers and test observers stay out of shipping artifacts; proof erasure and
instrumentation must not add avoidable input→render work or allocation.

The release evidence binds source/helper/model/lock/tool hashes, target/obligation
inventory, TLC/Loom bounds, theorem assumptions, calibration results, native fixture
platforms and raw attributable logs. The public claim is a **qualified assured core
at that revision**, with readable proved/modelled/tested/trusted distinctions—not
“the entire editor and every dependency are mathematically proved.”

## 11. Execution order and defect handling

1. Accept and freeze the completed architecture candidate; enumerate actual shipped
   boundaries/callers and existing evidence. Completion, debugger and GUI do not start.
2. Review the human contracts and independent safety meaning; build/update the claim,
   model, production-symbol, effect and trust mapping before proving branches.
3. Prove/check editing, identity, source/projection and UI ownership; then local/
   remote filesystem and exact helper/container interactions. Do not finish only
   easy arithmetic or wire-format targets and call the core qualified.
4. Discharge lifecycle/terminal/privacy/recovery/UI-server/install boundaries and
   generalized theorems with real-code/native campaigns throughout.
5. Calibrate every boundary family, repair source defects and update affected models/
   proofs/mappings; re-freeze after changes. Any necessary architecture correction
   is coordinated/backported, not deferred into a later feature release.
6. Run all required final-candidate gates and produce the complete VF ledger and
   baseline artifact. Then hand off to 0058 worker; completion remains gated afterward.

Verification work may be parallelized by independent boundary ownership, but one
integration owner controls the candidate and cross-boundary contracts. Same-file
proof/runtime mutations are coordinated. Do not create a new kernel/observer in a
sibling that merely resembles the one production executes.

## 12. Foundation for future features

The stable core is not frozen forever; its guarantees become a **change contract**.
Future changes identify affected claims, maintain their premises, add required
feature-specific targets/calibration/native evidence and pass the same gates before
release. An inherited baseline badge does not prove a new consumer.

- **0058 native worker:** consume this completed pre-worker baseline, migrate local/
  SSH/container filesystem and execution to shared Rust with one service protocol,
  and discharge WK16–WK20's claim/proof/runtime/deployment changes before completion.
  The baseline Python proofs are not automatically proofs of their native successors.
- **0059 completion:** consume the worker-requalified mutation/source/input/LSP core; add actual
  query/index/resolve/acceptance contracts and sustained-typing/cancellation evidence.
  Do not fix a generic unbounded LSP queue for the first time inside completion.
- **0060 debugger:** consume the assured worker/process/resource/UI core; bind the
  DAP reference model to the real client/adapter and stop/frame/variable semantics.
- **0061 GUI:** consume the assured engine/recovery/UI-server core; prove/check new
  client-state obligations and test actual native Windows input/render/IME/UIA.
- **0062 distribution:** extend assured TUI/worker installation/publication identities with the
  actual signed Windows installer/bootstrap/activation/native effects.

Preserve the earlier requirement for pre-feature reference semantics: the handoff
includes reviewed completion/DAP assumptions, calibrated DAP TLA+ reference scenarios
and the new claims each feature must discharge. Mark unimplemented-feature correspondence
pending that feature. No unused runtime stubs or claims that an unimplemented
completion provider/debug adapter has already been verified.

Not in this release: the 0058 native-worker cutover, completion/DAP/GUI, native Windows workspaces,
remote PTYs beyond completed 0055 scope, a reconnect daemon, new package channels or
a proof of third-party kernels/interpreters/libraries. These exclusions do not waive
any VF requirement or permit classifying the entire helper/effect boundary as trusted.
Explicit user approval is required to change a named mandatory claim or release gate.

## 13. Evidence anchors and prior programme preservation

This standalone release owns the expanded assurance programme formerly embedded
in 0056. It preserves human contracts, calibrated models, TLAPS UI/recovery safety,
same-source Verus and open composition obligation, Loom/runtime correspondence and
exact-candidate gates, and broadens the target to all listed core boundaries.

Existing evidence is reused, not relabelled as new work:

- [0045](0045-verus-pilot.md) and `strop-core/src/editmap.rs`: landed same-source
  geometry proofs; batch composition remains a required outstanding obligation.
- [0024](0024-transaction-gateway-verified.md),
  [0030](0030-correctness-hardening.md), current mutation/transact/source code and
  the existing conformance/differential harness: actual mutation and UI ownership.
- [0040](0040-remote-editing-and-saving.md), `strop-remote/src/exec/{spec,supervisor}`,
  `save/protocol.rs`, `protected.py`, save/filesystem helper bundles, `strop-fs` and
  `strop-containers`: concrete observer/authority/helper/effect/editor chains.
- `specs/*.tla`, configs and strict gate helpers: existing model/calibration coverage;
  update stale mappings and limits rather than copying a new toy protocol.
- The inspected sibling `../gripsack/verification/guarantees.md`,
  `scripts/check_models.sh`, `scripts/check_verus.sh` and same-source `gripsack-policy`
  kernels provide the guarantee/target/negative-control precedent. Its 0048
  TLAPS/Loom/refinement programme is a planned binding follow-on, not a claim all
  those obligations already landed there or here.
- [0056](0056-architecture-prerequisites.md), [0054](0054-unified-filesystem-workspace.md)
  and [0055](0055-embedded-terminal-tui-and-gui.md) define the baseline implementation
  to freeze; [0028](0028-roadmap-and-review.md) records sequencing and acceptance.

No project-wide models/proofs/tests were run against the other session's in-flight
implementation for this documentation change. The checks above are mandatory for
executing and completing this verification release, not assertions made by this plan.

## 14. Post-baseline worker decision: instructions for the dispatched owner

The user subsequently authorized [0058](0058-unified-native-worker.md), **after this
release and before completion**. AR01–AR16 and VF01–VF20 retain their numbers,
ownership and current-candidate acceptance. Worker implementation is not backdated
into them, and planned worker targets must not fail this baseline merely because
the worker does not exist yet.

1. **Continue the frozen real implementation.** Verify every shipped helper/observer/
   editor effect required above, including VF08/VF09's Python implementation. Do not
   omit a claim, weaken a theorem or substitute future Rust semantics for current code.
2. **Deliver reusable contracts and attributable evidence.** Preserve stable observable
   claim IDs, model/theorem/kernel targets, source/lock/tool/helper hashes, abstraction
   mappings, trusted assumptions, native fixtures and calibration results. Record
   proved versus tested connections, not just a badge the next owner can copy.
3. **Keep the baseline immutable after acceptance.** The worker owner starts a distinct
   candidate and migration manifest. This plan's Python-specific rows describe the
   pre-worker implementation; its successor updates the live assurance inventory
   without falsifying historical evidence or keeping obsolete code in production.
4. **Transfer promises, not unearned proof status.** WK16 maps every VF family to
   unchanged, transferred/requalified, strengthened/reproved, new or retired
   implementation-specific claims. Retirement names the successor; user-facing
   filesystem/lifecycle/privacy guarantees are not retired with a Python file.
5. **Worker changes require new evidence.** Local/SSH/container codecs and caller
   admission, Rust filesystem/supervision, child/control-stream isolation, target/
   namespace identity, verified artifact activation and lease-safe cache cleanup
   require the WK17–WK19 proof/model/Loom/native campaigns. Existing UI/recovery/edit
   proofs stay required and their changed bridges are requalified.
6. **Gate the exact worker release afterward.** 0058 runs the updated complete
   `test`/`model`/`verify`/`tlaps`/`core-assurance` plus native/deployment/artifact lanes.
   No old source hash, model-only tag check, silently skipped Python target or copied
   baseline result qualifies the new binary. Then 0059 completion may start.

If contracts need clarification while this agent is still implementing/proving,
coordinate the shared semantic decision and record the baseline revision; do not
race edits to active kernels/proofs or move the release boundary by implication.
The worker owner is responsible for all newly introduced obligations. A necessary
fix to an inherited defect is corrected at its source and requalified, not hidden
behind a worker-specific exception or deferred to debugger/GUI.

## Landed slices

### VF17 TLAPS lane bootstrap + SearchLifecycle proofs: landed (2026-09-16)

The `tlaps` lane exists: a Dockerfile `tlaps` stage pins TLAPS 1.5.0
(tlaplus/tlapm tag 202210041448, installer
`--checksum=sha256:ebb7a3f2…`, on digest-pinned debian:bookworm-slim;
the moving 1.6.0-pre asset was rejected per the tla2tools precedent),
wired as a separate compose `tlaps` service and CI step — deliberately
NOT chained into specs/gate.sh, since the model stage has no tlapm.
`specs/SearchLifecycleProofs.tla` (EXTENDS the exact module TLC checks)
proves Init⇒Inv, Inv∧[Next]⇒Inv′ across all 12 actions plus stuttering,
and Inv⇒(RowsCurrent ∧ RowsInScope ∧ StaleAcceptsNever ∧ CompletionHonest
∧ WarmBounded); the inductive core needed three strengthening
auxiliaries (JobsBounded, JobsUnique, RowsAreRecords), recorded in-file.
Constants are never instantiated — the proof covers arbitrary finite
GENS/REV_MAX/WARM_MAX and an arbitrary PATHS set; the binary scope
domain, warm-queue capacity and counter cap remain at the spec's shape
(a spec edit, not a proof gap). Negative control:
`specs/SearchLifecycle_MutantProofs.tla` fails proof on exactly the
mutation's own steps (3/225 obligations — unguarded ProviderPartial
publication, Accept's stale-accept, the counter bound); the gate asserts
rejection is unproved obligations, never tool failure. Evidence:
`docker compose run --build --rm tlaps` — "All 225 obligations proved"
+ mutant rejected, exit 0. This discharges 0063 §6.6's TLAPS half. The
remaining VF17 families (UI ownership, cohort recovery, resource-effect
authority) await their models from the VF11–VF16 slices.

### VF01 boundary inventory + freeze tooling: landed (2026-09-17)

`verification/` holds the machine-readable boundary/claim inventory: 18
boundary rows (the 14 required families plus collection-projection,
local-fs-authority, remote-save, identity-exhaustion) carrying 55
claims, each with live evidence pointers (named test fns, proof symbols,
model files, scripts) and sha256 pins over owning sources and evidence
content. `check.py` (stdlib) fails CI on any claim without evidence,
any dangling pointer, any missing required family, and any source drift
without evidence drift (all four failure modes demonstrated); `--stamp`
re-pins only after re-validating and refuses unexplained drift.
`freeze.sh` records the exact candidate (commit, lockfile, helper-source
digests, Dockerfile stage pins, tool downloads) into
`verification/candidate.json` and `--check` guards it. CI runs the
checker as its first step. Evidence: `inventory check ok: 18
boundaries, 55 claims, all evidence live, all pins current`.

### VF18 Verus kernel expansion: landed (2026-09-17)

Five verified kernel families joined editmap/searchguard, all same-
source with production call sites rewired (0045's no-copied-algorithm
rule): `mutguard` (mutation/save authority — classify_edit with the
readonly-before-stale precedence proved, save admission and save-ack
recency), `projectguard` (projection span ownership — a span owns a
journal edit iff the edit starts inside the writable view body and ends
within the rendered span), `cohortguard` (recovery — CohortComplete:
a draft joins whole iff it fits the remaining budget, never truncated;
SaveRetirement staleness halves; WatermarkOrdering), `viewguard` (UI
freshness — a prepared pane is stale iff its keyed revision moved), and
the in-place `id.rs` block (slot reuse advances the generation, wrap
retires forever, full index space refuses instead of truncating).
Evidence: `docker compose run --build --rm verify` — 50 verified,
0 errors in strop-core.

### VF02 batch composition + VF03 correspondence + VF04 ChangePlan: landed (2026-09-17)

The 0045 stretch obligation closed: `batch_map_composition` proves
folding per-edit mappings in reverse application order equals the direct
whole-batch map over original coordinates (production's apply_prepared
fold), with monotonicity and in-bounds preserved; the new spec fns are
#[verifier::opaque] with explicit reveals so definitional axioms never
inflate unrelated queries (rlimit bisected, documented). VF03's
correspondence landed as six trace-replay tests replaying
EditorProtocol.tla action sequences through the ACTUAL admission
handlers (stale-revision delivery drops without publication, dead-
document and reincarnation deliveries are NoDocument, tickets are one-
shot, readonly refusal precedes freshness). Resolver fidelity (the
nvim differential) and readonly-presentation checks were confirmed
registered in the required lanes. VF04: ChangePlan.tla gained the
projection-admission properties with four kept mutants killed by exactly
the named invariants. Evidence: `docker compose run --build --rm verify`
and `--rm model` both green.

### VF12 Terminal + VF16 Install models (VF11–VF16 partial): landed (2026-09-17)

`specs/Terminal.tla` models session lifecycle, pinned-snapshot and
refresh/exit boundaries over the 0055+0065 semantics — gate green with
six kept mutants rejected and nine witnesses reached.
`specs/Install.tla` models staged/verified/atomic publish, interruption
preserving the old binary, receipt identity and catalog facts over the
AR11/AR12 semantics — five mutants, nine witnesses. Both are chained
into specs/gate.sh. Recovery.tla (VF13) and LspWire.tla (VF05) exist
but are NOT gate-chained yet: Recovery's main instance does not settle
in bounded time (state-space calibration) and LspWire's first
calibration run never completed; their re-entry condition is recorded
in specs/gate.sh. Loom campaigns (VF11) landed for the LSP queue
(fifo/barrier/drain-disconnect); the event-channel campaigns and the
remaining VF13/VF05/VF14/VF15 work continue in the next release.

### VF13 Recovery + VF05 LSP wire models calibrated and chained (2026-09-24)

Both models now settle bounded and are chained into specs/gate.sh
(replacing the unchain note). Recovery.tla: REV_MAX 2→1 — the seven
revision-valued clocks each shrank 3²→2² (~300× on the cross product);
5.86M distinct states in 1m53s. REV_MAX=1 is the honest minimum: every
invariant keys on revision EQUALITY, never magnitude. Calibration
repaired two latent defects: the half-cohort-merge mutant was documented
but never implemented (now dies by exactly CohortCoherent +
DurableMatchesLastComplete), and WitnessNoMemoryOnly was violated by
the initial state (reformulated honestly). LspWire.tla: DOCS→{d1} +
VER_MAX 5→3 (the original config could never hold TypeOK — admSeq would
have overflowed); 5.09M distinct in 72s. Four latent spec bugs repaired
(unassigned alive, admLog index vs wire seq divergence in
LastAdmBefore, coalesce keeping the superseded seq, mutant 3's second
half unimplemented, unreachable reopen witness). Evidence: full
gate.sh green end to end via the docker harness (958s, 12 domains).

### VF14 UiSession model + parity journeys: landed (2026-09-24)

specs/UiSession.tla models the AR09/AR10 semantics: server-side
admission (incarnation → future ceiling → client_known floor), ordered
bounded publication, client drop/poison/snapshot-recovery, staged→
emitted effect authority, final-ack→bye shutdown. Thirteen invariants
(StaleNeverActs, FutureNeverActs, ForeignNeverActs, PoisonedNeverActs,
PoisonedUntilSnapshot, NoEmptyPublication, EffectExactlyOnce,
ByeAfterFinalAck, NoPostByePublication, CliNeverAhead,
PublishedNeverAhead, ByeNeverStrandsEffects, TypeOK), nine kept mutants
killed by exactly their named invariant, ten witnesses; main instance
197k states in ~1s; chained into gate.sh. The closure harness gained
four VF14 journeys (drop→poison→resync recovery, clipboard effects
exactly once, wrong-version hello typed+terminal, shutdown drains
in-flight work); ui_stdio.rs moved to a directory target under the
file ceiling. Surfaced for VF11 follow-up (not a VF14 blocker): picker
teardown during finish() can exceed its budget under an extreme
parallel storm — serial repros settle in <0.2s, no deadlock evidence.

### VF06–VF10 domain assurance campaigns: landed (2026-09-24)

VF06: strop-fs adversarial campaigns (10 in the test lane): symlinked
ancestors resolve but the final entry is never followed; source
rewrite/parent swap/symlink retarget between prepare and apply are
typed Conflicts with zero effect; permission transitions fail closed;
lock domains reject hardlinked/permissive/symlinked stand-ins; reserved
namespaces refuse case- and NFKC-folded aliases; cross-mount rename is
typed Unsupported. Honest limits documented (root lanes can't observe
EACCES). VF07: the SSH/SFTP model fleet re-verified against the current
code — no semantic drift; verification/check_model_anchors.py now pins
the five model digests + 42 production anchors (a silent model edit or
anchor removal fails typed). VF08: the Python helper bundle is
digest-pinned (assembled bytes reproducible from the tree, recorded in
verification/) plus supervisor/exec seam campaigns (spec decode,
tampered control blob). VF09: five semantic-boundary fault-injection
tests on the remote save path (pre/post commit). VF10: container
incarnation/StartedAt assurance extended per plan. All registered in
the VF01 inventory (21 boundaries, 73 claims, all evidence live).
