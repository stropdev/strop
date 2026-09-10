# 0041 — Architecture handoff: adoption proposal and prioritized roadmap

Status: **reviewed and adopted in part** (9 Sep 2026 session). Landed:
S1 identity (0042: strop-workspace, registry, `:explain`), S4a/S4b change
plans (0043: format/rename/code actions/`:undo-change` + ChangePlan.tla),
S5 v1 editable collections (0044), S3 DC1a container browse/read (0037
amendment). The bench stress fixtures and chunked forensics shipped with
them. Remaining stages keep their ranks below; the storming session
continues from there.
Source: `strop-implementation-handoff.md` (9 Sep 2026), written against the
0.19.0 tree. The handoff is unusually well grounded — it cites
real files, respects the numbered plans, and defers the GUI per user instruction.
This plan maps it onto strop's plan system: what to adopt, in what order, what to
defer, and where to push back.

Nothing in this plan is started until the review session settles the adopts.

## 1. ROI-ranked disposition of the handoff's stages

Rank is reward-per-effort for strop as it exists today, informed by the 0.19.1
field report (enterprise SSH/NFS reality), not by the handoff's own ordering.
P0/P1/P2/P3 keep their 0028 meanings; "adopt" means a numbered implementation
plan gets written; "defer" stays here with its re-entry condition.

| # | Handoff stage | Decision | Why (ROI reasoning) |
|---|---|---|---|
| 1 | S0 leftovers: `ciW` WORD objects, chunked forensics | Adopt — already on the roadmap (0028 P2) | Grammar fidelity is doctrine 1; unchanged by the handoff. |
| 2 | S1 identity slice: shared workspace/path/resource contracts; migrate `FileTarget`, LSP `target.rs`, Git `target.rs` | **Adopt first.** Next numbered plan | Highest leverage: the 0.19.1 field bugs (symlinked ancestors, path-spelling asymmetries) are exactly the "every consumer interprets paths alone" class. Also the prerequisite for containers (0037 DC1) and honest remote semantics. Medium-large effort, mostly mechanical migration with strong existing types to reuse. |
| 3 | "Explain editor decisions" buffer (handoff §S2 follow-on, rank 5) | Adopt early, small slice | Small effort, high daily value, zero architecture risk: reuse keymap/config/help data. Ships independently of S1/S2. |
| 4 | S4a single-doc change plans → LSP formatting/code actions; S4b rename with grouped undo | Adopt after S1 | Rank-3 user benefit; needs S1's identity contract to not paint over path confusion. The pure-plan/worker-preparation shape fits the existing transact gateway (0024). |
| 5 | S5 editable code collections | Adopt after S4 | The differentiating feature; genuinely hard (source mapping). Worth doing; not before change plans exist. |
| 6 | S3 existing-container attach (0037 DC1) | Adopt after S1; reorderable with S4 | Proves the workspace abstraction against a third backend. The handoff wants it before freezing workspace APIs — agree. |
| 7 | S7 named investigations/checkpoints/test evidence | Defer, P2 | Large persistence surface; revisit after S5 proves collections. Re-entry: user demand for long-running investigations. |
| 8 | S6 structural recipes | Defer, P2/P3 | Builds on S5; per-language semantic capability variance makes early ROI poor. |
| 9 | S8 devcontainer provisioning (0037 DC2–DC5) | Defer behind S3, as already planned | 0037 already sequences this; handoff adds trust-preview detail worth folding into 0037 when S8 starts. |
| 10 | Bounded event transport (§5: byte-bounded queues, backpressure) | Defer with a measurement gate | Real concern, wrong first move: 0038's turn limits hold today. Re-enter when a measured stall traces to queue depth, not before. |
| 11 | Verus pilot, Loom, Miri, fuzz campaign (§8.3, §9) | Defer, P3 | The existing TLC gates + differential harness catch the bugs we actually ship (this field round included). Verus's production-binding requirement makes it a toolchain project, not an afternoon. Re-entry: a pure algorithm with a demonstrated bug history (range/change-map is the right candidate when S4 lands it). |
| 12 | S9 structural Git review | Defer, P3 | Behind collections and change plans. |
| 13 | S10 external-tool contract | Defer, P3 | Reuse LSP/DAP until a concrete tool demands more. |
| 14 | R1 installed remote service | Defer, decision gate only | The handoff itself gates this on measured need. The 0.19.1 report shows the no-daemon path working on real enterprise hosts — the gate is not close to met. |
| 15 | G1/G2 GUI | Defer — user-deferred, final stage | Confirmed by the user again this round. Engine-boundary *compatibility* is preserved by doing S1/S2 cleanly, no GUI code. |

## 2. Pushback on the handoff (things not adopted as written)

- **Formal-methods breadth.** One bounded model per new protocol boundary is
  affordable and matches precedent (RemoteSave.tla). The full §8.2 matrix —
  six new models plus Verus plus Loom plus Miri plus fuzz — is process mass
  ahead of product risk. Adopt the per-boundary-model rule for S1's workspace
  binding and S4's change plan only; the rest wait for demonstrated need.
- **"Required regression scenarios" list (§9).** Good list; but as a gate for
  every stage it inflates each slice. Fold the relevant rows into each adopted
  stage's plan rather than maintaining a global checklist.
- **Queue bounding before measurement** (§5). The unbounded `mpsc` carrier is
  honest about its limits; bounding it changes failure modes (admission can now
  block/reject). Measure first under the mandatory stress fixtures the handoff
  lists — those fixtures are worth adopting immediately into `strop --bench all`.
- **Stress fixtures** (§5 measurement list: 100k-result stream, 1 MiB line,
  slow-SSH-while-editing, etc.) — adopt as bench scenarios; cheap and directly
  responsive to doctrine 3. Filed as part of the S1-adjacent hardening work.

## 3. The adopted next slice (to become its own numbered plan after review)

**Shared resource identity (handoff S1, slices 1–2 only):**

1. `strop-workspace` crate: native-path domains (client-local, remote POSIX
   bytes, URI, display label), `ResourceLocation`, workspace/incarnation
   identity. No SSH/container/LSP dependencies.
2. Migrate `FileTarget` (crates/strop/src/files.rs), `strop-lsp::target`,
   `strop-git::target` onto it; delete superseded shared definitions; no
   aliases, no default-to-local fallback.
3. Workspace registry owned by the editor; contexts resolved once, passed to
   jobs as stable bindings.
4. Regression coverage: same-spelled paths in two namespaces, slot reuse,
   reconnect with an old result, URI roundtrip without local probes.
5. Extend `RemoteWorkspace.tla` for the binding/incarnation separation; one
   deliberate fault per owner check.

Explicitly not in the first slice: workspace-registry routing of all I/O,
container attach, engine extraction, any GUI API.

## 4. Field-report items deferred here (from the 0.19.1 round)

- **Remote-save lock release on clean exit (P2).** `.strop-lock-<hash>`
  persists after `:q` — litter, not a defect. Today's stable inode is
  deliberate (0040 §3; RemoteSave.tla fault 6 covers namespace replacement).
  Safe release = unlink while holding the flock + acquire-side post-flock
  identity recheck with bounded retry + model amendment turning fault 6 into
  covered behavior. Do it when the save model is next touched for another
  reason.
- **`../` row attributes in remote listings (P3).** Renders `d?????????`;
  one SFTP stat of the parent in the list job fixes it. Cosmetic; batch with
  the next remote-listing change.
