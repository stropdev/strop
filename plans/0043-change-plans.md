# 0043 — Shared change plans: LSP formatting, code actions, rename

Status: adopted from 0041 (handoff S4), planned against the 0.19.1 tree.
Prerequisite landed: 0042 shared resource identity.

## 1. What exists and what is new

Exists: `Editor::apply(document, base, ChangeSet)` — the revision-checked
mutation gateway (0024) with prepared replacements and pre-edit journals;
project search/replace with its own preview/apply in `picker/replace.rs`;
LSP navigation/hover/diagnostics.

New: one **change plan** representation that every multi-edit producer —
LSP formatting, code actions, rename, project replace — feeds, with one
preview, one validation, one application path, and one receipt. No producer
mutates buffers directly anymore; the receipt is the undo/verify anchor.

## 2. Design

### The plan

`strop-changes` starts as an engine module (`editor/changes/`), extracted to a
crate only when a second consumer justifies it (handoff §6 rule). Pure: no I/O,
no async, no LSP types inside — producers convert at the boundary.

```text
ChangePlanId            generational identity of one plan
ChangePlan {
    producer: Provenance,          // LspRename{server}, LspFormat, CodeAction{..}, ProjectReplace
    base: Vec<DocumentSpan>,       // ResourceLocation + base revision per target
    edits: Vec<PreparedDocumentEdits>, // per document: sorted, validated replacements
    state: PlanState,              // Preparing | Ready | Invalidated(reason) | Applying | Done(ChangeReceipt)
}
ChangeReceipt {
    plan: ChangePlanId,
    applied: Vec<(DocumentId, before: BufferRevision, after: BufferRevision)>,
    refused: Vec<(ResourceLocation, Refusal)>,
}
```

Sorting, UTF-8/UTF-16 position conversion, overlap validation and preview
preparation run on a worker; the interactive thread registers the plan, shows
the prepared preview, and applies in bounded steps (never a loop over all files
in one input callback).

### Semantics

- Editing any source while a plan is Ready invalidates the plan (revision
  mismatch is detected at apply per document; a *visible* preview marks itself
  stale instead of applying quietly).
- Apply is grouped and version-checked, document by document, with an explicit
  partial outcome: applied targets carry before/after revisions; a conflicting
  target is refused and named. No all-files atomicity claim.
- **Apply, Save, Run tool are separate actions.** Saving after apply is the
  ordinary per-buffer save path with per-file receipts.
- Grouped undo is a new explicit command (`:undo-change` / leader binding TBD
  in review); it requires the receipt's recorded current revisions — intervening
  edits refuse, never a blind walk of per-buffer undo stacks. Vim `u` keeps its
  buffer-local behavior.
- LSP specifics: honor negotiated position encoding and document versions;
  versionless server edits are applied as unversioned proposals against a local
  snapshot with apply-time revision checks (the plan never claims to know which
  file version the server saw); `workspace/applyEdit` answers success only
  after real application; unknown URI schemes and unsupported resource
  operations refuse with reasons.

### Producer migration order

1. **S4a** — single-document producers: LSP formatting (`gq` stays as-is until
   the semantic-formatting binding is deliberately specified) and code actions
   (new picker kind over `textDocument/codeAction`). Exercises plan/preview/
   apply/receipt without multi-document risk.
2. **S4b** — rename (`:rename` / `gr`? — binding review) as the first
   multi-document producer; project replace migrates onto the same plan path,
   deleting its parallel apply logic in `picker/replace.rs`.

## 3. Safety and tests

- Stale proposal after source edit, document closed/reopened mid-plan,
  one read-only target among writable ones, second-file failure partial
  receipt, overlapping edits refused at validation, UTF-16 conversion at
  non-ASCII boundaries, unversioned server proposal, edit during
  application, duplicate completion delivery.
- Grouped undo after unrelated edits refuses; with clean revisions restores
  all documents and emits its own receipt.
- A `ChangePlan` TLA+ model (readiness/invalidation/ordered partial
  application/receipt ownership) with one deliberate fault per owner check,
  per the model-per-boundary rule adopted in 0041.
- Replay: recorded plan applications replay native-free from the trace.

## 4. Exit

Real rename and formatting workflows run through the common path; project
replace consumes it too; failures and partial results are visible; no
producer retains a private mutation route.
