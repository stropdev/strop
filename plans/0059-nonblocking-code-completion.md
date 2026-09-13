# 0059 — Nonblocking, precise code completion

Status: **unimplemented; deferred until the native worker and its assurance cutover**.
The user has dispatched [0056 architecture](0056-architecture-prerequisites.md) and
[0057 core verification](0057-core-verification-and-assurance.md). The authorized
[0058 native worker](0058-unified-native-worker.md) release follows both; only after
WK01–WK20 completes does this release start, before 0060 debugger and 0061 GUI.
C01–C09 remain required; a popup or one provider does not complete this plan.

Research baseline: Strop 0.28.0,
`c7a5ac6c9089cff654365a0376f6cf87d746db95`. No completion implementation,
benchmark result or passed completion gate is claimed by this document.

## 1. Release contract

| ID | Required for the code-completion release |
| --- | --- |
| C01 | Real LSP completions and real current-buffer word completions, with useful behavior when either source is unavailable |
| C02 | No blocking input/render work; newest-query admission, cancellation, independent stale-result rejection and bounded retirement |
| C03 | A polished caret-anchored menu with stable selection, readable kinds/source/detail, and selected-item documentation |
| C04 | Safe, complete acceptance through the mutation gateway, correct edit ranges/imports/undo, and no speculative source mutation |
| C05 | Bounded LSP synchronization/request transport under sustained typing; no hidden backlog of full-document snapshots |
| C06 | `[completion] enabled` and `auto_popup` controls with real lifecycle effects and correct manual-only behavior |
| C07 | Correct source/view ownership in ordinary buffers and a collection excerpt; safe, explicitly defined multicursor applicability |
| C08 | Capability-accurate protocol support, complete/error/cancel outcomes, and native-free replay/privacy integration |
| C09 | Deterministic concurrency/acceptance tests, real-server UI evidence, and measured typing/cancellation/memory performance |

An implementation agent may not silently defer one of these, call a partial
implementation “v1 complete,” or replace a required source with fake results.
An exception needs explicit user approval and a roadmap entry. The optional
extension list in §12 is the only pre-authorized scope beyond this release.

## 2. Product behavior

Completion helps finish the text being written. It must not interrupt the
writing, change unrelated code without the represented edit, or make the
caret/menu jump unpredictably.

Required sources:

1. **Language server**: use the selected source document's actual language
   service context, capabilities, ranges and ordering hints.
2. **Current source buffer words**: an incremental, revision-owned word index,
   not a full-buffer scan on each key or words copied from the wrong document.

For a collection, “current buffer” means the source under the caret, not the
concatenated collection headers, file paths and snippets from every language.
For plain text/no-server files, current-buffer completion remains a real useful
feature. If the server fails, do not invent suggestions or call word results
semantic results; source provenance stays truthful.

This is **code completion**, not AI edit prediction, an automatic code agent,
brace insertion, or the query-qualifier assistance required by 0051.

## 3. Consume the completed architecture and assured core

Entry requires **0056 AR01–AR16, 0057 VF01–VF20 and 0058 WK01–WK20 complete**, on
recorded candidates. Do not begin providers/indexing while those releases are in flight.
Consume their admitted source/mutation/input and worker-backed service/transport APIs;
update anchors after the cutover rather than coding against historical Python helpers.

Existing seams to consume, not alternate infrastructure to recreate:

- `strop-lsp/src/protocol.rs`, `caps.rs`, `client/api.rs`: typed requests,
  capability gating, `RequestStamp`, admission and replies.
- `client/queue.rs` and its released replacement/split: ordered per-connection
  transport with serialization off input, bounded retained work and synchronization
  barriers. Its historical unbounded full-rope queue is 0056 architecture work,
  qualified by 0057 before completion starts.
- `editor/trace/drive.rs`: recorded actions synchronize changed bindings;
  `editor/lsp/state.rs`: document/service bindings, request ownership and tape
  calls. Do not reuse the hover-versus-navigation owner slot for completion.
- `editor/remote_completion/`: useful precedent for cancellation plus independent
  prompt/ticket freshness checks. It is path completion, not code completion;
  do not rename it or conflate their state.
- `editor/analysis/`, `transact.rs`: owned incremental analysis and pre-edit
  journals. Reuse the publication feed for word-index maintenance.
