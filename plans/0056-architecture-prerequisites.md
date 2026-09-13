# 0056 — Architecture prerequisites before whole-core verification

Status: dispatched standalone architecture/reliability release handoff. This document
assigns implementation ownership; **the fixes are not implemented by writing it**.
The user explicitly requested architecture closure first, a separate whole-core
verification release immediately afterward, and only then more editor features.

Release sequence:

```text
0054 filesystem -> 0055 terminal -> 0056 architecture prerequisites
    -> 0057 whole-core verification
    -> 0058 native worker + assurance migration
    -> 0059 completion -> 0060 debugger -> 0061 GUI
```

[0062](0062-distribution-and-wsl-onboarding.md) supports GUI distribution rather
than adding another editor feature arc. Its existing TUI installation/release
correctness gaps are closed here; Windows installer/onboarding/channel work remains
there. Completion waits for this release, [0057](0057-core-verification-and-assurance.md)
and the subsequent [0058 worker](0058-unified-native-worker.md); C01–C09 is unchanged.

## 1. The boundary: infrastructure here, features in their own releases

**Ship a usable, tested architecture release without the native-worker cutover, completion, GPUI or DAP feature
code.** Its consumers are the real TUI, existing headless driver, services, recovery
UI, installer/updater and a real non-graphical UI-protocol client/server. Working
behavior is required; interfaces with no working consumer do not count.

0057 then verifies the stable core end to end: mutation/input, source projections,
local/remote filesystem authority, Rust/Python helper effects, containers, lifecycle,
recovery and UI/release boundaries. It owns the expanded proof programme, not this
architecture release. Existing tests, models and proofs remain mandatory here.

The subsequently authorized 0058 owns shared native worker/deployment and its
assurance migration **after 0057**; it does not interrupt the dispatched AR/VF work.
0059 owns completion next. 0060 owns adapters, launch discovery, DAP sequencing,
stopped-state semantics, breakpoints, evaluation and debugger views. 0061 owns native Windows rendering,
input/IME/accessibility, native-window automation/capture and full feature parity.
They may add their feature-specific data/actions through the completed contracts;
they do not invent another renderer boundary, recovery store, process supervisor
or protocol server.

If a preceding TUI release already closes an item, record its concrete implementation
and passing evidence and **reuse it**. Do not rebuild a second raw-input path,
terminal lifecycle, Directory model or transaction system to satisfy this plan.
Any remaining shared defect found during verification or later feature work requires
an explicit architecture-contract correction and affected evidence update, rather
than being buried in a renderer, provider or adapter callback.

## 2. Gap inventory and dispositions

This consolidates the concrete sweep and older named debt. It is not an invitation
to audit/refactor the whole repository without a demonstrated boundary or consumer.
The two user-reported gaps—mutating render admission and no dirty-buffer backup—are
taken as current facts, not re-tested merely to confirm the report.

