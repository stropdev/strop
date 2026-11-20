---
rfc: "2026-09-06-consolidation-round"
title: "Consolidating editor contract regressions"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-06"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/119"
milestone: "v0.10.3"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0023"
---

# RFC: Consolidating editor contract regressions

v0.10.3 closed the 18-probe review: clipboard replies retain their destination, symlink saves write through, panes survive scratch changes and tiny layouts, and register and Unicode operations use the right coordinates. It also made the document epoch authoritative for syntax invalidation and bound gutter results to the originating document ([378f36e](https://github.com/stropdev/strop/commit/378f36e965ea9d7261c889d7c1e430df8aa8caf1)).

[Tracking issue #119](https://github.com/stropdev/strop/issues/119) · [Published v0.10.3](https://github.com/stropdev/strop/releases/tag/v0.10.3) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0023-consolidation-round.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: accepted 2026-09-06. The fourth review ran 18 contract probes
against 3e5c7f4 — 264 tests and 41 nvim cases passed; the 18 probes
fail. The probe module ships in the repo as the acceptance suite
(editor/contract_probes.rs). Pushbacks at the end.

## P0 — crashes and text destinations (the probe failures)

1. `:vs <file>` + `ctrl-w h` from scratch panics — opening the file
   drops the scratch while the old pane references it. Panes get
   rebound when a document dies beneath them.
2. Stale replace range over a Unicode edit panics instead of
   reporting stale — boundary validation before any slice.
3. `d/f` on `界foo` panics in preview — search starts one byte into a
   multibyte char; ceil the start.
4. Clipboard reply pastes into the newly active document — the pending
   paste must retain its initiating document id and reject or retarget.
5. Save through a symlink replaces the link with a regular file —
   write through to the resolved target (vim's behavior).
6. More panes than cells panics the renderer — saturating geometry.
7. `df2` no-ops — after `f`, digits are the target character, never
   count digits.
8. `:1y` writes register `'\0'`; paste reads `'"'` — one register name
   for the unnamed register.

## P1 — the revision clock (the shared root cause)

`history.depth()` counts allocated nodes, not text state: open insert
transactions don't move it, undo returns to it. Every consumer moves
to `Buffer.epoch` (the text clock, bumped on every mutation): syntax
invalidation + tree bridge revision, hover/nav freshness. The anchor
watermark keys on (document, epoch). `GitJob::Hunks` carries the
document id. Undo/redo run the same anchor mapping as commit.

## P1 — the InputEdit coordinates

Single-line inserts report `new_end_point` without the start column
(multi-op transactions also need per-op states; the bridge computes
each op's points against a moving position model, not the final text).

## P1 — the Unicode picker seam

`Utf32Str::Ascii` on non-ASCII rows was misuse (the review is right:
library misuse, not a library flaw). Rows use the checked Unicode
constructor; match columns are char indices. Tabs render through the
same LineLayout the caret uses (one layout for glyph + caret).

## P1 — distribution honesty

Release builds depend on the test target (tagged source must pass the
gate to release). Installer checksum policy matches the updater's
(mandatory verification). The scorebench example measured the wrong
thing after the swap — it now measures `Picker::refilter` (the real
path) as a regression bench.

## Pushbacks (recorded)

- **"72 fields on Editor" → a full transaction-gateway rewrite now:**
  partially adopted. The `apply(doc, revision, changeset)` gateway is
  the 1.0 architecture and lands with the perf suite; this round fixes
  the clocks/identities that make probes fail, without pausing the
  editor for a rewrite.
- **Preview timing:** the `ci[` sequence has no inspection window —
  real. The site claims narrow NOW: previews exist for composition
  windows (search, replace, git ops, long counts), not for instant
  operator+motion chords. An explicit inspect mode is a roadmap item.

## Roadmap additions

- The single transaction gateway (`apply(doc, rev, changeset)`) —
  1.0 architecture headline.
- Session crash-recovery journal (dirty snapshots) — 1.0.
- PTY lifecycle suite (resize/paste/real terminal) — 1.0.
- LSP completion + rename + code actions — the adoption gap, after
  the 1.0 gate.
- Preview inspect mode (explicit confirmation for operator targets) —
  post-1.0.