- `transact.rs::apply`, prepared replacements and history groups: acceptance is
  an ordinary validated edit transaction, not raw mutable-buffer access.
- The shared field/list/card/geometry and documentation presentation from
  0050/0051: reuse presentation vocabulary, not the modal picker's entire state.

Completion request kinds, result/resolve decoding, buffer word indexing,
insert-mode menu ownership and completion settings are new work. Do not claim
those exist merely because LSP navigation and a popup renderer exist.

## 4. “Typing cancels completion” — the precise contract

**A keystroke immediately revokes obsolete query authority. It does not kill
the shared language server or destroy useful index maintenance.**

| Work | On further typing / context change |
| --- | --- |
| Old completion query/resolve/acceptance intent | Revoke its ownership synchronously; request cancellation; late work cannot publish or apply |
| Queued obsolete completion requests not yet sent | Remove/coalesce before transmission where safe |
| Sent LSP completion request | Send advisory `$/cancelRequest` when supported by the client path; still independently reject obsolete responses |
| Current-buffer query | Cancel/coalesce to the newest query; no backlog of one search per character |
| Incremental word-index maintenance | Maintain/catch up by document revision; do not restart a whole-file build on every character and starve the index forever |
| Shared LSP process / ordinary syntax service | Keep alive for the rest of the editor |
| Old candidate/index snapshots | Transfer final destruction to an appropriate retirement owner, not a large destructor on input/render |

### Ownership

Use a domain-local completion session and query generation. A query carries
only the identities whose lifetimes matter: source document/resource,
revision, language-service context/server, intended view, caret/prefix range,
relevant selection shape, invocation kind and settings generation.

Delivery requires both:

1. the result still owns the active provider/query ticket; and
2. source/view/revision/caret/mode/settings/projection state is still compatible.

Transport cancellation alone is not a correctness guarantee. Providers can
ignore cancellation, reorder replies, answer after Escape or finish after a
buffer closes/reopens. None of that may resurrect a menu or insert text.
IDs must not wrap into reused valid identities.

Use explicit states such as Inactive, Active session and Pending acceptance,
with per-provider Pending/Ready/Failed/Cancelled/Unavailable outcomes. Avoid a
set of unrelated `loading`, `done`, `cancelled`, `manual`, `visible` booleans
whose combinations implicitly define the protocol.

### Work placement and boundedness

The input side may inspect a bounded prefix, update ownership/selection,
clone a frozen rope handle and enqueue owned work. It must not:

- await a server, lock with unbounded contention, join a worker or block sending;
- scan/tokenize the whole buffer, sort a huge candidate set, or serialize the
  whole document into a String;
- spawn a process/thread per keystroke;
- perform filesystem/network work during render;
- synchronously free the last owner of a huge old candidate/index snapshot.

Use persistent owned workers and newest-only admission. Bound count **and
retained bytes** for candidates, documentation/data, indexes, pending work,
queued snapshots and retirement. Apply bounds before handing giant results to
the interactive loop, not after allocating/sorting them there.

Distinguish one *logically active* query from physically outstanding server
requests. Cancelled requests can still be executing remotely. Bound outstanding
unacknowledged completion work and define timeout/busy/backoff behavior without
blocking input or killing a healthy shared server.

Never drop required terminal bookkeeping to make a bounded queue fit. Preserve
cancellation/control delivery under a flood of suggestions or documentation.
Client-side truncation and server `isIncomplete` are different facts; neither
may be falsely presented as a complete candidate universe.

## 5. Word indexing and source merging

### Current-buffer words

- Initial indexing runs on an owned worker over a frozen source snapshot.
- Subsequent changes come from the existing mutation journal. Expand affected
  ranges to word boundaries and update occurrences/counts incrementally.
- An older initial snapshot can catch up through retained edits or be replaced
  by a newer snapshot under an explicit bounded policy. It cannot publish as
  current merely because it finished.
- Queries return a bounded ranked window; painting consumes a prepared window.
  A cold index is allowed to be pending—do not claim words are always instantly
  available or hide an initial full scan in the first keystroke.
- Define word boundaries consistently with the source/editor language policy,
  preserving actual spelling and Unicode. Ranking normalization must not change
  inserted text. Do not extract words from generated collection chrome.
