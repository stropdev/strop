# 0086 — Verified foundation: a Lean-proved kernel, rings outward, and a trim

- Status: **adopted 2026-11-09; F0 + F1 land in 0.53.0**; F2 onward is the roadmap
- Owner direction (2026-11-09): commission an independent critique and adopt what
  makes sense; adopt gripsack's ring design (Lean-proved kernel over the
  Charon/Aeneas translation, rings outward); sweep stale platform references and
  unused CI; delete the unshipped Windows GUI.
- Inputs: [0085](0085-independent-critique-0.52.md) (the critique),
  gripsack `plan/0056-verified-foundation.md` (the pattern).
- Supersedes: [0057](0057-core-verification-and-assurance.md) §1–§8 as release gates
  and its boundary inventory; the Verus/TLAPS/core-assurance parts of
  [0058](0058-unified-native-worker.md); [0061](0061-gui-windows-and-wsl.md) and the GUI
  half of [0062](0062-distribution-and-wsl-onboarding.md) (retired: the GUI crate is
  deleted); [0079](0079-local-linux-release.md)'s completion freeze. Runtime contracts
  those plans describe (save conflicts, recovery, worker admission) are **not**
  superseded — they move into rings below.

## 1. Why

0085 measured the problem: ~22k lines of verification artefacts (13.3k TLA+,
TLAPS, Python inventory/freeze/mutant registries, a 422 KB digest inventory)
with no recorded catch of a Rust defect; nine of ten Verus "kernels" whose
spec restates the code; a CI that has been red for eight releases while
releases shipped anyway; and a `strop-core` that is a grab bag shipping the
Verus runtime. Gripsack reached the same state and fixed it with plan 0056.
Its principles transfer; its protocol-proof ambition does not (§3).

## 2. Principles (adopted from gripsack 0056 §2)

1. **Rings.** A pure kernel at the centre, proved. Each ring outward holds less
   trusted code checked a cheaper way. Code moves *inward* to gain guarantees;
   models are never added *outward*.
2. **Proofs are about the shipped code.** The kernel is plain safe Rust,
   translated to Lean by Charon/Aeneas; theorems are stated over that
   translation. No hand-written Lean copy of the Rust.
3. **One proof language.** Lean 4. Verus and TLAPS retire. TLC survives only as
   a counterexample finder, never as the proof of record.
4. **Sans-IO.** Kernel code never touches the world: no IO, no clocks, no
   allocation of OS resources, no dependencies.
5. **The survival rule.** An artefact stays only if it is (a) a machine-checked
   proof about shipped code, (b) a test guarding behaviour a user can see, or
   (c) a counterexample finder that has caught a real bug. Anything that exists
   to prove we ran something goes.
6. **The release rule.** A release is a green gate on the exact source. Proofs
   run like any other test. No receipts, evidence inventories, freezes or
   re-pin commits.
7. **Generated claims.** The guarantee list is the Lean `Audit` output (theorem
   names + axioms), never prose.

## 3. Where strop differs from gripsack (0085 §4, adopted)

Gripsack's product *is* crash-safe transactions, so its centre is P1–P4 over a
storage model. Strop's product is vim-grammar fidelity and a preview that
cannot lie. The oracle for fidelity is `nvim --clean`, not a theorem; Lean is
for algebra that the oracle cannot exhaust (edit-map composition, range
normalisation, count arithmetic, admission bounds). Therefore:

- No ring-1 storage model or crash theorems in this plan. Strop's crash-relevant
  state (drafts, sessions, worker Store saves) is checked by a crash-injecting
  harness and property tests, not Lean.
- Every kernel theorem about the grammar lands **together with** the matching
  differential-harness coverage against Neovim (F3), so the kernel is tested
  against the oracle, not only against itself.

## 4. The rings

| Ring | Contents | Checked by |
|---|---|---|
| **0 Kernel** (`strop-kernel`: `no_std`, no deps, Aeneas-shaped) | edit-map algebra (position mapping, batch admission, composition); frame-size admission; arena generation arithmetic. **Next:** `Range`/`MotionShape` normalisation, operator-pending parser + count arithmetic, byte-slice motion scanners, worker session/deploy admission | Lean theorems over the Aeneas translation, axiom audit, one calibration mutant per theorem family |
| **1 Protocol** (sans-IO machines) | worker session/stream machine, save/conflict plan, draft-recovery cohort decisions, LSP queue admission, event-channel policy | property tests with shrinking (`transaction_conformance`), Loom, crash harness; ≤3 TLC models on probation |
| **2 Host** | `strop-fs`, `strop-worker`, `strop-remote`, `strop-containers`, `strop-git`, `strop-terminal`, `strop-trace` | small enough to read; fault injection; gated e2e over real sshd/docker |
| **3 Edge** | LSP wire, tree-sitter grammars, SSH/SFTP output, `docker inspect`, `devcontainer.json`, release catalog + tarball, terminal input | typed admission and bounds; fuzzing of the parsers (weekly) |
| **4 Product** | `strop-engine::Editor`, keymap + trie, `crates/strop` render/CLI | differential harness vs Neovim, preview==execute property, cell-grid tests, headless e2e |

