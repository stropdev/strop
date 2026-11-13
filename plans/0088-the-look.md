# 0088 — The look: design language, original palettes, honest chrome (0.54)

- Status: **in force** for 0.54; adoption record for [0087](0087-independent-critique-0.53.md)
- Date: 2026-11-09
- Supersedes the visual parts of 0064, 0068, 0071, 0078 and 0080 where they disagree.
  0001 §4's design intent stands; this plan is how it is finally made true.

## 1. Why

The 0.53 critique (0087) measured what the owner felt: the UI has no hierarchy
because the palettes have no tonal ladder (statusline 1.07:1, surface 1.13:1,
Visual selection 1.30:1 against base), one grey does three jobs, chrome is painted
even when it carries nothing, and each widget was drawn by hand in isolation. Two
of eight themes are other projects' brands that ship their licences in the binary.
Beside the look, running the editor surfaced real UX defects: an info message hides
ex errors, `:e` resolves relative paths against a directory buffer, and declining
leader verbs say nothing.

## 2. The design language (the contract every renderer answers to)

All rendering reads colours from one per-frame `Tokens` value (`render/style.rs`)
and draws chrome through one primitive module (`render/card.rs`). Nothing in
`render/` names a hex value or builds a bordered `Block` by hand.

1. **Three planes, one ladder.** `bg.base` < `bg.surface` < `bg.raised`, each step
   1.07–1.30:1. The statusline sits on `surface`. The cursor line is painted in file
   buffers on `bg.cursorline`.
2. **One accent, one job:** where am I / what will happen — Normal chip, current line
   number, pending footer, input caret, matched characters. Modes are their own
   tokens (`mode.insert/visual/replace/pending/terminal`), never syntax slots.
3. **Cards:** border `fg.faint`, title `fg.text` bold, hints `fg.muted`, rounded,
   identical on every card. Focus is the border going to `fg.muted`.
4. **Chrome only when it carries information.** Scrollbar is a thumb, no track,
   hidden when content fits. Picker scope row only when a scope differs from the
   default. `Spaces:N` only when it differs from the configured default. A failed
   service is a 4-cell `LSP!` badge; ready services show nothing. Pane titles live
   in the pane's top border (`╭ 1 statusline.rs ● ──`), only with ≥ 2 panes.
   *Amended by [0103](0103-seams-and-splits.md): a title rule `─ 1 statusline.rs ● ──`, no corners.*
5. **One selected-row idiom:** full-row `bg.raised` + `▌` in accent at column 0.
   Applied/current = `●` accent after the label. Dirty = `●` warning. No `[x]`,
   `✓`, `▸`, `▶` elsewhere.
6. **Keycaps are the only chip:** help, which-key, welcome, picker hints and
   completion footers share one keycap renderer.
7. **Typography:** bold = headings and keycaps; italic = comments and unavailable
   rows; dim never; underline = links and the active search match.
8. **Four greys:** `fg.text`, `fg.secondary`, `fg.muted`, `fg.faint`. Comments are
   muted italic, punctuation secondary — never the same ink.
9. **Widths from content**, cards capped at 60 % of columns and ≥ 32; the picker is
   `columns − 4` at ≥ 100 columns and full-bleed below.
10. **Hints are whole chords or nothing**, everywhere.