| Gap / evidence | Disposition |
| --- | --- |
| Rendering accepts mutable Editor, admits hunk/analysis/preview work and feeds viewport state; 0046's dependency extraction is not full presentation isolation. | AR01–AR03 implement the actual pure-render/action/view cutover now. |
| Broad public document/service/ownership fields let frontends bypass admission; a crate move is not encapsulation. | AR02 closes the supported public boundary and migrates all real consumers. |
| Session persistence reopens disk files and conditionally restores history; captured ropes are not a persisted dirty/scratch/remote backup. | AR04 implements a separate real recovery contract, not a rename of session metadata. |
| Source relocation, stale consumers and mutation outcomes must survive view changes and failures; 0054 already owns filesystem behavior. | AR05 consumes its binding/receipt model and closes cross-consumer lifetime gaps, without another filesystem implementation. |
| `editor/events/channel.rs` wraps `std::sync::mpsc::channel`; EVENTS_PER_TURN/TURN_BUDGET provide fairness, not a retained-byte bound. Long-lived services cannot simply count as pending forever. | AR06 supplies bounded admission/transport and explicit liveness/shutdown behavior. Do not drop semantic events or block input to simulate a bound. |
| Container `exec_command` is `docker exec -i` without an in-container lifetime supervisor; its EOF comment is not cleanup proof. It accepts a bare ID, and `exec_capture` ignores its EngineRef. | AR07 fixes selected execution context, incarnation and actual leased cleanup before a debugger relies on them. |
| Executable discovery can find an unusable toolchain shim; tool/runtime selection must stay in the selected namespace. | AR07 provides real bounded readiness/admission diagnostics, not universal pre-probing or implicit install. |
| Native input/effects/capture need one target-aware policy; terminal input/frame data can expose secrets before later handlers redact them. | AR02/AR08 reuse and generalize the completed 0055 contracts for current consumers. |
| GUI currently owns a not-yet-built generic UI protocol/server, snapshot/revision ordering and recovery integration. | AR09–AR10 deliver the non-graphical server/protocol/driver here; GUI becomes a consumer. |
| Arena insertion still increments a u32 generation unchecked and casts slots.len to u32 in `strop-core/src/id.rs`. | AR13 fixes checked exhaustion and all callers, with seeded boundary evidence. |
| `:explain` has real LSP/indent data, but general config provenance and active-owner/receipt coverage are incomplete; its generic readonly hint suggests remote edit even for other readonly sources. | AR14 closes decision/provenance presentation from actual typed owners, not message parsing. Preserve working indentation provenance. |
| Normal Docker builder still starts from floating `rust:alpine`; other runtime images and shared-boundary module sizes need deliberate ownership. | AR14–AR15 close named module/build reproducibility debt, not a line-count campaign. |
| The old 0049 container “required mode silently skips” finding is already fixed in current `tests/docker.rs::gate`: explicit opt-in asserts engine availability. SSH required mode is wired too. | Preserve and exercise these gates under AR15; do not reimplement them or claim they are still missing. |
| `install.sh` writes via install/cp fallbacks while the updater has a staged native transaction; install channel identity is inferred from path substrings. | AR11 closes current TUI installation/ownership gaps; worker deployment extends this under 0058, Windows packaging under 0062. |
| Website hero and dynamic release chip were observed at different versions; site promotion is best effort despite stronger user-facing release requirements. | AR12 establishes the shared release catalog and observable promotion evidence now, not inside GUI implementation. |
| GPUI native capture/IME/UIA, Windows signing identity, real GPU hardware lane and Windows setup are not existing core bugs. | Keep their exact gates in 0061/0062. Do not build a GUI, signing service or installer here. |
| Existing TLC/Verus coverage is narrower than whole-core assurance; observer/helper/executor correspondence, generalized proofs, calibration and exact-candidate tag gating need their own release. 0045's batch-mapping composition remains unproved. | Preserve existing gates here; [0057](0057-core-verification-and-assurance.md) VF01–VF20 owns the expanded verification immediately afterward, before completion or any new consumer. |

Evidence anchors are in §13. Historical findings are not a substitute for checking
what the preceding implementation actually delivered; record **closed/reused** versus
**needs repair** before assigning ownership, without diluting the required contract.

## 3. Required architecture-release ledger

All AR requirements are release requirements. Each gets an owner, implementation
paths, current status, failure/privacy behavior, exercised checks and downstream
consumer contract. “Interface exists” or “compiles” is not completion.

| ID | Required deliverable |
| --- | --- |
| AR01 | Readonly rendering with explicit idempotent view/viewport/model-driven preparation; drawing never admits native work or changes editor semantics. |
| AR02 | Narrow admitted action/input/effect API with complete rich-input ownership and migrated TUI/headless/service callers; no public mutable frontend bypass. |
| AR03 | Stable source-aware semantic presentation/read models, explicit coordinate/focus/revision contracts and bounded owned view updates. |
| AR04 | Private, bounded, real dirty/scratch draft checkpoint/recovery with cohort/source baselines, policy, conflict handling and durable watermarks. |
| AR05 | Unified resource-binding/operation outcome integration across existing views/services/history/recovery; confirmed mutations are not lost on UI staleness. |
| AR06 | Bounded physical work/event/data retention, distinct finite-work versus service liveness, and nonblocking ownership-aware shutdown. |
| AR07 | Correct selected-context native/SSH/container execution, capability/health diagnostics, incarnation validation and real native evidence of leased process cleanup. |
| AR08 | Target-aware host effects, trust and privacy classification across input, records, projections and replay, with explicit native-free semantics. |
| AR09 | Implemented bounded UI protocol/client state and real `strop --ui-stdio` backend, not a GUI-owned future scaffold. |
| AR10 | Rust-first semantic/transport driver and deterministic schedules, real stdio/WSL closure tests, state/revision barriers and parity with current consumers. |
| AR11 | Shared verified TUI install/update transaction and installation identity/receipts; no unchecked direct-copy fallback or guessed manager ownership. |
| AR12 | Generated current-product release catalog, consistent installer/site version facts and resumable verified public promotion. |
| AR13 | Checked arena/session/request identity exhaustion, safe failure/retirement behavior and complete caller migration. |
| AR14 | Completed decision/explain/help ownership and responsibility-based splits at the affected engine/presentation/service/persistence boundaries. |
| AR15 | Deliberately pinned build inputs and enforced existing required-mode native/container/SSH evidence; no vacuous skip promoted to success. |
| AR16 | Full integration smoke/regressions/replay/performance/static/package and existing model/proof gates, source/docs cleanup and an exact stable-candidate handoff to 0057 verification. |