Dependencies flow strictly inward. `strop-kernel` depends on nothing.

## 5. Adoption of 0085, finding by finding

| Finding | Decision | Lands |
|---|---|---|
| C1 red CI, decorative lanes | **Adopt.** Delete `native-linux`, `release.yml`, TLAPS and VF01; CI = `test` + `proofs` (path-filtered) + `model` + `container-test` + `ssh-pythonfree`; `release-local.sh qualify` runs the same lanes locally and refuses a red one | 0.53 |
| C2 duplicate keymap table | **Adopt.** Delete `strop-core/src/commands/bindings.rs` | 0.53 |
| H1 `Editor` god object | **Adopt, scheduled.** Subsystem structs + explicit interfaces; the four private channels onto `AppEvent` | F2 (0.54) |
| H2 `strop-core` grab bag | **Adopt.** 0.53: Verus leaves the release graph, edit geometry moves to the kernel. Dissolving the rest (`strop-text` + moves) is F2 | 0.53 / F2 |
| H3 verification without catches | **Adopt.** §7 trim register | 0.53 |
| H4 test hermeticity | **Adopt.** Quick wins in 0.53 (dead `dbg_*` tests; a path-dependent status-line fixture). **Partly reject** deleting `DISPATCHED`: ex commands, pickers, panes and visual absorbers still dispatch outside the trie, and that list is the only guard keeping them visible in `?`; it goes when they move onto the trie. Differential lattice + preview==execute is F3 | 0.53 / F3 |
| H5 build cost | **Adopt.** BuildKit cache mounts in the Dockerfile; serial deep lanes leave CI | 0.53 |
| H6 `Result<_, String>` | **Adopt, scheduled** (148 sites; mechanical but wide) | F2 |
| H7 platform text, `strop-ui-protocol` | **Adopt.** SECURITY.md rewritten, signing text gone, arm64 container mapping refuses, `--ui-stdio` server and `strop-ui-protocol` deleted with the GUI (owner decision: no GUI) | 0.53 |
| H7 historical update channels | **Reject.** Brew, cargo and mise installs of earlier releases exist and carry those receipts; `strop update` must keep refusing them by name. 0.53 corrects the advice (brew/cargo are retired → reinstall via install.sh; mise follows GitHub releases and still works) | 0.53 |
| M1 plans drift | **Adopt.** `plans/STATUS.md` ledger; bodies of retired plans are history | 0.53 (ledger), F2 (path fixes) |
| M2 unused deps | **Adopt** | 0.53 |
| M3 test-helper duplication | **Adopt, scheduled** (`strop-test-support`) | F2 |
| M4 layering | **Adopt** as the ring direction lint | F2 |
| M5 per-keystroke `String` label | **Adopt** | F2 |
| L1–L4 | **Adopt** where cheap | 0.53 |

## 6. Phases

**F0 — Trim and honest gates (0.53).** §7 "now" rows. CI and the local release
qualify run the same lanes; no lane exists that may fail without blocking.
*Exit:* a green CI run on the release commit; `verification/` contains no
inventory, freeze or measurement; no TLAPS; no Verus in `Cargo.lock`.

**F1 — Aeneas pilot (0.53, go/no-go).** Create `crates/strop-kernel` by *moving*
(not copying) `strop-core::editmap` (position mapping, batch admission,
batch composition), the worker frame-size admission, and the arena generation
arithmetic. Production calls the kernel. `proofs/` holds a lake project, the
committed Aeneas translation (`proofs/Extracted/`, drift-checked), theorems
re-establishing the former Verus postconditions in Lean, an axiom audit and one
mutant per theorem family that must fail. The `proofs` compose service uses the
same pinned Charon/Aeneas/Lean toolchain as gripsack, so the image is shared.
*Falsifiers (any → no-go):* Charon/Aeneas cannot translate the subset without
contorting the code; a proof needs an axiom equivalent to its claim; the gate
exceeds ~10 min warm. *No-go fallback:* the kernel crate and the trim still land;
the Lean lane is removed and the edit-map theorems are restated as property tests.

**F2 — Structure (0.54–0.55).** Decompose `Editor` (H1); dissolve `strop-core`
into `strop-kernel` + `strop-text` + moves (H2); dependency-direction lint;
`strop-test-support`; thiserror at library boundaries; `ureq 3`.

