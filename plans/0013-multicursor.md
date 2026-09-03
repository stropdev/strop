# 0013 — Multicursor

> Neovim 0.13 landed true multicursor; Helix was born selection-first.
> Strop's doctrine is vim-grammar fidelity (0001 decision 1) with a pure
> resolver shared by preview and execute (decision 2). This plan picks
> the model that keeps both.

Status: planned. Target: after 0.2.2 hardening, before or with M5.

---

## 1. The two reference models

**Helix** (`helix-core/src/selection.rs`): selections are the primary
editing construct; even a cursor is a one-char `Range { anchor, head }`.
Every command maps over a `Selection` set; `Transaction` remaps all
ranges through each edit. Selection-first *replaces* the vim operator
grammar (`wd` not `dw`) — adopting it wholesale would break doctrine 1.

**Neovim 0.13** (`:help multicursor`): cursors toggle on/off (`Q`,
`<C-LeftMouse>`, `nvim_mcursor()`), stored as extmarks in the
`nvim.multicursor` namespace. Each edit *cascades*: the same operator
runs once per cursor through the ordinary single-cursor machinery, then
overlapping results merge. Vim grammar untouched; `.` degrades to a
builtin cascade.

**Decision: nvim's interaction model, helix's machinery.** The grammar
(`strop-grammar`) keeps its single-cursor signature —
`resolve(buf, cursor, cmd)` stays pure and scalar. The executor maps a
resolved command over the selection set. Preview (0001 decision 2)
renders the primary cursor's resolution live and the secondary cursors'
resolved ranges as passive highlights — same resolver, no special-casing.

## 2. Data model (strop-core)

```rust
/// One cursor/selection. `anchor == head` is a plain cursor.
pub struct Range { pub anchor: usize, pub head: usize }  // exists today

pub struct Selection {
    ranges: Vec<Range>,  // invariant: sorted, non-overlapping
    primary: usize,      // the one statusline/preview follows
}
```

- `Selection` lives on `Buffer` (per-buffer, like history). Editor's
  scalar `cursor`/`anchor` fields become accessors over the primary
  range; a `debug_assert` guards sortedness/non-overlap.
- **Remapping through edits** is the hard part and helix already solved
  it: port the `Transaction`/`ChangeSet` mapping logic (associate each
  edit with range displacement; apply bottom-up). Boring and correct
  beats clever here.
- Overlap policy: after every command, merge overlapping ranges
  (helix `Selection::merge`). Two cursors typing the same text converge
  to one.

## 3. Interaction (nvim 0.13 semantics, strop keys)

- `Q` toggles a cursor at point (normal mode). Esc collapses to the
  primary cursor — the only always-available escape hatch.
- Visual mode extends the primary range; operators then cascade over
  all ranges.
- Insert mode: one live transaction across all cursors; typed text is
  mirrored per cursor; `<backspace>`/motions apply per cursor. The
  insert session remains one undo unit (0001 §5.5 grouping holds — the
  transaction simply spans N insertion points).
- Registers, counts, and dot-repeat fan out per cursor (nvim 0.13's
  cascade semantics for `.`).
- v1 non-goal: mouse cursor placement; incremental `C-n`-style
  select-next-match can land as a follow-up built on `/` search state.

## 4. What must change

| Layer | Change |
|---|---|
| strop-core | `Selection` set on Buffer; transaction-based range remapping; merge invariants |
| strop-grammar | none to signatures; `cursor_after` maps per range |
| editor normal/visual/insert | cascade loops replacing single-cursor paths; pending/preview renders all ranges |
| render | secondary cursors as reversed-video blocks; selection bands per range |
| picker/git/LSP | untouched (they operate on buffers, not cursors) |

## 5. Test contract

- Differential corpora (0001 §5.10) run single-cursor paths unchanged —
  the scalar path must be bit-identical when the set has one range.
- New grammar-level tests: cascade delete/yank/change over N ranges,
  overlap merge, undo of a multicursor insert as one unit.
- Golden cell-grid snapshots for secondary cursor rendering.

## 6. Risks

- **Cascade order bugs**: bottom-up application per buffer edit is
  mandatory; helix's ChangeSet discipline is the reference.
- **Preview honesty**: secondary-range previews must derive from
  `resolve` outputs, never re-computed heuristics.
- **Scope creep into selection-first editing**: resist. The vim grammar
  stays the product (0001 decision 1).