- Keep scope to the current source buffer for this release. No background
  project crawl or unexpected cross-workspace/private-buffer collection.

### Merge/filter/rank behavior

- Sources progress independently. A slow LSP must not withhold valid ready word
  candidates; a cold word index must not withhold valid LSP candidates.
- Honor `filterText`/`sortText` and the protocol's text-edit filtering semantics.
  Do not replace a server's explicit replacement range with guessed word bounds.
- Preserve the selected candidate by real identity while new data arrives.
  `label + kind` is **not** sufficient identity: overloads, imports and providers
  can produce the same label with different edits.
- Deduplicate only when represented insert/edit semantics are equivalent.
  Keep distinct candidates or provenance when operations differ.
- Do not move explicit selection to a neighboring/new first item when the
  selected candidate disappears. Clear it or preserve an operation-equivalent
  candidate under a proven mapping; never accept a different item accidentally.
- Complete cached lists can be locally refiltered while a compatible prefix
  grows. `isIncomplete=true` requires re-requesting with the current context.
  Late results of an obsolete query are still rejected.
- A cached visible item is not automatically a valid current edit. Acceptance
  requires current validation/adaptation of its source ranges and required
  effects; otherwise wait nonblockingly for a fresh prepared result or refuse.

## 6. Interaction and visual design

### Keys and ordinary typing

Choose Vim-honest, explicit acceptance:

- **Ctrl-Space**: manually request the combined completion menu.
- Support the conventional **Ctrl-X Ctrl-O** language-service route and
  **Ctrl-N/Ctrl-P** keyword/menu navigation through the same subsystem where
  these are advertised in the keymap. Do not create a second completion engine.
- Arrow keys or Ctrl-N/Ctrl-P choose a candidate while the menu is active.
- **Ctrl-Y** accepts an explicitly chosen candidate.
- **Ctrl-E** dismisses suggestions and preserves the typed text in Insert mode.
- **Escape dismisses completion AND leaves Insert mode in the same event.**
  No new “press Escape twice” behavior. This is a hard regression gate given
  the original Escape failure that motivated these handoffs.
- Enter/Tab retain newline/indent behavior when there is no explicit selection.
  With a deliberately selected candidate, their documented accept behavior can
  share the same acceptance action. Mere menu appearance must not steal them.
- No automatic preview insertion into the buffer while moving the menu
  selection. The source changes only on an admitted acceptance or normal typing.

Handle keys with a typed outcome—menu navigation, acceptance, dismissal with
forwarding, or pass-through—not a boolean that accidentally swallows input.
All bindings and hints come from the existing registry. Query-field Ctrl-Space
belongs to 0051 query assistance; completion does not hijack it or Ex prompts.

### Menu

Use a compact non-modal card anchored to the source caret/replacement-prefix
location, not a centered file picker. Keep the typed prefix visible. Fit within
the active editing area, flip above/below at edges, and use stable geometry
while filtering rather than moving/resizing for every candidate label.

Schematic, not a screenshot:

```text
    request.re
    ╭ completions ──────────────────────────────────╮
    │  meth  retry_request        (attempt)      LSP │
    │▌ meth  retry_with_backoff   (policy)       LSP │
    │  word  retry_budget                       buf │
    ╰ C-n/p choose · C-y accept · C-e dismiss ───────╯
```

- Primary label first; quiet kind/source information; useful secondary detail;
  exact match emphasis; one full-width selected band. No bright chip barcode.
- A bounded number of rows, an honest overflow/incomplete indication, and a
  selected-item documentation area when useful space exists.
- Documentation reuses the common markup/fenced-source rendering from 0051,
  with its own selected-candidate ownership. Late docs cannot attach to the
  newly selected item.
- Resolve documentation lazily on selection where appropriate. It must never
  stall caret motion, typing, menu navigation or source editing.
- Automatic empty results should not produce a large blank popup. Manual
  invocation can show a compact truthful pending/empty/disabled/unavailable
  explanation. Expected supersession is not an error notification.
- Caret, active pane, source namespace and all clipping are real display-cell
  geometry. Keep the editor in Insert mode; the menu is not a new editor mode.

Do not copy GUI-only geometry, require an icon font, add AI ghost text, or open
a separate popup stack with independent focus rules.

## 7. Safe LSP edits and resolve behavior

Implement the supported protocol contract explicitly:

- Completion request context, trigger characters and `isIncomplete` re-querying.
  Bare item arrays are complete; `null` is an empty result, not a transport error.
- Correct precedence of `textEdit`, `insertText` and label fallback.
- Checked conversion using the negotiated position encoding. **Mutation ranges
  must be rejected when invalid, not clamped like navigation positions.**
- Implement both `TextEdit` and `InsertReplaceEdit`, advertising support only
  once the behavior is complete. Validate the shared start/prefix relationship.
  Default explicit acceptance uses the replacement range so completing in the
  middle of a word does not duplicate its suffix; present any alternate insert
  policy explicitly rather than guessing from the label.
- `additionalTextEdits` are part of the same represented acceptance. Validate
  non-overlap with the primary edit and each other, including same-position
  insertions. Imports apply once per owning document, not once per cursor.
- Preserve bounded opaque `data` and required tags/fields through
  `completionItem/resolve`. Advertise only implemented resolve properties and
  list defaults; do not claim every newer protocol extension by association.

### Acceptance is one validated operation

Prepare a `CompletionAcceptancePlan` against current source/selection state.
Apply all required edits through the existing mutation gateway as one logical
completion action, joining the active Insert undo group in its documented way.
A later `u` must undo the insertion/import effects coherently. Do not directly
splice a rope or bypass source/projection publication.

If preparation/resolve is required at acceptance, register an owned acceptance
intent and return to the event loop. Further typing, moving, Escape, a setting
change or a source revision change revokes it. **No late surprise insertion.**

**Never accept without required additional edits because resolve timed out.**
A documentation-only failure need not block an otherwise complete edit; missing
required edit data must keep acceptance unavailable or fail visibly while
preserving typed text. Do not replace a missing implementation with a label
insertion or silently dropped import.

Server commands with external effects follow the existing capability/trust
policy. Do not silently execute arbitrary completion commands or silently omit
an operation on which the item depends. Unsupported items are explained,
not converted into misleading plain-text approximations.

Snippet insertion is outside the first bounded release (§12). Do not advertise
snippet support until placeholder/tabstop behavior is implemented. If a server
still sends a snippet-formatted item, refuse/mark that item honestly; never
strip placeholders or insert `${1:...}` into code while claiming completion.

## 8. Multicursor and collection ownership

### Ordinary/mirrored selections

Current-buffer word completion can apply to compatible selected prefixes as one
validated edit group. LSP completion starts from the primary source context,
but its edits must **not** be blindly mirrored as a text delta everywhere.

- Track each participating prefix/range and source revision.
- Mirror only when the represented edit is valid for each target prefix and
  context. Different prefix lengths, suffixes, language contexts or capabilities
  require proper preparation or a named refusal.
- Additional document edits/imports are deduplicated per source.
- If semantic completion cannot safely cover a mixed-context selection, explain
  applicability before acceptance. Do not silently edit only the primary while
  advertising an all-selection operation.

### Collections

Completion inside an editable excerpt uses its **source document**, language
binding, revision, indentation settings, prefix and word index. The menu anchors
to the visible projection caret; acceptance maps through the current projection
and source gateway. Generated headers/gaps are not completion text.

One-source/excerpt LSP completion and compatible word completion are required
for this release. Fully general semantic completion across heterogeneous
workspaces/languages is not implied; unsupported combinations must be named.
Partial/following/readonly resources retain their existing admission rules.
No local-path or current-workspace fallback can give a remote/source item the
wrong server or write authority.

## 9. LSP transport must stay current under typing — C05

At entry, 0056/0057 establish off-input serialization, physical retention bounds,
coalescing and ordering, and 0058 has preserved/requalified them through its real
local/SSH/container worker protocol. C05 preserves these under completion traffic;
it does not repair generic queues or finish the worker migration for the first time.

Before enabling higher request frequency:

1. Account completion work in the real count/byte admission and retirement bounds;
   prove/check completion cannot bypass them with its own unsent snapshots.
2. Reuse the admitted coalescing policy for superseded unsent full-document updates
   when no consumer needs the intermediate version. Preserve didOpen/didClose,
   incarnation changes, request barriers and required mutation/lifecycle outcomes.
3. Remove obsolete queued completion requests before sending where possible;
   send advisory cancellation for sent requests. Do not corrupt an in-progress
   framed message by aborting it halfway through serialization/transmission.
