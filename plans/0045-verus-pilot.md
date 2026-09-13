# 0045 — Verus pilot: the change-map geometry, verified in production

Status: **landed** (10 Sep 2026, 0.20.1). Both seams proved in production:
`strop-core/src/editmap.rs` verifies in place (10 verified, 0 errors:
map_position spec-equivalence/monotonicity/in-bounds, check_batch sorted/
non-overlapping/in-bounds with honest error witnesses). Production calls
the verified functions — `editor/transact.rs` anchor remaps and
`buffer/mutation.rs` `prepare_replacements` geometry. Gates: compose
`verify` service (checksum-pinned Verus 0.2026.09.06 / rustc 1.98.0 /
z3 4.16.0) and a required CI job; negative control confirmed the gate
bites. Proof finds folded in: the no-overflow precondition and
`start <= new_end` invariant are explicit where they were implicit.

Open: batch-mapping composition (mapping through a validated batch equals
folding per-edit mappings) was the stretch property and is NOT yet proved —
the two landed seams do not claim it.

The open composition obligation and expanded **whole-core** verification now belong
to the separate [0057 VF01–VF20](0057-core-verification-and-assurance.md) release,
immediately after 0056 architecture and before completion/debugger/GUI. Preserve
the two landed proof seams below; the expanded assurance is not already done.
[0058](0058-unified-native-worker.md) follows that completed baseline and preserves
these edit/composition targets while requalifying changed callers and adding its
own worker/deployment/kernel obligations. A Rust port does not inherit proof status.


Original scope statement: the pilot boundary is the code
S4 made load-bearing: every change plan, collection write-back, mark,
jumplist and cursor rides `prepare_replacements` batch validation and the
anchor-mapping arithmetic in `editor/transact.rs`.

## 1. Target and properties

Two pure seams, extracted so production and the verifier share one source:

1. **Batch validation geometry** (`strop-core`): given buffer length and a
   list of (start, end, text) edits — after preparation the edits are sorted
   by start, non-overlapping (`end <= next.start`, no equal starts), every
   range inside bounds and well-formed (`start <= end`), empty no-ops removed,
   and the revision/epoch arithmetic cannot wrap.
2. **Anchor mapping** (`map_position`, currently `editor/transact.rs`): one
   anchor through one edit —
   - anchors before the edit never move;
   - anchors at or past the old end land at the new end plus their offset
     (right affinity);
   - anchors inside the replaced range collapse to the edit start;
   - mapping is monotone (`a <= b` implies `map(a) <= map(b)`);
   - results stay in `0..=new_len` (never off-rope).
3. **Composition** (stretch): mapping through a validated batch applied in
   reverse equals folding the per-edit mappings — the property multicursor
   and collection anchors rely on.

## 2. Production binding is the point

The verified function is the one strop runs. `verus!` blocks with
requires/ensures live in the same file as the implementation; the normal
build compiles them away via crates.io `vstd`/`verus_builtin` (weekly
0.0.0-<date> pins matching the verifier release). If the crates.io pass-through
does not work under the repo's stable rustc, the fallback is a thin pure module
(`strop-core/src/editmap.rs`) written in the verus-accepted subset with the
macro only — never a copied algorithm in a proof directory.

## 3. Toolchain isolation

Verus release 0.2026.09.06.8dea4a2 pins rustc 1.98.0 (checksum-pinned
tarball). The repo's TUI build floats on `rust:alpine` stable; the pilot never
touches it. A separate compose service `verify` builds a dedicated image
(rust 1.98.0 + the pinned Verus binary) and runs `verus` over the pilot
targets. CI gains a `verify` job, required once the pilot lands; provers stay
out of the shipping images.

## 4. Trust boundary and limits

Verified: the two landed seams above, under explicit assumptions (usize = 64-bit
machine arithmetic modeled as int with bounds; rope invariants — boundary
correctness of byte offsets — remain trusted, stated as such). Trusted:
ropey, the verifier, the solver, and the preconditions callers actually
establish. A solver timeout is a failed obligation, not green.

Fold-in rule: any geometry bug the proof work exposes is a strop bug — fix it,
regression-test it through the differential/unit suites, and it rides the next
release (0.20.1 if user-facing).

## 5. Exit

The verified seams pass `verus` in the compose `verify` service and CI job;
production calls them; the plan's acceptance record lists proved targets,
trusted assumptions, verification time and binary impact (expected: zero —
ghost code erases).
