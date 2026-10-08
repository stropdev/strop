# 0068 — Full UI polish: spacing, alignment, truncation, restraint

Status: **implemented** (2026-10-08) — shipped in 0.41.0. Before/after frames
are at the bottom.

## 1. Research

- **which-key.nvim** (the reference for this dialog class): window padding
  `{1,2}` (one row top/bottom, two columns right/left), column spacing 3,
  minimum column width 20, multi-column grid, title centered. Its core idea:
  *the card breathes* — keys and descriptions never touch the border and
  never clip mid-word.
- **Zed / VS Code command palettes**: centered, max-width card; label left,
  muted hint right; overflow fades or ellipsizes, never hard-clips.
- **Strop's own house standard already exists** and is the quality bar:
  `render/text.rs::clip_end/clip_start` (grapheme-safe, ellipsis-reserving),
  `render/completion.rs` (aligned menu + doc card), `render/diff.rs`
  (byte-exact sign/number columns), the welcome card's MUTED border
  (0001 §4: one accent, reserved for cursor mode / active match / selection).

## 2. Audit findings (two parallel code audits, file:line on record)

### Cards
| # | Surface | Defect | Fix |
|---|---|---|---|
| C1 | `which_key.rs` | Fixed 40-wide bottom-right card; descriptions hard-truncated at the border (no ellipsis, mid-word: "restore fro│"); unaligned key column; 1-cell inner padding; overflow hints silently cut at the bottom | Rebuild the card: bottom-center, width fitted to content within [32, screen-2]; keys right-aligned in a 2-space column, descriptions clipped with `clip_end`; 1-cell inner padding each side; a quiet `+N more` footer instead of a silent cut |
| C2 | `cmd_card.rs` | Inline match count after the input without reserved budget → truncation; doc strings unclipped; candidate name column offset by one cell vs typed text | Reserve the count's budget; clip docs with `clip_end`; fix the column |
| C3 | `hover_card.rs` | Height counted from unwrapped lines while `Wrap` is on → wrapped content silently cut at the bottom border | Count wrapped rows when sizing |
| C4 | `blame_card.rs` | Fixed 7 rows for 3 lines of content; unclipped summary | Fit height to content; clip summary |

### Picker & modeline
| # | Surface | Defect | Fix |
|---|---|---|---|
| P1 | `picker/mod.rs` | Entire card border is ACCENT (one-accent violation; the welcome card is MUTED); hardcoded `Rgb(0x3a,0x3d,0x4d)` rule color (a second, unnamed convention, duplicated in preview.rs); suggestion detail strings hard-truncated | MUTED border like the welcome card; name the rule color once in the palette; clip details |
| P2 | `statusline.rs` | Accent stacked on chip + status + dirty + multicursor + warning count simultaneously (mode is the only accent per 0001 §4); fit() degradation order mismatches its own doc comment | Mode chip keeps accent; move status/dirty/multicursor to SECONDARY/MUTED semantics (severity colors stay for diagnostics); align fit() with the doc |

### Buffer chrome
| # | Surface | Defect | Fix |
|---|---|---|---|
| B1 | `buffer/rows.rs` | EOL diagnostic note clips at 80 *chars* without an ellipsis | `clip_end` at the cell budget |
| B2 | `buffer/scrollbar.rs` | 1-cell thumb regardless of viewport ratio | Ratio-sized thumb, minimum 1 |
| B3 | `field.rs` | Label clipped to spaces silently when narrow | Ellipsis clip |
| B4 | `diff/list.rs` | Long paths push the right-aligned +N −M stats offscreen without clipping | Clip path with `clip_start` before the stats budget |
| B5 | `help.rs` | Section split relies on the first double-space; 6-hue section palette is intentional structure | Harden the split; leave the palette (0069 revisits hues anyway) |
| B6 | `buffer/directory.rs` | Byte-split label/metadata; hint string drift | Cell-safe split; keep one hint source |