No AR item may be delegated to “the GUI phase” to finish this release. A genuine
scope change requires explicit user approval and a [0028](0028-roadmap-and-review.md)
entry with impact, evidence and re-entry condition. Signing/hardware prerequisites
for future native GUI publication remain explicitly outside this release.

## 4. AR01–AR03: action, preparation and presentation

### Pure paint is a behavioral invariant

The supported flow is:

```text
input / source change / service result / viewport change
          -> admitted engine update
          -> prepared view and owned work requests
          -> readonly presentation query
          -> TUI paint or serialized semantic view
```

Move refresh_hunks, visible-analysis/preview admission and viewport/caret adjustment
out of renderer callbacks. Do not merely rename render to prepare_frame and keep
admitting work on every paint. Preparation depends on meaningful state/geometry
changes and is idempotent for unchanged inputs.

A renderer consumes `&Editor`-equivalent readonly view data, but the signature alone
is not proof: no hidden lazy I/O or interior mutation behind “readonly” accessors.
Repeated paint without an engine/view change must not allocate worker tickets,
launch a process, mutate selections/history, consume a proposal or change source
state. Paint can update its **own frontend caches**, not engine authority.

Geometry becomes an explicit input owned by a view/epoch. TUI rows/cells, source
bytes/lines and future GUI pixels remain separate domains. A viewport update can
legitimately schedule bounded preparation; arbitrary compositor repaint cannot.
Preserve existing relative scroll/jump/collection behavior and native-free replay.

### A narrow real API, not a second engine

Keep the existing Editor implementation and shared grammar/transaction gateway.
Expose admitted inputs/actions, service result delivery, viewport interest and
borrowed prepared-view queries. Make fields/private helpers that let a frontend
mutate documents, service maps or leases inaccessible to that frontend.

The keymap/Ex/action registry remains canonical. No duplicate command table, direct
GUI-style set_buffer API or `_pub` wrapper that still hands out unrestricted mutable
state. New feature handlers in 0059/0060 remain engine internals and use the same gateway.

Reuse 0055's rich frontend-neutral key/text/paste ownership. The target is chosen
in the ordered engine stream before editor-specific normalization or privacy
recording. Preserve 0048's ordinary editor Esc/Alt/Ctrl-C behavior; terminal-owned
keys retain their child meanings. A logical pointer/selection action carries the
view/document/base revision and checked coordinates; no pixel or toolkit types
enter grammar. Native Windows gesture/IME adaptation remains 0061 work.

### Stable semantic views

Prepare source windows, syntax/decorations, fields, rows/trees, selections, cursor
and capabilities as actual semantic data with stable identities. Reordering a
catalog does not change row identity; a display label or row number never becomes
a source/resource handle. Collections refer to authoritative source documents, not
another editable copy; Directory drafts retain their own protected provenance.

Borrow existing data for the TUI. Own/serialize only bounded changed or requested
windows for process clients, keyed by backend incarnation, source/binding revision
and view generation. No entire-Editor/full-repository clone per key/frame. Declared
bounds distinguish complete/partial/loading/stale/error; no empty-success fallback.

Test source-byte↔UTF-16↔logical-cell conversions, tabs, CRLF, combining/wide text and
clipping at their actual boundaries. The native font/pixel transform and AccessKit
text provider remain GUI work, consuming these checked source contracts.

## 5. AR04: real draft recovery, not session relabeling

Normal session restore and draft recovery are different products. Existing session
metadata can still reopen a file and restore compatible view/history state. It must
not claim to recover unsaved bytes that were never persisted.

### Captured data and authority

A recovery record identifies its workspace/namespace, source or scratch draft,
resource binding epoch, captured document revision, last saved/source observation,
text snapshot and related change-group/cohort identity. It contains the bytes needed
to recover the draft, not only a hash or undo pointer whose base is no longer present.

Never serialize native handles, remote write permits, DAP references, active process
leases, pending execution authority or commands to auto-run. Generated collections,
search rows and Directory presentations are not substitute source backups. Recover
eligible underlying source drafts and explicit unresolved edit drafts; retain view
metadata separately where useful.