4. Keep completion scheduling from starving normal sync, explicit navigation,
   cancellation and terminal bookkeeping. Do not reorder a request ahead of the
   document version it was admitted against.
5. If incremental synchronization is adopted, implement and verify its negotiated
   encoding/revision contract with the existing journals. It is not permission
   to guess UTF-16 positions from UTF-8 byte counts.
6. Record/replay the delivered decisions through the existing tape; live timing
   must not create a second headless implementation.

Do not hide a growing wire queue behind a fixed 250 ms popup delay. A short
measured automatic-request coalescing interval may be useful; manual invocation
and ready cached results should not wait for latency theater. Separate provider
request delay from input/render latency and from time to a useful suggestion.

## 10. Configuration is functional, not cosmetic — C06

Required user-facing controls:

```toml
[completion]
enabled = true
auto_popup = true
```

Manual-only:

```toml
[completion]
enabled = true
auto_popup = false
```

Fully disabled:

```toml
[completion]
enabled = false
```

Semantics:

- `enabled=false` wins. No completion requests, popup, completion-only index
  worker or hidden background completion search remains. Revoke pending
  queries/resolves/acceptance and retire resources off the interactive thread.
- Ordinary language servers, syntax services and unrelated shared features
  continue working. Disabling completion must not break `gd` or highlighting.
- `auto_popup=false` does not merely hide an automatically queried result.
  Ordinary typing initiates no automatic completion requests. Manual triggering
  still works.
- Once a **manual** session is explicitly active, further compatible typing can
  update that session; it must not become a stale one-shot list because automatic
  popups are disabled. Closing it returns to manual-only idle behavior.
- Turning automatic popups off closes/cancels an automatic session; an explicitly
  manual session can remain. Turning completion off cancels both.
- Reloaded settings affect admission and publication immediately. Old callbacks
  cannot resurrect the menu after disable→enable because they have old ownership.
- Include actual values/provenance in config help and `:explain`. Keep query/Ex
  completion distinct; manual query suggestions from 0051 remain available.

A minimum automatic prefix length and per-source enablement can be exposed as
clearly named settings when implemented. Avoid a large speculative tuning
surface. Defaults must be justified by the measured behavior, not copied from
another editor's debounce setting.

## 11. Code quality and verification

### Structure

Use a focused `editor/completion/` subsystem, for example session/ownership,
source adapters, word indexing, candidate presentation and acceptance concerns.
Keep the TUI renderer separate and reuse the shared geometry/palette/doc blocks.
Move protocol completion decoding into a dedicated LSP module instead of growing
an already oversized `lsp.rs`/`api.rs` handler.

Use descriptive types (`CompletionQuery`, `CompletionCandidate`,
`CompletionProviderSnapshot`, `CompletionAcceptancePlan`, `CompletionSettings`),
not `Ctx`, `Req`, generic `Data` bags or tuple forests. Opaque protocol `data`
remains opaque by contract; that is different from avoiding domain modeling.
No generic plugin/provider framework is needed for two concrete sources.

Honor the existing file-size discipline and split touched overgrown modules in
the same change. No compatibility shims, duplicate request paths, warning
suppression, placeholder implementations or tests that only assert field copies.

### Deterministic gates

Use production admission/delivery/mutation handlers under controlled schedules:

- old response after more typing; out-of-order sources; provider ignores cancel;
  old resolve after selection changed; buffer/pane switch; close/reopen; Escape;
  source/projection/settings changes;
- current complete-list reuse versus `isIncomplete` re-request; client truncation
  versus server completeness; same labels with different operations;
- current-buffer indexing catches up under typing without a scan per key or
  starvation; close/disable retires the correct index;
- no automatic request in manual-only mode; manual session continues correctly;
  disabling cancels pending acceptance; re-enabling does not revive stale items;
- Enter/Tab without explicit selection preserve ordinary typing; Ctrl-Y accepts;
  Ctrl-E preserves text; one Escape exits Insert and dismisses the menu;
- middle-of-word/suffix replacement, Unicode/UTF-16/CRLF positions, invalid ranges,
  non-overlapping imports, duplicate edits, failed resolve, unsupported snippets
  and commands; one coherent undo group;