**F3 — Grammar into the kernel, with the oracle (0.55+).** Move `Range`/
`MotionShape` normalisation, the operator-pending parser + counts and the
byte-slice motion scanners into the kernel with theorems ("linewise ⇒ whole-line
bounds", "counts multiply and saturate", "resolved range contains the cursor or
is empty"). In the same release, grow `differential.rs` to the full
`Motion × Op × Object` lattice, require nvim in the gate (`STROP_REQUIRE_NVIM=1`,
pinned version) and assert `preview()` == executed range for every case.

**F4 — Expand (ongoing).** One decision per PR moves inward with a theorem; the
tests and models it makes redundant are deleted in the same PR. Candidates:
worker session/deploy admission, save-conflict plan, recovery cohort decisions.

## 7. Trim register

| Artefact | Fate | When |
|---|---|---|
| `strop-gui`, `windows.yml`, macOS/Windows cfg arms, committed TLC traces, `.tlacache`, `__pycache__` | deleted | done (`c309ef8`) |
| `strop-ui-protocol`, `strop/src/ui_stdio`, its tests, `UiSession.tla`, `--ui-stdio` | delete (no client) | now |
| TLAPS: `specs/*Proofs.tla`, `tlaps-gate.sh`, Dockerfile/compose `tlaps` | delete | now |
| Verus: all `verus!` blocks, `vstd`/`verus_builtin*`, Dockerfile/compose `verify` | delete; the decision functions stay as plain Rust; `editmap`'s theorems move to Lean | now |
| `searchguard::completion_is_honest` (no production caller) | delete | now |
| `verification/check.py`, `inventory.json`, `schema.json`, `freeze.*`, `baseline/`, `measurements/`, `native/`, `completion_*.py`, `qualify_completion.py`, `bench_*.py`, `check_mutants.py`, `mutants*` | delete; history stays in git | now |
| Dockerfile/compose `core-assurance` | Loom campaign folded into `test`; the rest deleted. The three Rust mutant seams (`cfg(strop_mutant)` branches inside shipped functions) are deleted too: their runner is gone and the tests they calibrated stay | now |
| TLC models | keep EditorProtocol, SearchLifecycle, WorkerSession (the only ones with Rust correspondence) on probation; delete the other 17 with their cfgs and gate scripts. Correction to 0085 H3: UiSession *does* have a recorded catch (the picker-ranking storm wedge, `editor/events/tests.rs` `forwarded_semantic_event_survives_a_full_lane`); it goes anyway because its subject, `--ui-stdio`, is deleted, and the regression test stays | now; probation ends 0.55 — delete any that caught nothing |
| `ci.yml` `native-linux`, VF01, `release.yml` | delete | now |
| `strop-core/src/commands/bindings.rs` | delete | now |
| `keymap/tests.rs` `DISPATCHED`, `dbg_*` tests | delete | now |
| `SECURITY.md` (Windows signing), README signing section | replace with a reporting policy | now |
| Unused deps (0085 M2) | delete | now |
| Plans 0018–0084 | stay as history; `plans/STATUS.md` is the ledger | now |

## 8. Gates

| Compose service | Contents | When |
|---|---|---|
| `test` | fmt, clippy `-D warnings`, `cargo test`, Loom campaign | every push, release |
| `proofs` | regenerate `proofs/Extracted`, refuse drift, `lake build`, axiom audit, mutants | CI when `crates/strop-kernel/**` or `proofs/**` change; always at release |
| `model` | the ≤3 probation TLC models | every push (cheap), release |
| `container-test`, `ssh-pythonfree` | real Docker / real sshd journeys | every push, release |
| `release` | static musl tarball | release |

A lane that fails blocks. There is no advisory lane.

## 9. Implementation record (0.53.0)

What landed, and where it departs from §6 as first written:

- **Kernel API.** `check_batch` (sort + validate, `Err(())`) became
  `validate_batch(len, &[(start, end)]) -> Result<(), BatchRefusal>`. Both
  production callers already sorted with `std` before calling, so the kernel's
  insertion sort was dead weight; the kernel now only checks, and the refusal is
  typed (`OutOfBounds`, `Overlap`, `Unsorted`). The theorem is stronger than the
  Verus contract it replaces: *accepted ⇔ strictly ascending, in bounds and
  disjoint*, and each refusal kind has a witness in the input.
- **What is proved** (`proofs/Audit.lean` claims, axioms ⊆ {propext,
  Classical.choice, Quot.sound}): `map_position` equals its saturating
  specification; that specification is monotone and stays in the edited buffer;
  `validate_batch` as above; batch composition (production's reverse-order fold
  = classify once in original coordinates); the bridge from `validate_batch`
  acceptance to the composition premise; frame header/body admission and
  `frame_ready` (ready ⇒ the body slice is inside the buffer, no overflow);
  arena generation advance/retire and index-space admission. Composition is
  stated over the specification `mapNat` and tied to the shipped function by
  `map_position_spec` + `mapSpec_eq_mapNat`.
- **Calibration.** Nine mutants in `proofs/mutants.py` (affinity, collapse
  target, unbounded end, shared start, ignored reach, header off-by-one,
  unbounded body, early retirement, short index space); each must break exactly
  its named theorem.
- **Falsifiers.** Charon/Aeneas translated the kernel unchanged on the first
  run (one clippy-preferred `slice::swap` was later removed with the sort); no
  proof needed a project axiom; the warm gate (translate, drift check, build,
  audit, nine mutants) runs in minutes. **Verdict: go.**
- **Where `proofs` runs.** Its pinned toolchain image is ~20 GB and does not fit
  a hosted GitHub runner, so it is a blocking gate of local qualification
  (`release-local.sh qualify`) rather than of the hosted `ci` workflow, which
  runs test, model, ssh-pythonfree and container-test in parallel.
- **The other Verus "kernels"** (`mutguard`, `viewguard`, `cohortguard`,
  `searchguard`, worker `session`/`recovery_policy`/`deploy_policy`) are plain
  Rust now — tested, not proved. Each is an F4 candidate only when its theorem
  would say more than its code.
- **A "flake" that was a hermeticity bug.** `editor/terminal/pty_tests.rs`
  launched the host's `$SHELL`. A readline shell (bash ≥ 5.1, fish) enables
  bracketed paste, and a bracketed multiline paste is not held for review, so
  `real_pty_paste_review_decisions_are_redacted_and_ticket_scoped` raced the
  shell's startup on developer machines (2/10 on the 0.52.0 base) while
  passing in Docker's `sh`. The three interactive-shell tests now run
  `:terminal exec /bin/sh`.

### 9.1 0.54 additions (0087 P1, P2, P4; 0088 §4)

- **The journal fold moved inward (P1, K1).** `editmap::map_through_edits(position,
  &[(start, old_end, new_end)])` folds `map_position` over a journal in
  application order — an explicit forward loop over the slice. It is the only
  pointwise remap through a journal: `editor/transact.rs` (marks, selections,
  navigation records, collection hit anchors — one call over the whole
  journal), collection rebuilds (`view_positions.rs`) and snippet stops
  (`snippet.rs`, whose ranges are now sorted before the batch is built, so the
  journal really is the batch descending). Collection excerpt *spans* and
  match retention stay per edit: they are span rules, not the point fold.
  Completion acceptance (`acceptance.rs`) was checked and does not fold
  `map_position`; it only calls `validate_batch`. The forward loop is shaped by
  production, which journals edits in the order it applies them, so a
  multi-batch journal needs no reversal; a single accepted batch applied back
  to front journals as the batch reversed.
- **Theorems over the extracted function.** `map_through_edits_spec` (the
  loop computes the `mapSpec` fold), `map_through_edits_monotone`, and
  `map_through_edits_composition`: for a journal that is a `Chain3` batch
  reversed, with `p + Σ new_end ≤ Usize.max`, the result is `directMap`. The
  old `batch_map_composition` claim is gone; `batchMap` survives only as the
  `Nat` proof device the composition reduces to (`batchMap_eq_directMap`).
  Claims: 11 → 13. Mutant `journal-reversed` (iterate the journal back to
  front) must break `map_through_edits_loop_spec`.
- **P2.** Both `validate_batch` callers `debug_assert!` the `(start, end)`
  order of the pairs they just sorted; the typed `Unsorted` refusal stays.
- **P4.** `proofs/gate.sh` refuses a `pub mod` in `strop-kernel/src/lib.rs`
  that `[package.metadata.charon] start_from` does not name, and a
  `start_from` entry that is not a module.
- **K2 deferred to 0.55** (0088 §4: "else first item of 0.55"). The
  operated-span decision is not one function in production today — each
  resolver arm builds its own span, inclusive ends are `byte + 1`,
  `ceil_boundary(+1)` or explicit object bounds, and visual mode repeats the
  linewise rule. Moving a three-way selection over precomputed bounds into the
  kernel would prove a restatement while the arms keep their own arithmetic.
  What makes it fit: first make each arm return data (`cursor`, `target`,
  shape, or an explicit object span) and normalise in one place against the
  buffer's grapheme end, land the differential cases (`d$ dj dk d} dG v$d
  Vjd`, `dl` on a multibyte grapheme) with that refactor, then move the single
  normaliser inward.