Default recovery covers ordinary local dirty documents and scratch drafts in private
state storage. Remote/sensitive content requires explicit user policy/consent; a
project config cannot grant itself permission to persist it. Keep memory-only mode
honest and visible. Terminal input/output and future debugger values remain outside
general document recovery unless their own explicit capture policy permits them.

### Persistence and scheduling

Use a separate owned recovery module and private state directory; stage with private
permissions before writing, validate native paths/records and atomically publish
complete checkpoints. Coalesce immutable snapshots on a bounded worker queue, not
mutating text commands. No whole-rope String materialization or fsync in input/render.

A multi-document change captures a coherent cohort. Publication/recovery must not
silently combine half of a newer group with half of an older checkpoint and call it
one atomic edit. If the cohort exceeds a bound, report its recovery state rather
than silently truncating it. Retain the last complete checkpoint until a newer one
is durable; cleanup removes only proven owned obsolete records.

Keep **applied revision**, **captured checkpoint** and **durable checkpoint** separate.
Recovery guarantees the last completed checkpoint, not unflushed bytes after power
loss/SIGKILL. Policy limits and persistence failure must reach the existing status/
explain/notification surfaces. A checkbox or “recovery enabled” flag is not proof
that the current draft is durable.

### Restore, save, rename and close

Offer recovery through a real TUI-accessible command/surface before any GUI exists.
Show original location, checkpoint time/revision, source comparison and conflict/
missing/renamed state. Restore into a checked draft; never overwrite changed disk
content, recreate a deleted target or silently re-grant remote write authority.
Use existing review/conflict/source-admission paths for reconnecting or saving it.

Checkpointing does not mark text clean or advance its saved baseline. Confirmed
save can retire only the checkpoint it actually supersedes; edits made after that
save snapshot still need recovery. Rename/move updates the binding through AR05,
not a path-string substitution. Explicit user discard differs from accidental
transport/window loss and has a deliberate recovery-retention decision.

Orderly close/stdio EOF quiesces new input, reconciles admitted effects, checkpoints
eligible drafts and closes owned services under AR06. Recovery failure is not a
successful save or a reason to auto-write source files. Restore itself launches no
adapter, terminal, project hook or remote connection without fresh authority.

## 6. AR05–AR07: identity, outcomes and execution lifetimes

### One binding/outcome owner

Consume 0054's DocumentId-stable resource relocation and checked file-operation
receipts. Integrate source binding changes with LSP/analysis/preview/search/collection/
history/recovery state and every existing relevant caller. Retire stale work by
binding incarnation, not just matching text revision or display pathname.

A stale read can be discarded. An admitted mutation outcome cannot disappear because
its popup closed or focus changed. Reconciliation is idempotent by operation identity;
UI navigation/toasts have a separate focus policy. Preserve pending/unconfirmed
outcomes and block conflicting authority until resolved. No automatic retry or
rollback that can overwrite/delete a resource now owned by another actor.

Use typed namespace/resource/native-path contracts from 0042. Mounted-Windows storage
classification and conservative capability policy are backend facts, not GUI button
state or a literal `/mnt/c` prefix. Expose an honest read-only/refused profile for
unsupported mutation semantics; don't weaken protected operations or claim a sandbox.
Keep existing namespace behavior compatible unless its change is explicitly part
of the accepted policy and verified with the current TUI.

### Fairness is not backpressure

The current turn/event-count budget limits time between events; unbounded mpsc
retention can still grow without limit. Inventory each producer/consumer boundary
and impose real byte/count/work bounds before publishing/admitting new work.

- Do not make UI sends block as the default fix.
- Input, VT byte streams and durable outcomes cannot be dropped/coalesced as if
  they were repaint hints. Refusal before admission is visible; accepted work
  retains a terminal outcome.
- Prepared view snapshots/wake notifications may coalesce when their revision/base
  rules permit it. A dropped delta requires resynchronization, not blind application.
- Backpressure native readers off the UI thread, bound outstanding requests and
  retirement, and preserve input/cancellation fairness under sustained output.

AR06 also closes the existing LSP wire queue's full-rope retention before completion
starts. Bound unsent synchronization snapshots, requests and retirement; serialize
and destroy large payloads off input. Coalesce superseded unsent document updates
only when no admitted consumer needs that intermediate version. Preserve didOpen/
didClose, binding incarnations and request/version barriers, and never cancel an
already emitting frame halfway through. Exercise this with current LSP consumers;
0057 qualifies this transport; 0058 preserves/requalifies it before 0059 completion.