- collection source ownership and compatible multicursor application; wrong
  namespaces, readonly/partial sources and mixed contexts refuse honestly;
- bounded data output cannot prevent cancellation/terminal outcomes, and a stale
  large snapshot is not synchronously destroyed on the UI path;
- native-free replay and payload-free metadata capture. Candidate text/docs/edit
  payloads follow the existing content policy, not an unrestricted debug log.

### Real measurement and visual gates

Benchmark **before and after on the same build profile/machine**. Do not use an
old 0049 timing as a universal speed guarantee. Record input enqueue→consume,
consume→frame, first useful suggestions, latest-query publication, cancellation
acknowledgment, wire backlog and retained-memory high water separately.

Required scenarios: rapid typing/backspace, slow/ignoring LSP, large buffer,
1 MiB line, many unique words, oversized completion responses, repeated
open/close, disabled/manual/automatic modes, selected-item resolve while typing,
and old-result retirement. A warm cached case alone is not acceptance.

Use p50/p95/p99/max and explicit fixture/build metadata. Aim to keep typing
within the editor's established interactive envelope; timing reports complement
deterministic work/queue/freshness bounds, not replace them with flaky CI clocks.

Drive real clangd/rust-analyzer (or other supported real servers) plus an owned
slow-response fixture through the actual TUI at wide/narrow/split geometries.
Styled grids must show selection/source/detail/doc hierarchy and edge placement;
text-only snapshots do not establish visual polish. Ordinary typing must remain
usable while every provider is slow or unavailable.

Run the Compose quality gate plus the applicable 0057/0058 `model`, `verify`, `tlaps`,
`core-assurance` and native/service/deployment lanes on the exact candidate. Register
completion query/index/resolve/acceptance extensions and update affected worker/core
claims, models, production proofs and correspondence; neither baseline proves new code.
Record exact checks, limitations and C01–C09 acceptance in this plan and roadmap.
No release is complete with only a renderer, fake provider or unbounded “async” work.

## 12. Authorized extensions after this bounded release

Record these in the roadmap with re-entry conditions; do not leave them as
silent missing behavior:

- A real snippet engine with placeholders, choices, linked tabstops and undo,
  before advertising snippet support.
- Independent filesystem/path, other-open-buffer or workspace symbol providers,
  after the required sources prove ownership, performance and privacy.
- General semantic completion across heterogeneous source/workspace selections,
  beyond the explicitly supported applicability in §8.
- Commit-character acceptance, speculative preview insertion or AI edit
  prediction only under a separate interaction/safety contract. None is needed
  for the required explicit-acceptance completion experience.

These extensions do not authorize deferring C01–C09. This whole release is an
explicit D01 deferral from 0051, not a reason to defer 0051's static query
suggestions or matching delimiter highlighting.

## Sources and research distinctions

- [LSP completion protocol](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_completion)
  and [current completion specification source](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/language/completion.md):
  edit/filter precedence, incomplete lists, lazy resolve and non-overlapping
  additional edits. Advertise only the implemented version/capabilities.
- [Zed completions](https://zed.dev/docs/completions): automatic-popup opt-out
  and Ctrl-Space manual invocation; code completion is distinct from edit
  prediction. This plan does not infer a double-press acceptance rule from
  documentation rendering.
- [Neovim Insert completion](https://neovim.io/doc/user/insert.html): explicit
  completion acceptance/cancellation and preservation of typed text.
- [Helix keymap](https://docs.helix-editor.com/keymap.html) and
  [configuration](https://docs.helix-editor.com/configuration.html): modal
  completion controls and configurable automatic behavior; their timing choices
  are precedents to measure, not mandatory Strop delays.
- [VS Code IntelliSense](https://code.visualstudio.com/docs/editing/intellisense):
  suggestion/detail/documentation hierarchy; no guarantee about Strop's latency.
- [0051 whole-editor contract](0051-whole-editor-polish-and-query-language.md),
  [0050 visual roles](0050-picker-visual-polish-handoff.md), and
  [0028 roadmap](0028-roadmap-and-review.md).
- [0056 architecture](0056-architecture-prerequisites.md),
  [0057 core verification](0057-core-verification-and-assurance.md) and
  [0058 native worker](0058-unified-native-worker.md): completed prerequisites,
  including the worker's assurance migration—not completion work or optional hardening.