**Amendment 2026-11-10 — File names.** A path is two inks, through one helper
(`render/file_name.rs`): the directory is context in `fg.secondary`, ending in
its own `/`; the basename is the identity in `fg.text`. The name is **bold where
it is a heading** — a grep group header, a pane title, the preview header, the
recovery card's subject, a directory buffer's own path — and regular inside a
list (files, buffers, diagnostics, git file lists, the statusline). A remote
file leads with its authority, `ssh:host`, in `fg.muted` as its own segment
(the statusline's namespace chip) — never spliced into the path. Clipping
spends the directory first, from the left, so the authority and the name
survive; an absolute path keeps its leading `/` ahead of the `…`. A grep group
header is a *ruled heading*: name, a `fg.faint` leader `───`, the hit count in
`fg.secondary` — the same grammar as a pane title in its border, so grouping
reads without relying on how a terminal draws bold. File names are never
chips (rule 6 stands), never the accent (rule 2), never a syntax ink.

Messages get their own **echo row** above the statusline, present only while a
message is live, ordered by severity (error > warning > info). Errors stay until
the next keypress; info expires (4 s or first keypress). The statusline has no
transient slot. The mode chip is fixed-width (8 cells) so the path never jumps.

**Amendment 2026-11-12 — the modeline (0098).** The chip is 10 cells, so every
label has a cell of padding either side. Distances on the row come from one
scale (0 glued, 1 inside a group, 2 between groups); the pending keys have a
left-aligned slot; `line:col` and the percentage are fixed-width fields, and
`line:col` is `fg.secondary`. A floating card keeps one empty row between
itself and the edge or row it hangs from. See
[0098](0098-modeline-design.md).

## 3. Palettes

The `Theme` struct becomes the token set of 0087 §4.2 (bg / fg / accent / mode /
diag / diff / syn / ansi16). Six built-in palettes, all original:

| Id | Kind | Note |
|---|---|---|
| `hone` | dark, **default** | amber identity kept, graphite base, visible selection |
| `slate` | low-chroma dark | absorbs Strop Ink's intent |
| `carbide` | high-contrast dark | every ink ≥ 7:1 |
| `vellum` | light | warm paper, umber accent |
| `paper` | light | owner's palette, re-mapped onto the tokens, ladder fixed |
| `paper-dark` | dark | same |

Hex values are 0087 §4.3 (Slate's comment raised to `#7f8593`). `dark`, `light`,
`strop-ink`, `catppuccin-*` and `nord` are deleted with their licence files and the
`strop licenses` subcommand (it listed only theme attributions). A config naming a
deleted theme gets the default and a one-line warning naming the six choices — the
same path as any unknown theme name; no alias table.

A theme test asserts the contrast rules of 0087 §4.2 for every palette (text ≥ 10,
secondary ≥ 7, muted/comment/every `syn.*` ≥ 4.5 on base and surface, selection ≥
1.45 light / 1.55 dark — Slate is held to 1.49 and documented as the floor, ladder
steps in range, on-accent ≥ 4.5). The process-global `AtomicU8` theme and its
re-exec tests are deleted: the theme id lives on the editor and `Tokens` is built
once per frame.

## 4. Adoption of 0087, finding by finding

| Finding | Decision | Lands |
|---|---|---|
| §3 design language, mockups | **Adopt** as §2 | 0.54 |
| §4 tokens + palettes, delete Catppuccin/Nord | **Adopt** as §3 | 0.54 |
| §4.4 degraded terminals | **Defer.** Detect truecolor first; quantise only when the probe says so | 0.55 |
| U1 feedback hides errors | **Adopt** (echo row, severity order) | 0.54 |
| U2 `:e` relative to directory buffer | **Adopt** (resolve against cwd, "new file" echo) | 0.54 |
| U3 permanent LSP badge | **Adopt** (`LSP!`) | 0.54 |
| U4 Esc in `/` `:` enters field-Normal | **Owner decision** — it is a deliberate 0050 behaviour and a fidelity call | — |
| U5 welcome says `?` | **Adopt** | 0.54 |
| U6–U10 picker counts, grep grouping, diagnostics rows, which-key, help | **Adopt** | 0.54 |
| U11 cursor line, selection | **Adopt** (§3) | 0.54 |
| U12 terminal numbering | **Adopt** | 0.54 |
| U13 empty devcontainer notice | **Adopt** | 0.54 |
| U14 `[keys]` remap, U15 `:tutor` | **Defer** | 0.55+ |
| U16 whole-chord hints, U17 declining verbs speak | **Adopt** | 0.54 |
| A1 render mega-functions | **Adopt for render** (`render_pane`, `render_picker`, `cmd_card` fall out of the primitives). `resolve_controlled`, `open_explain`, `execute.rs` → F2 | 0.54 / F2 |
| A2 per-grapheme allocation | **Adopt where the row renderer is rewritten** (runs, not graphemes); the rest F2 | 0.54 / F2 |
| A3 global theme | **Adopt** (§3) | 0.54 |
| A4 `Editor` regression | **Adopt the ratchet now** (field and `impl Editor` counts may not grow); decomposition stays F2 | 0.54 / F2 |
| A5 path projections, A8 mega-tests, A10 workspace lints, A12 unwraps | **Defer** to F2 | 0.55 |
| A6 card primitive | **Adopt** (§2) | 0.54 |
| A7 keymap 1234 lines | **Adopt** — split by section; which-key and help grouping read the same sections | 0.54 |
| A9 silent SSH skips, `STROP_REQUIRE_NVIM` | **Adopt** | 0.54 |
| A13 production allows, A14 `fade_mix` dead branch | **Adopt** where cheap | 0.54 |
| P1 composition theorem over hand-written Lean | **Adopt K1**: the batch fold moves into the kernel, the theorem is restated over the extracted function, a loop-direction mutant must break it | 0.54 |
| P4 forgotten kernel module | **Adopt**: the gate asserts every `pub mod` is in Charon's `start_from` | 0.54 |
| P2 trusted sort | **Adopt** `debug_assert` of sortedness before admission | 0.54 |
| P3 drift check in CI | **Defer**; local qualification runs the full gate | — |
| K2 `operated_span` | **Adopt if it lands clean** with its differential cases; else first item of 0.55 | 0.54 / 0.55 |
| K3–K6 | **Next**, in that order | 0.55+ |
| §8 website | **Adopt fully** — copy fixes, redesign on Hone/Vellum, docs generated from `docs/*.md`, roadmap page replaced by the STATUS in-force table, real-frame hero, `pages.yml` filter widened | 0.54 |
| Demo tape `Ctrl+w w` | **Adopt** (`Space w w`) | 0.54 |
| §9 CHANGELOG bullets, Zig error pointer | **Adopt** | 0.54 |

## 5. Gate

Golden cell-grid snapshots are the regression net this plan promised in AGENTS.md
and never had: the editor + statusline, echo row, split, files picker, grep
picker, help, which-key and completion surfaces, in `hone` and `vellum`, at 140×40
and 80×24. A small in-tree helper (`STROP_UPDATE_GOLDEN=1` rewrites) stores the
character grid plus a per-row style-run summary; no snapshot crate. Plus the usual:
compose `test`, `model`, `proofs`, release fixtures, a real tmux smoke of every
surface in §5's list.

## 6. Implementation record (0.54.0)

Shipped as planned: the token set and six palettes with the contrast test;
`Tokens` per frame and the process-global theme deleted; `render/card.rs`
(card, keycap, whole-chord hints, selected row) behind every bordered surface;
the echo row; the statusline of §2; thumb-only scrollbar; titles in the top
border; grouped grep hits, honest counts, message-first diagnostics rows; which-key
and help by keymap section (the keymap is now `keymap/bindings/{moving,editing,
insert,visual,search,leader,panes,git,pickers,terminal,ex}.rs`); 32 golden
cell-grid snapshots (8 surfaces × hone/vellum × 140×40/80×24); U2, U3, U5–U13,
U16, U17; A1 for `render_pane`/`render_picker`; A4 ratchet; A9; A13 (dead-code
allow); K1, P2, P4.

Departures, with reasons:

- **Palette values** moved where a §3 rule failed on the surface plane: Hone
  `fg.muted #858c9a`, `syn.comment #838a97`; Slate `syn.comment #878d9a`,
  `syn.punctuation #9ea3ab`; Vellum `bg.surface #fdfbf6`, `bg.raised #e5dfd0`,
  `bg.selection #d1c9b3`, `syn.comment #696d77`, `accent #8a510f` (keycaps — accent on raised — now ≥ 4.5:1, a rule the palette test enforces); Paper `bg.surface #eef1e8`,
  `bg.raised #dbe2d6`, `bg.selection #c2d2ba`, `syn.number #92592f`; Paper Dark
  `syn.punctuation #b4c0b4`. Carbide's separate black statusline is dropped
  (statusline = surface).
- **Echo-row severity glyphs**: `Editor::message` is an untyped string set at
  ~650 sites, so fresh messages rank above feedback by freshness (they clear on
  the next key) — that fixes U1 — but carry no severity glyph. Typing messages
  is F2 work. Info feedback has no timestamp, so it does not auto-expire.
- **`of 999+`**: the picker cannot tell a capped catalogue from a complete one;
  it shows `shown of total`.
- **Help stays one column**: two columns would truncate descriptions inside a
  searchable buffer.
- **`Editor` ratchet**: moving the palette onto the editor added a field; the
  palette, yank flash and cursor fade now live in `editor::Presentation`, so the
  ratchet went down (115 → 113) instead of up.
- **K2 deferred** to 0.55 (recorded in 0086 §9.1): production has no single
  operated-span step to move; each resolver arm computes its own span. The
  refactor to one normaliser, with its differential cases, comes first.
- **`#[allow(deprecated)]` on LSP `rootUri`** stays: servers that read only
  `rootUri` would break.
- **U4** (Esc in `/` `:`) awaits the owner's decision; unchanged.

Also fixed on the way: search and file-finder walks sort directory entries by name, so results no longer follow the filesystem's raw order (it made the grep golden differ between host and Docker); goldens redact the crate version so a release bump is not a visual change. `x` left its undo group open, so `xxu` undid both
characters (differential case `xlxu` added).