Separate finite work (start, handshake, mutation, checkpoint, stopping) from a
running long-lived service. Existing headless settle waits for the requested owned
work/state, not for every language server/terminal to exit. Shutdown explicitly
quiesces, drains/reconciles and closes; no infinite async_pending condition and no
blocking child Drop on the editor thread.

Reuse terminal lifecycle delivered by 0055 and existing LSP ownership, extracting
only a genuinely shared narrow primitive. Do not implement a universal process
or actor framework. Owned launch versus borrowed target identity must remain
expressible; future debugger detach is not permission to kill an attached process.
The actual DAP terminate/disconnect policy belongs to 0060.

### Captured execution context and real cleanup

Execution requests freeze namespace/principal, selected engine/connection, cwd,
program/argv/environment and relevant incarnation/authority. Honor the selected
EngineRef; don't discard it and invoke a changed global Docker context. Revalidate
container canonical ID + StartedAt before execution admission. A container name or
host docker-top PID is not a namespace-local process identity.

Complete real container supervised stdio: a fixed helper/lease owns the in-container
program and reports launch/exit/termination outcomes. Local docker-client EOF/exit
alone is not that guarantee. Reuse tested SSH supervisor mechanics where appropriate,
without requiring an SSH server in the container or conflating DAP with SSH.

Keep argv/native paths inert. No shell-interpolated target/label, no global cwd/env
mutation, no automatic elevation, container restart/provisioning or package install.
Missing helper/permission/adapter capability is a named refusal. Small/distroless
containers cannot be assumed to contain a shell, Python or which; probe only where
supported, otherwise classify the real launch failure truthfully.

Readiness is established by the appropriate bounded version/handshake, not merely
which finding a toolchain shim. An explicit invalid override does not silently
fall back to another program/namespace. Preserve SSH's documented limits around
network partitions, dead-peer detection and deliberately detached descendants;
never claim universal remote kill guarantees.

## 7. AR08: host effects, privacy, explainable authority and replay

Use one target-aware effect/admission classification for clipboard, external open,
process launch, source mutation, persistence and observation. Explicit user actions
and project/remote authority remain distinguishable. A future frontend consumes
requests/results; it cannot grant itself write/exec/trust via a serialized field.

Choose input ownership/privacy **before** generic trace recording. Apply capture
policy to keys/paste, command/env, requests/responses, draft/projection content and
cell/view exports. Redacting one event while a frame snapshot records the same
secret is not privacy. Preserve 0055's terminal defaults and current remote policies.
Future debugger-specific values/expressions are classified in 0060 through this
same mechanism, not another logger.

Private opt-in full capture records enough ordering/state to replay without native
execution. Metadata-only capture names unavailable content/replay fidelity; it
doesn't fabricate empty equivalent views. Replay and recovery cannot acquire fresh
process, filesystem, clipboard or network authority. No implicit side effects
from a title, restored URI or a field displayed as a command.

## 8. AR09–AR10: deliver the non-graphical UI backend now

This release owns **strop-ui-protocol and a real strop --ui-stdio mode**, using the
existing engine. The future Windows GUI owns its native process/window integration,
not the protocol's initial invention or the engine-side composition root.

### Minimal concrete contract

Use the existing bounded Content-Length-style codec at the byte boundary where it
is genuinely shared with LSP; keep protocol envelopes distinct. Pure protocol data/
client state can live in strop-ui-protocol, without GUI/OS/executable callbacks.
Reserve stdout for framed messages and stderr for bounded diagnostics. No listener
port, daemon, login-shell banners or skip-garbage-until-JSON fallback.

Implement:

- version/release/limits/capability handshake with backend incarnation;
- admitted input/actions carrying client/action sequence and source/view/base identity;
- acknowledgement/error with last-applied sequence and explicit outcomes;
- bounded semantic snapshots/deltas/text windows with base/new revisions and stable IDs;
- viewport interest, host effect/result, resynchronization and authorized shutdown;
- failure/disconnect checkpoint/drain policy from AR04–AR08.

Expose all **currently implemented** TUI families through actual common actions and
semantic observations, including completed terminal/filesystem features. No no-op
actions, empty dataset placeholders or text-editor-only smoke advertised as the
complete backend. 0060 later adds actual debugger views/actions through this contract;
ordinary versioned feature extension is not permission to redesign the engine.

The client is a readonly cache/consumer, not another editing model or undo stack.
Control/input and side-effect requests remain ordered; only safe presentation data
coalesces. An unknown/stale delta base requests a complete current snapshot. After
link loss, stop accepting actions, retain known outcomes and recover explicitly;
no optimistic replay of launch/save/rename/evaluate or uncertain text input.