### Explicitly good — do not regress
`completion.rs` menu/doc alignment, `text.rs` clipping, `diff.rs` gutter
math, preview's LEFT border + numbered gutter, the picker row `finish()`
band/ellipsis discipline.

## 3. Execution slices

S1 C1 (headline) → S2 P1+P2 (chrome restraint) → S3 C2–C4 (cards) →
S4 B1–B4+B6 (buffer chrome) → S5 B5. Each slice ships styled cell-grid
snapshots at 100×24, 80×24, 48×16, 24×10 (narrow!) plus a live headless
frame; no behavioral regressions in the existing 1,791-test suite.

## 4. Acceptance

1. The Space overlay and every submenu show *no mid-word truncation* at
   any of the four audit widths; overflow reads `+N more`.
2. Every card has uniform inner padding; no text touches a border.
3. One accent: only the mode chip, active match chars, and selection use
   ACCENT on the picker/statusline; the rule color comes from the palette.
4. Hover content that wraps is never cut at the bottom border.
5. All existing styled/snap tests keep passing unchanged except where a
   snapshot intentionally moves (recorded in the changelog).
6. Container gate green; 0.41.0 release with before/after frames attached
   to this plan's landed section.

## 5. Landed (2026-10-08)

All slices shipped as planned. Dispositions:

- **C1 which-key** — rebuilt: bottom-center, content-fitted width [20, area−2],
  one padded key column, `clip_end` descriptions with 1-cell padding on both
  sides, `+N more` footer. Regressions pin: full text at 100 cols, ellipsis at
  48, footer on overflow, horizontal centering. B5 (help split) and B6
  (directory split) audited *acceptable*: help rows are generated with
  single-spaced keys and the fallback span covers edges; the directory split
  uses `split_at_checked` with a plain-render fallback.
- **S2 chrome restraint** — picker card border ACCENT→MUTED (the welcome
  card's convention); the rule line's hardcoded `0x3a3d4d` turned out to BE
  the seeded `PAIR_BG` value and now reads from the palette; modeline
  hierarchy: dirty `●` and worktree `*` join `[RO]`/staged in HELD,
  multicursor and the transient status move to SECONDARY — the mode chip is
  the only accent left (0001 §4).
- **C2 cmd card** — search count right-aligned inside the field's width
  (was appended past it); candidate name column sizes to the shown
  candidates (was a fixed 10 that overflowed); names and docs ellipsize.
- **C3 hover** — new `wrapped_rows` estimator (word-boundary greedy,
  hard-break fallback) sized against the REAL widget in a test that renders
  samples and compares the last non-empty row; height math uses it instead
  of source-line counts.
- **C4 blame** — height fits the 3 content lines (was fixed 7); author/age
  and summary ellipsize.
- **B1 EOL note** — the 80-char char-count pre-clip is gone; the painter
  expands tabs against absolute stops first (a raw tab would emit `�`), then
  clips at the row's real width with an ellipsis.
- **B2 scrollbar** — thumb LENGTH = visible fraction of the track (min 1),
  start = `top/scrollable × travel`, in a `Thumb` struct shared by editor
  and terminal panes; the viewport test's position assertion moved to the
  new contract (row 3 under the formula at that fixture).
- **B3 field label** — narrow fields ellipsize the label instead of
  clipping it to blanks. **B4 diff stats** — long paths yield their prefix
  (`clip_start`) so `+N −M` always shows.

### Verification

Four-width sweep (100×24, 80×24, 48×16, 24×10) over Space/Space-g menus,
the `:` card, the picker and the modeline: no mid-word truncation anywhere,
uniform padding, `+14 more` at 24 wide, clean modeline degradation. Frontend
suite 124 passed (including the new which-key/wrap/thumb regressions);
workspace suite green; container gate green.

Before (the complaint) vs after, Space-g at 100×24:

```text
│  u   hunk: undo unstaged (restore fro│        │  u   hunk: undo unstaged (restore from index) │
(right-clamped 40-wide card)                      (centered, content-fitted card)
```