### Working consumer and proof before GUI

Provide a Rust-first protocol driver using production input/actions/observation
paths. It can open/edit/undo/search/collect/review/save, inspect current terminal/
filesystem state, wait for named revisions/outcomes and close the real backend.
It must not expose a set-buffer/set-trust fake-success backdoor as its e2e path.

Tests drive the actual stdio mode plus controlled native service fixtures. Compare
semantic results with TUI/headless consumers, not GUI pixels. Readiness uses owned
state/action/revision barriers; a live service is not a reason to wait forever and
arbitrary sleeps are not evidence.

Include a real Windows→wsl.exe→backend stdio smoke driven from WSL or the native
CI lane, with explicit distro/user/absolute backend path. Verify Unicode/quoting,
startup directory, partial IO, EOF and lifecycle. This needs no window, GPUI, IME
or installer. The future GUI-specific Win32 adapter uses the proven protocol/client
state; native window capture, UIA and graphical input remain in 0061.

All protocol/driver endpoints are explicit opt-in, private and bounded. No new
public network service or automatic backend installation is part of this release.

## 9. AR11–AR12: close current installation/release truth gaps

These are repairs to existing distribution foundations, not the Windows packaging
feature from 0062.

### Verified install transaction and identity

Make the TUI bootstrap installer and native updater consume one documented verified
artifact/installation transaction. Stage in an owned same-destination context,
verify expected bytes/format/permissions, then publish atomically; failure preserves
the old installation. Remove direct overwrite/cp fallback as a purported equivalent.
The shell bootstrap can remain thin, delegating to a verified native installation
path when appropriate; no independent unsafe transaction hidden in a script.

Record installation identity/ownership and exact artifact/version in a private
receipt. Cargo/Homebrew/mise and unmanaged installs receive truthful update routing;
path substring matching alone cannot authorize overwriting a manager-owned file.
Handle pre-receipt installs explicitly without inventing provenance. Keep known
previous/current identity for recovery/rollback mechanisms rather than pretending
an arbitrary older download is a verified rollback.

No GUI installer, Windows host path or WSL backend provisioning is implemented here.
0062 extends these contracts with signed Windows installation and selected-distro
backend caches. Existing TUI installs must not be overwritten to prepare that future.

### One current-product release catalog

Generate current TUI artifact/version/digest/support facts from build outputs. The
installer/updater/site consume the same immutable release choice; don't fetch latest
independently for different components or maintain a stale hero beside a dynamic
version chip. Consolidate or verify duplicated installer/site copies.

Keep the existing tag/version/lock, static artifact, checksum, attestation and Cargo
publication gates. The publication-order script already handles private packages;
reuse it. Add a resumable promotion ledger and verify the actual public rendered
site/download/install facts. A site hiccup cannot unpublish an immutable crate, but
it also cannot be hidden behind a misleading fully-promoted status.

Checksum, publisher signature, TLS and build provenance have distinct guarantees;
don't rename one into another. 0062 adds the Windows signing identity/installer and
winget channel; no signing certificate or native GUI artifact is required here.

## 10. AR13–AR15: finish the named quality debt

### Checked identity exhaustion

Fix generational-arena and relevant session/request counters before additional
long-lived consumers depend on them. Insertion is checked and returns a typed
failure; exhausted generations cannot wrap and make a stale ID resolve. Retire
unreusable slots, continue where a valid allocation is possible and refuse capacity
exhaustion without truncation or release-mode aliasing.

Migrate all callers, including multi-document opens/restores: allocation failure
must preserve existing documents/edits and not leave a half-published operation.
No unwrap/panic/fabricated ID or deprecated infallible alias. Seed near-boundary
states in isolated tests rather than allocating billions of objects. Keep private
iteration indexes boring where no domain identity crosses a boundary.

### Decision provenance and component splits

Complete :explain from actual decision records: active input/action owner, selected
service program/root/namespace and why, effective settings with provenance, source/
save/recovery capability and latest relevant proposal/outcome. The readonly reason
must belong to the current source; don't prescribe :remote edit for every readonly
buffer. Preserve existing indentation provenance and registry-based config access.

Use the canonical action/Ex/help registry and semantic rows, so current TUI and
later clients get the same discoverability. No log-string inference or second
settings/command catalog. New feature-specific diagnostics later extend it.

Split affected overgrown files by actual responsibility: lifecycle/admission,
projection/geometry, identity/history, recovery/persistence, protocol and render
consumers. Keep the approximate 800-line ceiling and complexity rule, with the
established data-table exception. No unrelated line-count campaign or one crate
per feature. Replace implementation/command-string tests in touched boundaries
with behavior/lifetime evidence instead of re-pinning the same weak assertions.

### Reproducible, required evidence

Pin normal compiler/base-image inputs deliberately, including the correct multi-
architecture digest/target handling. Keep the separate verified toolchain lane;
don't accidentally pin arm64 to an amd64-only image or disable static verification.
Version tags plus lockfiles alone don't pin a floating rust:alpine environment.

Required-mode SSH/container fixtures already exist in current code. Preserve their
fail-not-skip behavior and wire changed ownership paths into the real lane. Optional
local omission is not successful release evidence. No mocks or inaccessible Docker
turned into a green “required” check; no unlabelled container/user-state cleanup.

## 11. AR16: implementation sequence and integrated evidence

One integration owner owns the API/identity/lifecycle contracts and the final AR
ledger. Independent recovery, execution and release-foundation slices can run in
parallel after their shared contracts are explicit; siblings do not invent
competing capability, receipt, input or presentation schemas.

Dependency order:

1. Reconcile current preceding-release implementations against §2; assign one owner
   and observable acceptance for each AR ID. Preserve already-correct behavior.
   Record the actual semantic contracts, effects, failure semantics and evidence;
   preserve/update existing models and proofs for changed behavior.
2. Close render/action/view and identity/focus/coordinate boundaries; migrate the
   real TUI/headless consumers and update trace scheduling deliberately.
3. Finish bounded liveness/outcome/execution and privacy contracts using real current
   LSP/terminal/container/native consumers, not future debugger stubs.
4. Implement draft recovery/cohorts and TUI recovery UX, including failure, discard,
   save/rename and sensitive-source policy.
5. Deliver the UI protocol/server/driver and actual stdio/WSL parity/closure evidence.
6. Integrate TUI installation/catalog/provenance repairs, checked IDs, explain/module
   cleanup and pinned/required evidence gates; hand the stable candidate to 0057.

Required behavioral evidence includes:

- Repaint/resize/model-change schedules: unchanged repeated paint has no native
  admission or semantic mutation; genuine viewport changes prepare once correctly.
- Every current input owner retains grammar/terminal behavior and private capture
  policy; stale focus/revision inputs cannot act on another document.
- Crash/EOF/restart with local dirty and scratch drafts, cohort edits, changed/deleted/
  renamed sources, failed persistence, memory limits and remote consent; source files
  remain unchanged until an explicit checked save.
- Mutations/receipts delivered after their view closes, pending/unconfirmed saves,
  relocation with old service replies, and allocation failure preserve correct state.
- Sustained output/request churn/large paste remains bounded and cancellable without
  lost protocol bytes, accepted outcomes or UI-thread waits.
- Real supervised SSH/container program plus descendant cleanup, selected Docker
  context/incarnation, missing toolchain/helper and borrowed-process safety; never
  stop a container or kill a reused/unowned PID.
- Actual ui-stdio open/edit/undo/search/collection/review/save/terminal/lifecycle
  journeys agree with current TUI/headless semantics; malformed/partial/version-
  mismatched traffic and lost response/delta state fail or resynchronize truthfully.
- Current TUI install/update interruptions, verification failure, ownership routing,
  rollback identity and rendered site/version/download agreement.
- Seeded ID exhaustion, decision-source :explain behavior, pinned multi-arch inputs
  and required-mode fixtures that fail when their prerequisite is absent.

Run `docker compose run --build --rm test`, `model`, `verify` and applicable existing
required-mode native/SSH/container/WSL, package/static and public-promotion checks
on the exact candidate. Preserve TUI performance/boundedness and current proofs;
no source-text/wiring oracle substitutes for behavior.

The new generalized TLAPS, whole-core correspondence/calibration and core-assurance
gates belong to the **next separate release, 0057**, not AR16. Architecture must
finish correctly with existing evidence; it does not claim the expanded assurance
has passed. After 0057, 0058 owns the worker cutover/requalification before completion.

No checks/builds are run against another session's unfinished source merely to
validate this documentation restructure. The architecture implementation release
must produce the evidence above. Remove throwaway scaffolding and obsolete aliases/
comments; update actual user docs/changelog and the AR handoff ledger.

## 12. Downstream ownership and entry gates

| Consumer | Consumes from 0056 | Still implements itself |
| --- | --- | --- |
| 0057 core verification | Stable implemented AR01–AR16 candidate, semantic contracts, actual code/helper/effect paths and existing test/model/proof/native evidence. | VF01–VF20 whole-core inventory, required generalized theorems and production kernels, calibrated correspondence/helper/Loom/native evidence and exact-candidate assurance gates. |
| 0058 native worker | Implemented AR contracts and the separately accepted VF baseline: source/operation identities, bounded service lifecycle, release catalog/install ownership and real consumers. | One local/SSH/container worker protocol, shared Rust native effects, authorized deployment, full Python cutover and all WK assurance migration/additions. |
| 0059 completion | Admitted mutation/input/source API and the worker-requalified bounded LSP/service/queue contracts. | C01–C09 providers/query/index/resolve/acceptance/configuration, UI behavior, real-server performance and completion-specific assurance extensions. |
| 0060 debugger | Admitted engine API, views/coordinates/bindings, bounded service/framing/exec leases, effects/privacy and the requalified native worker. | strop-dap client, adapter profiles/build/attach, stop/frame/variable lifetimes, breakpoint/evaluation semantics and debugger UI/tests. |
| 0061 GUI | Readonly action/view/input contracts, separate ui-stdio/protocol/server, recovery/outcomes and worker-backed services. | Native Windows GPUI process/window integration, renderer/cache, font/IME/pointer/UIA, native capture/automation and complete graphical parity. |
| 0062 distribution | TUI installation/ownership/catalog foundations and 0058's completed native-worker deployment/assurance. | Signed Windows installer/launcher, selected-WSL backend activation, GUI asset/channel expansion and real native installation evidence. |

0057 starts immediately after AR01–AR16 closes. Then 0058 WK01–WK20 closes before
0059 completion, 0060 debugger and 0061 GUI; 0062 supports GUI delivery. The user has
already dispatched AR/VF work: do not fold worker implementation or new worker proofs
into its active candidate. All C/DBG/UI/PKG scope remains, and WK adds its own gate.

Later feature entry includes 0057's baseline plus 0058's explicit assurance migration
and requalified native worker. New source/admission/effect/protocol behavior updates
affected targets and evidence. Inherited models/proofs do not automatically prove
new worker, completion, DAP, native GUI or installer code.

**Not in this release:** expanded 0057 assurance, the subsequent 0058 worker cutover, completion,
a GUI/window/GPU/IME/UIA provider, DAP adapter/session features, remote interactive
PTYs beyond completed 0055 scope, native Windows workspace/ConPTY support, an always-
running reconnect daemon, arbitrary tasks/plugins, automatic provisioning/elevation,
new graphics frameworks or signing-service enrollment. Their absence does not make
this architecture release incomplete; missing AR behavior or existing evidence does.

## 13. Evidence and source anchors

- User-confirmed render admission and missing dirty-draft backup; prior sweep in
  [0060 §2](0060-debugger-workflow-and-architecture.md), the GUI research retained
  under [0061](0061-gui-windows-and-wsl.md), and [0046](0046-engine-extraction.md).
- Current inspected `editor/events/channel.rs`, `strop-containers/src/exec.rs`,
  `strop-core/src/id.rs`, `editor/explain.rs`, Dockerfile and container test gate
  establish the boundedness/context/ID/provenance/build inventory and the already-
  fixed required-mode behavior. These are source findings, not new runtime claims.
- [0049 §9](0049-product-and-architecture-handoff.md) records the earlier named
  boundary/quality debt; preceding releases may already close individual items.
- [0042](0042-shared-resource-identity.md), [0043](0043-change-plans.md),
  [0044](0044-editable-collections.md), [0054](0054-unified-filesystem-workspace.md)
  and [0055](0055-embedded-terminal-tui-and-gui.md): reuse their real identity,
  mutation, source, input and terminal contracts rather than introducing duplicates.
- [0006](0006-e2e-harness.md), [0029](0029-session-tracing.md),
  [0002](0002-build-and-release.md) and [0062](0062-distribution-and-wsl-onboarding.md):
  existing testing/capture/release policy and the division of distribution work.
- [0045](0045-verus-pilot.md), `strop-core/src/editmap.rs`, `specs/*.tla`,
  `specs/check-model.sh`/`gate.sh`, Dockerfile and CI/tag workflows: existing
  Strop model/proof coverage, actual production binding and the open composition
  obligation. No new proof is claimed by this plan.
- [0057](0057-core-verification-and-assurance.md) owns the expanded whole-core
  assurance programme, Gripsack precedents and open composition/theorem/runtime
  obligations as the immediate next release. No new proof is claimed here.
- [0058](0058-unified-native-worker.md): subsequent native-worker decision and
  verification migration, not a change to the already-dispatched AR/VF ownership.
