# 0087 — Independent critique of 0.53.0 (look and feel, themes, UX, architecture, proofs, website)

- Status: **record** (input to [0088](0088-the-look.md), which decides what is adopted)
- Date: 2026-11-09
- Author: an independent reviewer agent (Claude Fable), commissioned by the owner;
  read-only against `75698ba` (v0.53.0) and the `stropdev.github.io` clone at `8e7f7db`.

The text below is the reviewer's report, unedited except for this header. Capture
files it cites (`scratchpad/caps/*`) were the reviewer's working material and are not
committed; every finding cites the source line it rests on.

---

- Target: `stropdev/strop` at `75698ba` (v0.53.0); website repo `stropdev.github.io` (local clone `8e7f7db`, live site fetched 2026-11-09).
- Method: read the doctrine (AGENTS.md, 0001, 0003, 0078, 0080, 0081, 0085, 0086), the render/theme/kernel/proof sources, built the 0.53.0 binary on the host (`cargo build -p strop-editor` with the pinned Zig 0.16.0 installed into the scratchpad) and drove it in tmux at 140×40 and 80×24 against this repository with an isolated `$HOME`/XDG, capturing ~60 cell grids with escape codes (`scratchpad/caps/*.ansi|txt`). Colours in the report were read back from those captures with `scratchpad/ansi.py`; contrast ratios were computed from the theme sources with `scratchpad/contrast.py` and for the proposals with `scratchpad/proposal.py`. Two sub-reviews (architecture measurements, website) were delegated and spot-checked.
- Labels: **[V]** verified in code or by running; **[S]** inferred. Line numbers are at `75698ba`.
- Not run: the Docker gates, the `proofs` image, clippy (host build only). Nothing here claims they passed.

---

## 1. Executive summary — top 10, ranked

1. **The UI has no hierarchy because the palette has no steps.** In the default theme the statusline background is 1.07:1 against the editor base, the picker/help surface 1.13:1, the cursor row 1.14:1 (and the cursor row is never painted in file buffers at all), the Visual selection 1.30:1. Everything sits on one plane; the only thing that pops is the amber mode chip. Comments, punctuation and line numbers all render in the same `#818696` (149 of ~480 styled runs in a Rust file are that one grey). This, more than any individual widget, is why the owner feels "the UI elements don't look right". **[V]** §3, §4.
2. **Mode colours are borrowed from syntax slots.** `render/mod.rs:234-242`: Insert = `class_string` (green), Visual = `class_keyword` (violet), Terminal = `class_type`. A theme cannot tune "insert mode" without recolouring strings. The theme struct (`strop-core/src/theme.rs:59-99`) has 28 colour slots plus ANSI, but no mode, hint, diff-fg, variable/property, selection-fg or on-accent slot. The token set, not the hex values, is the first thing to fix. **[V]** §4.1.
3. **Chrome noise: the full-height scrollbar track on every pane, a blank divider column plus the scrollbar, a title row per pane, `[x]` checkboxes and repeated file headers in grep results, a 30-cell permanent `lsp rust-analyzer@strop failed` badge and an undismissable "Dev container discovery completed · Space a actions" message that hides real errors.** In the grep picker 50 % of result rows are repeated headers (`[x] remote.md … 3 hits` printed once per hit). **[V]** §3, §5.
4. **Feedback precedence bug: an info-level feedback record outranks ex errors.** `:e other.rs` on a dirty buffer sets `"unsaved changes — :e! to force"` (`normal/ex.rs:364`) but `statusline/transient.rs:55-63` shows `presented_feedback()` first, so the user sees "Dev container discovery completed" and the command silently does nothing. Reproduced. **[V]** §5.
5. **`:e relative/path` from a directory buffer resolves against the directory, not the cwd**, producing a bogus new file `crates/strop/src/render/crates/strop/src/render/statusline.rs` with no error. **[V]** §5.
6. **0086 landed F0/F1 and nothing else moved:** `Editor` grew to 109 fields (+3), 162 `impl Editor` blocks across 137 files, the four private mpsc pairs are still on the struct, strop-core still does file IO and depends on `strop-trace`, 147 `Result<_, String>`. Genuinely new debt is concentrated in `crates/strop/src/render`: a 511-line `render_pane`, a 436-line `render_picker`, per-grapheme `String` allocation in the row renderer, a process-global `AtomicU8` theme read 58 times per frame, nine copies of the rounded-card `Block`, three path-display helpers and four clip helpers. **[V]** §2, §6.
7. **The proof ring is honest and small.** 176 lines of kernel, 11 audited claims, no `sorry`/axiom, nine calibrated mutants, production really calls the kernel (7 call sites). But the headline composition theorem (`batch_map_composition`) is about a hand-written Lean fold (`batchMap`, `EditMap.lean:255`), not extracted code — the Rust fold lives in `transact.rs`/`view_positions.rs` and is tied to the theorem only by a comment — and no mutant targets that family. The next inward moves with user-visible value are: the batch fold itself, `Range`→byte-span normalisation (linewise/inclusive), count arithmetic and the word/WORD scanners, each paired with differential cases. **[V]** §7.
8. **Themes: 8 shipped, 2 are third-party ports (Catppuccin, Nord) whose licences ship in the binary, 2 are near-duplicates (Dark vs Strop Ink), Catppuccin Latte's syntax inks are 3.5:1 (fail AA for body text), Nord's error red is 3.05:1.** Keep Paper/Paper Dark (reworked onto the new token set), replace Dark/Light/Strop Ink with the four original palettes in §4 (Hone, Vellum, Carbide, Slate), drop Catppuccin and Nord (a zero-config editor should not carry other projects' brands and licences in `strop licenses`). **[V]** §4.
9. **Website: the live site is current for 0.53.0 but lies about the product in three places** — "Paper is not an editor theme" (it has shipped since 0.47.3), GUI described as "deferred" after deletion, roadmap stops at 0.32.4 and names retired plans — and the demo tape sends `Ctrl+w w` which `panes.rs:295` asserts is no longer bound ("the demo must never lie"). The real product is a 7.5 MB GIF hidden behind a closed `<details>` under a fake SVG editor. **[V]** §8.
10. **The design language in 0001 §4 / 0003 §5 was good and has been half-abandoned:** "help as centered floating cards with aligned key tables" is a plain buffer with `[section] ───` rules; "one accent colour" has become six section hues in help (`render/help.rs:13-22`); "floating panes dim the border column for depth" is not implemented; the which-key card lists 21 unsorted rows in table order. The fix is to write the design language down as a short contract (§3) and derive every widget from one `Card`/`Chip`/`Row` primitive. **[V]**

---

## 2. 0086 scorecard

| 0086 item | Promised | State at 0.53.0 | Verdict |
|---|---|---|---|
| C1 honest CI | delete native-linux, release.yml, TLAPS, VF01 | `ci.yml` = test/model/ssh-pythonfree/container-test/gate; `verification/` gone; 5 `.tla` left (3 models + 2 mutants) | **done [V]** |
| C2 duplicate keymap | delete core copy | `strop-core/src/commands/bindings.rs` gone; engine table 1234 lines, 154 rows, not in 0028 | **done**, ceiling violation uncommented |
| H5 Docker cache | cache mounts | `Dockerfile:32-33,45-46,63-64` `--mount=type=cache` | **done [V]** |
| H2 Verus out of release graph | remove vstd | `grep -c vstd Cargo.lock` = 0 | **done [V]** |
| F1 kernel + Lean | move editmap/frame/generation, prove, mutants, drift gate | `crates/strop-kernel` 176 lines, 0 deps; `proofs/` 11 claims, 9 mutants, drift check; `proofs` runs only in local qualification (image ~20 GB) | **done [V]**; see §7 for what is not yet about shipped code |
| H1 `Editor` god object | F2 (0.54) | 109 fields (+3), 162 impl blocks / 137 files, `preview/git/clip/shell` tx/rx pairs at `editor/mod.rs:120-178` | **not started; regressed** |
| H2 strop-core dissolve | F2 | same modules; `tempfile` runtime dep used at `buffer/io.rs:203`; depends on `strop-trace` | not started |
| H6 `Result<_, String>` | F2 | 147 (was 148) | not started |
| M3 test-support crate | F2 | none; two test files > 800 lines (`picker/tests.rs` 1022, `filesystem/tests.rs` 822) with no 0028 entry | not started |
| H4 hermeticity | quick wins 0.53 | `dbg_*` gone; `STROP_REQUIRE_NVIM` does not exist anywhere; nvim skip prints a notice; 9 SSH gates in `strop/tests/remote_ssh*.rs` still `return` silently on host | partial |
| H4 golden snapshots | AGENTS.md promise | 0 `.snap`/insta; 72 `TestBackend` sites assert cells inline | unchanged |
| M1 STATUS ledger | 0.53 | `plans/STATUS.md` exists; `plans/0084` is untracked in the working tree | done; one plan outside git |
| L2 allows | cheap | 3 `too_many_arguments` + 1 `deprecated` + `#[allow(dead_code)]` at `strop-worker-deploy/src/container.rs:99` in production | mostly unchanged |

0085's "140 unwrap/expect in non-test code" over-counted `#[cfg(test)]` modules; the real figure is ~45 sites in 22 files (`editor/collections/projection.rs` ×8, `normal/execute.rs:119,143,150,207`, `grammar/resolve/objects.rs:28,29,81`). **[V]**

---

## 3. Look and feel: what is there, and a design language to replace it

### 3.1 What the screen actually shows (default theme, 140×40, Rust file)

```
   1 //! The modeline (0032): one quiet row at the terminal's bottom edge.                      ▮
   2 //!                                                                                         ▮
   3 //! The hierarchy borrows from the observed OMP footer — a compact mode                     │
  …
  96 impl Modeline {                                                                             │
  97 │   fn gather(editor: &Editor) -> Self {                                                    │
  98 │   │   let buf = editor.buf();                                                             │
▌ NORMAL  │ main │ crates/strop/src/render/statusline.rs   Dev container discovery completed · Space a actions │ lsp rust-analyzer@strop failed │ Spaces:4 │ 96:1 17%
```

Observed problems, each confirmed in a capture:

| # | Element | Observation | Evidence |
|---|---|---|---|
| L1 | Scrollbar | A `│` track runs the full pane height on every pane, even on the empty welcome buffer, with a `▮` thumb in `border` colour (2.67:1 vs base). It is the most visible vertical line on screen and carries no information when the file fits. | `render/buffer/scrollbar.rs`, caps `01-welcome`, `02-file` |
| L2 | Split divider | Vertical splits paint a blank column *and* the left pane's scrollbar; horizontal splits paint a full-width `─`. Two different divider idioms. | `render/buffer.rs:187-209`, cap `14-split`, `15-split3-dir` |
| L3 | Pane titles | Only in splits, a full row per pane: `1  crates/strop/src/render/statusline.rs` / `2 ▌README.md`. Number + `▌` focus mark + path + `*`; the single-pane case has no title, so the layout jumps by a row when you split. | `render/buffer.rs:128-134` |
| L4 | Gutter | Sign column + 3-digit number + space; number right-aligned in `muted`, current line in accent. Git signs `▎`, diagnostics `●`. Fine, but `muted` is the same colour as comments and punctuation, so the gutter visually merges into doc-comment blocks. | cap `theme-dark.ansi`: `#818696` for `97`, `{`, `//!` |
| L5 | Indent guides | `│` in `guide` (1.62:1) at every indent level from column 0 — in Markdown they draw through indented HTML. Reasonable in code, noisy in prose. | cap `14-split` README |
| L6 | Statusline | Seven groups: chip, branch, path, transient message, services badge, diag counts, `Spaces:4`, position. `" │ "` separators in `border` colour. The transient message and the LSP badge are both permanent; the message is the devcontainer notice at startup in an empty scratch. `Spaces:4` is shown always. | `statusline.rs:270-362`, cap `01-welcome` |
| L7 | Mode chip | `▌ NORMAL ` with `▌` in accent then bold base-on-accent text. In pickers the chip becomes ` FILES INSERT ` / ` SEARCH INSERT ` (13-14 cells); in prompts ` ? INPUT `. The chip is the only strong colour on screen — correct — but it changes width per mode, so the path jumps. | `statusline.rs:149-183` |
| L8 | Which-key | Bottom-centred rounded card, title ` space `, rows `  f   file finder` with a keycap on `select_bg`. 21 rows in **table order** (`e f o b / R ? y p P d k a s S j u c g w`), no grouping, `+1 more` at 80×24. `space w` shows nine identical rows `1…9 pane move / cycle / focus 1-9`. | `render/which_key.rs`, caps `03`, `41`, `51` |
| L9 | Help (`Space ?`) | A read-only buffer: `strop help — / searches · q closes`, `[normal] ─────…`, rows in six different section hues. Not the "centered floating card with aligned key tables" of 0001 §4. Order is table order (`h j k l`, `w b e`, `zz zt zb`, `H M L`, `ZZ`, `gv`, …). | `render/help.rs:13-22`, cap `06-help` |
| L10 | Pickers | Near-full-frame card over a dimmed backdrop; input `❯ status▏`; scope row `/home/tarek/workspace/strop · fuzzy · hidden on (session) · ignored off (session)`; result rows `▌STATUS.md … plans`; a *second* vertical rule pair `││` between list and preview (list scrollbar + preview border); preview header repeats the full absolute path; count `38 / 999` (999 is a cap presented as a total). At 80×24 the card starts at x=1 so the backdrop's `~` column shows to its left. | `render/picker/mod.rs`, caps `08`, `42` |
| L11 | Grep picker | Each hit is two rows: `[x] remote.md   /home/…/docs · 3 hits` then `22:45  …ot file identity. The modeline…`. The header repeats for every hit. `[x]` is an inclusion checkbox. The scope row repeats the root that is already in the title. | `render/picker/rows.rs:668`, caps `09`, `43` |
| L12 | Theme picker | Rows `▌✓ Dark  ■■■■`, swatches are four `■` in the theme's colours; selected row detail below. Good idea; the swatches are 4 cells wide out of a 115-cell row. | cap `13-theme-picker` |
| L13 | Command/search card | Top-centre, 50 % width, ` command ` / ` search ` title, `: th▏` with ex completion rows and `:s` inline diff. Good — the best surface in the editor. The `:s` card clips long diff lines at 50 % width. | `render/cmd_card.rs:59-61`, caps `10`, `19` |
| L14 | Completion | 58-cell card anchored at the word start, rows `var    new_end    lsp`; footer hint `tab s-tab cycle candidates with a live preview · ctrl…` clipped mid-chord. | `render/completion.rs:11-13`, cap `54` |
| L15 | Diagnostics picker | Rows show the **absolute path**, truncated with `…`, and not the message; the preview header repeats the path. | cap `57-diag-picker` |
| L16 | Welcome card | 58×10 rounded card, brand + tagline + three keycaps + `space o  remote & containers` + a one-line hint. Fine, except the remote line is an odd second priority and the statusline already shows a devcontainer message. | `render/mod.rs:302-384` |
| L17 | Terminal | Statusline: `▌ TERMINAL  │ local launch /home/tarek/workspace/strop/terminal #27 F2 inspect · ctrl-\ ctrl-n alt…` — session numbering started at 27 in a fresh session; the hint clips. | cap `21-terminal` |
| L18 | Git | Blame column `157b261 tknawara  1mo` left of the gutter (good, GitLens-like); log is a plain graph buffer; commit files view lists paths with `+7 -0` right-aligned. All three are undecorated buffers — consistent with "everything is a buffer" but visually indistinguishable from a file. | caps `32`, `64`, `65` |
| L19 | Glyph vocabulary | `▌ ❯ ▏ ▮ │ ─ ╭╮╰╯ ✓ ■ ▸ ▶ ● ▎ [x] · →`. Eleven marks for five concepts (selected, focused, current, applied, dirty). | grep in `render/` |

What is **good** and should be kept: one accent chip for mode; the command/search card with live `:s` diff and match counts; the preview spec footer (`delete, search /width, 579 bytes · exclusive`); the dimmed picker backdrop; the blame column; the `Space` leader structure; no Nerd Font dependency; cell-safe clipping everywhere.

### 3.2 Why it feels unfinished (diagnosis)

1. **No tonal ladder.** A terminal UI has exactly one tool for depth: background steps. Strop's themes define `base / surface / statusline_bg / cursor_row_bg / inactive_bg` but the deltas are 1.04–1.14:1 — invisible on most monitors. Helix and Zed use ≥1.25:1 per step; lazygit uses the border colour change instead. Strop uses neither consistently.
2. **One grey for three jobs.** `muted` paints comments, punctuation, line numbers, scope rows, hints. The eye cannot separate "code structure" from "annotation" from "chrome".
3. **Chrome is painted even when it is empty.** Scrollbar track, scope row, `Spaces:4`, services badge, repeated headers, `[x]`.
4. **Each widget was designed in isolation.** Nine hand-rolled `Block::default().borders(ALL).border_type(Rounded)` sites with different title styles (`text bold`, `secondary`, section-hued), different hint styles, different widths (58, 50 %, 40, 46, 30, 27).
5. **Mode colours are not design tokens**, so the palette author cannot reason about them.

### 3.3 Proposed design language (the contract)

Write this into `plans/0087-design-language.md` and make every renderer consume it through one module `render/style.rs` (tokens) and one `render/card.rs` (primitive). Ten rules:

1. **Three planes, one ladder.** `bg.base` (editor) < `bg.surface` (cards, previews, statusline) < `bg.raised` (selected row, keycaps). Each step ≥ 1.18:1 and ≤ 1.30:1 against the previous. The statusline sits on `surface`, not on its own slot.
2. **One accent, one job.** Accent = *where am I / what will happen*: mode chip (Normal), current line number, pending-operator footer, the caret of inputs, matched characters. Nothing decorative is accent. Mode colours are their own tokens (`mode.normal = accent`, `mode.insert`, `mode.visual`, `mode.replace`, `mode.pending`, `mode.terminal`).
3. **Borders are `fg.faint`, titles are `fg.text` bold, hints are `fg.muted`** — the same on every card. Rounded corners everywhere (already true). Depth is border colour: focused card `fg.muted` border, unfocused `fg.faint`. No "dimmed border column" (0001 §4) — it was never built and is not needed.
4. **Chrome only when it carries information.** Scrollbar: thumb only, no track, hidden when the content fits. Scope row: only when a scope differs from the default. `Spaces:4`: only when the buffer's detected indent differs from the configured default. Services badge: only `LSP!`-style 4-cell badge when a service is *failed*, blank when ready. Pane title row: only when there are ≥ 2 panes (already) — but then the single-pane layout must reserve nothing, so make the title row the *top border* of the pane (`╭ 1 statusline.rs ───`) instead of a separate row.
5. **One selected-row idiom.** Selected = full-row `bg.raised` + `▌` in accent at column 0. Applied/current (theme picker, buffer list) = `●` in accent after the label. Dirty = `●` in `warning`. Nothing else uses `▌`, `▸`, `▶`, `✓`, `[x]`.
6. **Keycaps are the only chip.** ` f ` on `bg.raised` in accent bold, fixed width per card, right-aligned keys, left-aligned descriptions. Help, which-key, welcome, picker hints and completion footers all use the same keycap renderer.
7. **Typography:** bold = headings and keycaps only; italic = comments and `(soon)`/unavailable rows; dim = never (unreliable across terminals); underline = links and the active search match only.
8. **Text hierarchy = four greys:** `fg.text` (code, filenames), `fg.secondary` (directory context, descriptions), `fg.muted` (comments, hints, line numbers), `fg.faint` (borders, indent guides, inactive panes). Comments are *muted italic*, punctuation is *secondary* — never the same colour.
9. **Widths come from content with three caps:** cards are `min(content, 60 % of columns)` and never narrower than 32; the picker card is `columns − 4` wide at ≥ 100 columns and full-bleed (no `~` margin) below.
10. **Hints are whole chords or nothing.** Already the picker rule (`0050 §8`); apply it to completion and terminal hints too.

### 3.4 Mockups (140 columns shown at 100 for brevity; default Hone palette)

**Editor + statusline** (single pane; no title row, no track, cursor line on `surface`, comments italic muted, current line number accent):

```
  94 }
  95
  96 impl Modeline {
  97     fn gather(editor: &Editor) -> Self {
▎ 98         let buf = editor.buf();                                                            ▮
  99         let (dir, name) = file_display(editor);
 100         let terminal_input = editor.terminal_input_active();
 NORMAL  main*  crates/strop/src/render/ statusline.rs ●                     ●1 ▲2   LSP!   98:9  17%
```

- Chip ` NORMAL ` base-on-accent, no leading `▌` (the chip *is* the mark). Fixed 8-cell width for every mode name (`NORMAL`, `INSERT`, `VISUAL`, `V-LINE`, `V-BLOCK`, `REPLACE`, `PENDING`, `TERM`, `FILES`, `SEARCH`) so the path never jumps.
- Branch `main` with `*` (dirty worktree) in muted; directory in secondary, filename in text; dirty `●` warning.
- Right side: diagnostics as `●1` error / `▲2` warning only when non-zero; `LSP!` 4-cell badge only on failure; position `98:9  17%`. No `Spaces:4` unless it deviates; no transient message slot — messages get their own row (next mockup).
- Separators are two spaces, not `│`. The row background is `bg.surface`.

**Messages and errors** — a one-row *echo line* above the statusline, present only while a message is live (≤ 4 s for info, until the next keypress for errors), in `warning`/`error` colour with a leading glyph. This restores vim's model and fixes the precedence bug (§5 U1):

```
 100         let terminal_input = editor.terminal_input_active();
 ✗ unsaved changes — :e! to force                                      Space a  actions
 NORMAL  main*  crates/strop/src/render/ statusline.rs ●                           98:9  17%
```

**Operator-pending preview** (`d/width` composing): the range on `bg.preview`, the spec footer replaces the right side of the statusline in accent:

```
 PENDING  main  statusline.rs                   delete · search /width · 579 bytes · exclusive
```

**Vertical split** (title in the pane's top border; focused pane border `fg.muted`, unfocused `fg.faint`; one divider column shared):

```
╭ 1 statusline.rs ● ───────────────────────────────╮╭ 2 README.md ───────────────────────────────╮
│  96 impl Modeline {                               ││   1 <div align="center">                    │
│  97     fn gather(editor: &Editor) -> Self {     ▮││   2   <img src="logo.svg" width="480" …     │
│  98         let buf = editor.buf();               ││   3   <h3>see the cut before you make it.   │
```
(Only the top border is drawn; sides are the divider column. One title idiom for terminals, directories and git surfaces too: `╭ 3 git log ──`, `╭ 2 terminal #1 · ~/workspace/strop ──`.)

**Picker (files)** — full-bleed card at < 100 columns, `columns−4` above; no scope row unless changed; one rule between list and preview; count honest (`38 of 2,311`); hints as keycaps:

```
╭ files ─────────────────────────────────────────────────────────────────── 38 of 2,311 ╮
│ ❯ status▏                                                                            │
│                                                                                      │
│ ▌ STATUS.md                      plans/          │ plans/STATUS.md                   │
│   status.rs    …engine/src/editor/lsp/           │  1  # Plan ledger                  │
│   status.rs    …engine/src/editor/picker/        │  2                                 │
│   tests.rs     crates/strop/src/render/statusline│  3  One line per plan state, …     │
│   services.rs  crates/strop/src/render/statusline│  4  standing (plans/0086 §5, …     │
│                                                  │  5  inside a plan that disagrees   │
╰  enter  open   ctrl-v  split   ctrl-o  collect   esc  close ──────────────────────────╯
```
Matched characters accent bold (keep); directory in `fg.secondary` *after* the name, left-aligned, elided from the left; no second `│`; preview header is the relative path only.

**Picker (grep)** — one row per hit, file header once per file group, no checkbox (excluded rows render struck-through muted instead):

```
╭ search  modeline ─────────────────────────────────────────────── 141 hits · 9 files ╮
│ ❯ modeline▏                                                                          │
│                                                                                      │
│ docs/remote.md                                     3 │ docs/remote.md:22              │
│ ▌  22  …ot file identity. The modeline identifies    │ 20  `:e!` refreshes the …      │
│    57  ignore files; the modeline names this fixed   │ 21  there. Shrink, …           │
│   130  …ctory retained in the modeline, pane title,  │▶22  size/mtime alone are …     │
│ docs/terminal-and-scripting.md                     2 │ 23  byte windows, whose …      │
│     6  …ns the endpoint in the modeline, pane title  │ 24                             │
╰  enter  open   ctrl-r  replace   ctrl-x  drop hit   ctrl-d  drop file   esc ─────────╯
```

**Help (`Space ?`)** — stays a real buffer (searchable, motions work) but laid out as cards: a two-column key table per section with the keycap renderer, sections in a fixed curated order (Moving · Editing · Visual · Search · Leader · Panes · Git · Terminal · Ex), keys in accent, descriptions in text, unavailable in muted italic. No per-section hue.

```
 strop 0.53.0 · help                                        / search   q close
 ─ Moving ─────────────────────────────────────────────────────────────────────
   h j k l      move                        w b e  W B E   word / WORD
   0 ^ $        line start / first / end    gg G           file start / end
   { }          paragraph                   % ]d [d        pair · diagnostic
 ─ Leader (space) ─────────────────────────────────────────────────────────────
   f  files     b  buffers    /  search    R  replace    s  symbols   g  git…
```

**Which-key** — grouped and ordered by the table's `sections` field, two columns when ≥ 10 rows:

```
╭ space ───────────────────────────────────────────────────────╮
│  f   files               s   symbols            g   git…     │
│  b   buffers             S   workspace symbols  w   panes…   │
│  /   search              d   diagnostics        o   remote   │
│  R   replace             a   actions            ?   help     │
│  j   jumps               k   hover              u   undo tree│
│  y p P  clipboard yank / paste after / before                │
╰──────────────────────────────────────────────────────────────╯
```

**Completion** — one row per candidate: kind icon-word in secondary, label in text with matched prefix accent, source as a 3-letter tag in muted; footer only when the menu is incomplete:

```
│   let q = new_e▏
│   ╭──────────────────────────────────────╮
│   │ ▌ var   new_end                 lsp  │
│   │   word  new_end                 buf  │
│   ╰ tab  next   enter  accept ───────────╯
```

**Diagnostics** — gutter sign `●`/`▲`/`ℹ` in severity colour; the cursor-line EOL note in muted italic; the picker rows are `severity  message` with the location in secondary after:

```
│ ▌ ● cannot find function `xyz_undefined` in this scope      editmap.rs:25  │
```

### 3.5 Implementation shape

- `render/style.rs`: `Tokens` struct built once per frame from the theme (not 58 atomic loads), passed by `&` to every renderer. Deletes the free functions `base()/text()/muted()/…` in `render/mod.rs:42-77`.
- `render/card.rs`: `Card { title, hints: &[Hint], focused }` → `(Block, inner)`; `Keycap(key)` span; `SelectedRow(area)` style. Replaces the nine ad-hoc blocks and the three hint-string builders.
- `render/echo.rs`: the message row (§3.4) — fed by `editor.message()` + `presented_feedback()` with explicit severity ordering.
- Statusline fixed-width chip table in `keymap`/`Mode` (`Mode::chip()` already exists; pad to 8).
- Theme struct becomes the token set in §4.1. The theme picker swatches should show `base/surface/text/accent/keyword/string` as 6 cells.

---

## 4. Themes and palettes

### 4.1 Inventory and verdicts

Eight compiled-in themes, `strop-core/src/theme.rs:116-208`, one `AtomicU8` live selector (`:229`), 28 colour slots + `ansi16`. Syntax has eight `class_*` slots; `Comment`→`muted`, `Constant`/`Heading`/`List`→`accent`, `Link`/`Tag`→`diag_info`, `Code`→`class_string`, `Variable`→`text` (`render/mod.rs:93-113`). No slot for: mode colours, hint/keycap, on-accent text, diff foregrounds (`+`/`-` text), variable/property/namespace, selection foreground, search-match, cursor colour, inactive-pane text.

Measured (`scratchpad/contrast.py`; WCAG ratio against `base`):

| Theme | text | muted | accent | sel bg | statusline bg | surface | weakest body ink | Verdict |
|---|---|---|---|---|---|---|---|---|
| **dark** (default) | 14.2 | 4.95 | 8.65 | **1.30** | **1.07** | **1.13** | muted 4.95 | hierarchy flat; keep identity (amber), rebuild as *Hone* |
| **light** | 12.6 | 4.07 | **4.11** | 1.24 | 1.17 | 1.08 | string 4.23, muted 4.07 | low-contrast, accent weak; replace with *Vellum* |
| **strop-ink** | 12.7 | 5.40 | 7.54 | 1.50 | 1.05 | 1.16 | fine | near-duplicate of dark with brass accent; fold into *Slate* |
| **catppuccin-mocha** | 11.3 | 4.44 | 8.07 | 1.52 | 1.07 | 1.30 | muted 4.44 | third-party brand + licence in the binary; delete |
| **catppuccin-latte** | **7.06** | 4.37 | 4.79 | 1.31 | 1.17 | 1.08 | **string 3.56, number 3.48, type 3.61, operator 3.59** (fail AA) | delete |
| **nord** | 10.8 | 4.04 | 6.24 | 1.45 | 1.24 | 1.24 | **diag_error 3.05**, number 4.41 | punctuation `#d8dee9` 9.25 is louder than keywords; delete |
| **paper** | 12.1 | 7.00 | 6.44 | 1.23 | 1.11 | 1.04 | fine | the owner's palette; keep, re-map onto new tokens, fix ladder |
| **paper-dark** | 13.9 | 7.10 | 9.13 | 1.67 | 1.04 | 1.11 | fine | keep, fix ladder |

Common defects across all eight **[V]**: selection 1.23–1.67:1 (Visual mode is barely visible — cap `16-visual.ansi` shows `#2a2c3a` on `#16161e`); `cursor_row_bg` is only used by list surfaces (`render/diff/list.rs:22`, `render/buffer/directory.rs:66,96`), never in file buffers; `surface` 1.04–1.30; `statusline_bg` ≤ 1.24; comments = punctuation = line numbers in six of eight themes.

### 4.2 Proposed token set (replace `Theme`)

```
bg:    base surface raised statusline(=surface) cursorline selection preview flash pair
       diff_add diff_del diff_add_strong diff_del_strong
fg:    text secondary muted faint guide on_accent
accent
mode:  normal(=accent) insert visual replace pending terminal
diag:  error warning info hint
diff:  add del change (foregrounds for +/- text and gutter signs)
syn:   keyword function type string number constant attribute operator punctuation
       comment variable property namespace heading link code
ansi16
```
Rules for every palette: `text ≥ 10:1`, `secondary ≥ 7:1`, `muted ≥ 4.5:1`, `comment ≥ 4.5:1`, every `syn.*` ≥ 4.5:1 on `base` *and* `surface`; `selection ≥ 1.45:1` (light) / `≥ 1.55:1` (dark) vs base; `raised ≥ 1.18:1`; `on_accent/accent ≥ 4.5:1`; `comment` and `punctuation` differ by ≥ 1.2:1 or by italic. A `#[test]` in `strop-core/src/theme/tests.rs` should assert these for every palette (it currently asserts a subset).

### 4.3 Four original palettes (all values computed; ratios vs `base` / `surface` / `selection`)

**HONE — dark default** (strop's amber identity kept; cool graphite base; selection finally visible)

| token | hex | base | surface | selection |
|---|---|---|---|---|
| bg.base | `#15171c` | — | — | — |
| bg.surface | `#1c1f25` | 1.09 | | |
| bg.raised | `#23272f` | 1.20 | | |
| bg.cursorline | `#1b1e24` | 1.07 | | |
| bg.selection | `#30405a` | **1.71** | | |
| bg.preview | `#4a3617` | 1.56 | | |
| bg.flash | `#6e4c1a` | 2.32 | | |
| bg.pair | `#334155` | 1.73 | | |
| bg.diff_add / del | `#1e2d22` / `#331f24` | 1.24 / 1.16 | | |
| bg.diff_add_strong / del_strong | `#2b4a33` / `#5a2c35` | | | |
| fg.text | `#e6e1d6` | 13.75 | 12.66 | 8.81 |
| fg.secondary | `#a8adb8` | 7.97 | 7.34 | 5.10 |
| fg.muted | `#7b8290` | 4.64 | 4.27 | 2.71 |
| fg.faint (borders) | `#4a5160` | 2.25 | | |
| fg.guide | `#2b2f38` | 1.34 | | |
| accent / on_accent | `#e8a848` / `#15171c` | 8.65 | 7.96 | 5.54 |
| mode.insert | `#8fc48a` | 8.91 | | |
| mode.visual | `#b79ce6` | 7.61 | | |
| mode.replace | `#e07a7a` | 6.18 | | |
| mode.terminal | `#6fb8c4` | 7.97 | | |
| diag.error / warning / info / hint | `#e8707c` / `#d9a85a` / `#7fb2d4` / `#7b8290` | 6.01 / 8.28 / 7.87 / 4.64 | | |
| diff.add / del | `#8fc48a` / `#e8707c` | | | |
| syn.keyword | `#c99ce0` | 7.95 | 7.32 | 5.09 |
| syn.function | `#86b7d8` | 8.35 | 7.69 | 5.35 |
| syn.type | `#8ecdb8` | 9.88 | 9.10 | 6.33 |
| syn.string | `#aec37f` | 9.32 | 8.58 | 5.97 |
| syn.number | `#e39a7b` | 7.83 | 7.21 | 5.02 |
| syn.constant | `#e8a848` (accent) | 8.65 | | |
| syn.attribute | `#d1aa62` | 8.23 | | |
| syn.operator | `#b4b8c2` | 9.03 | | |
| syn.punctuation | `#9da3af` | 7.08 | 6.52 | 4.13 |
| syn.comment (italic) | `#79808d` | 4.51 | 4.15 | 2.63 |
| syn.variable / property | `#e6e1d6` / `#d7cfc0` | | | |

ansi16: `#101217 #e8707c #8fc48a #d9a85a #7fb2d4 #c99ce0 #8ecdb8 #cfcac0 #4a5160 #f08a94 #a6d6a0 #e8a848 #9ccbe8 #d9b2f0 #a8e0d0 #e6e1d6`.

**VELLUM — light / paper** (warm paper, graphite ink, burnt-umber accent; no green cast, so distinct from Paper)

| token | hex | base | surface | selection |
|---|---|---|---|---|
| bg.base / surface / raised | `#f6f2e9` / `#fbf8f1` / `#ece7dc` | — / 1.05 / 1.10 | | |
| bg.cursorline | `#efeadf` | 1.07 | | |
| bg.selection | `#d3cbb6` | **1.45** | | |
| bg.preview / flash / pair | `#f0dfb8` / `#e9c98a` / `#d4cdbd` | 1.18 / 1.43 / 1.42 | | |
| bg.diff_add / del | `#dfeadb` / `#f3dcd8`; strong `#c3dbbb` / `#e8bfba` | | | |
| fg.text | `#262a30` | 12.91 | 13.60 | 9.57 |
| fg.secondary | `#4b525c` | 7.06 | 7.44 | 5.24 |
| fg.muted | `#656d7b` | 4.67 | 4.92 | 3.23 |
| fg.faint / guide | `#aeb3ba` / `#d8d3c8` | 1.89 / 1.34 | | |
| accent / on_accent | `#9a5a12` / `#f6f2e9` | 4.89 | 5.15 | 3.62 |
| mode.insert / visual / replace / terminal | `#3f7a3a` / `#6a3faa` / `#b03a3a` / `#1f6f7a` | 4.63 / 6.47 / 5.35 / 5.20 | | |
| diag.error / warning / info | `#b8323f` / `#8a5d13` / `#2a6a94` | 5.27 / 5.14 / 5.24 | | |
| syn.keyword | `#6f3aa8` | 6.60 | 6.96 | 4.89 |
| syn.function | `#235f8c` | 6.09 | 6.41 | 4.51 |
| syn.type | `#1f6e5c` | 5.46 | 5.75 | 4.05 |
| syn.string | `#4e6f22` | 5.19 | 5.47 | 3.85 |
| syn.number | `#9c4a22` | 5.50 | | |
| syn.constant / attribute | `#9a5a12` / `#7a5a14` | 4.89 / 5.70 | | |
| syn.operator / punctuation | `#4b525c` / `#656d7b` | 7.06 / 4.67 | | |
| syn.comment (italic) | `#6b6f79` | 4.50 | 4.74 | 3.11 |

Note: on the light palette comments and punctuation are the same luminance class by necessity; italic carries the distinction (rule 8).

**CARBIDE — high contrast dark** (for projectors, bright rooms, low-vision; every ink ≥ 7:1, selection 2.17:1)

| token | hex | base |
|---|---|---|
| bg.base / surface / raised / statusline | `#0b0c0f` / `#14161b` / `#1d2027` / `#000000` | — / 1.08 / 1.20 / 1.07 |
| bg.selection / preview / flash / pair | `#2f4a6e` / `#5a3d0c` / `#8a5e14` / `#3b4a63` | **2.17** / 1.96 / 3.44 / 2.18 |
| bg.diff_add / del | `#143d1f` / `#4a1a22` | 1.60 / 1.36 |
| fg.text / secondary / muted / faint / guide | `#f4f2ec` / `#cfd3da` / `#a3a9b4` / `#6a7280` / `#3a3f48` | 17.47 / 13.02 / 8.28 / 4.03 / 1.85 |
| accent / on_accent | `#ffb74a` / `#0b0c0f` | 11.29 |
| mode.insert / visual / replace / terminal | `#9be08f` / `#cfaaff` / `#ff8a8a` / `#7fd6e4` | 12.53 / 10.10 / 8.62 / 11.77 |
| diag.error / warning / info | `#ff7b86` / `#f0c060` / `#8fc8ff` | 7.85 / 11.56 / 11.06 |
| syn.keyword / function / type / string / number | `#dcb0ff` / `#8fc8ff` / `#8fe3cf` / `#c3e088` / `#ffa98c` | 10.89 / 11.06 / 13.09 / 13.35 / 10.53 |
| syn.constant / attribute / operator / punctuation / comment | `#ffb74a` / `#f0c060` / `#d8dbe2` / `#aeb4bf` / `#9aa1ad` | 11.29 / 11.56 / 14.11 / 9.39 / 7.52 |

**SLATE — low-chroma dark** (near-monochrome; one brass accent; syntax by luminance, so it reads calmly in long sessions — this is where Strop Ink's intent lands)

| token | hex | base |
|---|---|---|
| bg.base / surface / raised / cursorline | `#1a1c20` / `#202329` / `#272b32` / `#1f2227` | — / 1.08 / 1.20 / 1.07 |
| bg.selection / preview / flash / pair | `#343a45` / `#3d3a30` / `#54503f` / `#3a4150` | 1.49 / 1.50 / 2.11 / 1.67 |
| bg.diff_add / del | `#22302a` / `#34262a` | 1.24 / 1.18 |
| fg.text / secondary / muted / faint / guide | `#dcd9d2` / `#aeb1b7` / `#858a93` / `#545962` / `#2d3139` | 12.10 / 7.94 / 4.92 / 2.42 / 1.31 |
| accent / on_accent | `#c9b27e` / `#1a1c20` | 8.25 |
| mode.insert / visual / replace / terminal | `#a5bd9a` / `#b8aacb` / `#cf8f8f` / `#93b7bd` | 8.40 / 7.84 / 6.48 / 7.92 |
| diag.error / warning / info | `#d98a90` / `#cfb27a` / `#92b2c8` | 6.49 / 8.37 / 7.66 |
| syn.keyword / function / type / string / number | `#c9b27e` / `#b9c6d2` / `#b6c9c0` / `#b9c3a6` / `#cdb6a3` | 8.25 / 9.81 / 9.83 / 9.28 / 8.79 |
| syn.constant / attribute / operator / punctuation / comment | `#c9b27e` / `#c6bb9c` / `#aeb1b7` / `#8d929b` / `#7c8290` | 8.25 / 8.93 / 7.94 / 5.46 / 4.43 |

(Slate's comment is 4.43 — raise to `#7f8593` (≈4.6) if the AA rule is enforced strictly; selection 1.49 is at the floor for a dark theme and could go to `#373e4b`.)

Plus **Paper** / **Paper Dark** re-mapped onto the token set with: `bg.raised` added (`#e4e9df` / `#243229`), `bg.selection` raised to ≥ 1.45 / ≥ 1.6 (`#cfdcc8` / `#2f4b3a`), `syn.comment` italic, mode tokens (`insert #3b7a4a`, `visual #6d5a8c`, `replace #9c3f38`, `terminal #25677a` for Paper).

Result: six palettes — Hone (default), Slate, Carbide, Vellum, Paper, Paper Dark — all original, `LICENSE-CATPPUCCIN`/`LICENSE-NORD` and the `licenses` subcommand deleted.

### 4.4 Degraded terminals

`render/mod.rs:36-40` emits `Color::Rgb` unconditionally; there is no 256/16-colour path and no `COLORTERM`/`TERM` probe **[V]** (`fade_mix` has a non-RGB branch that can never be hit). On a 256-colour terminal most emulators quantise RGB themselves; on 16-colour terminals the output is undefined. Proposal: at startup, if `COLORTERM ∉ {truecolor,24bit}` and `TERM` lacks `direct`/`truecolor`, map every token to the nearest xterm-256 index (`theme.rs` already has the cube/grey ramp in `ansi256()`); if `TERM` is `*-16color`/`vt100`/`linux`, use the theme's `ansi16` with a fixed role table (`text=15, muted=8, accent=11, error=9, …`). Keep bold/italic as the hierarchy carrier so a 16-colour fallback still reads.

---

## 5. UX findings

| # | Finding | Evidence | Fix |
|---|---|---|---|
| U1 | **Ex errors are hidden by persistent info feedback.** `:e README.md` on a dirty buffer did nothing visible; the statusline kept "Dev container discovery completed · Space a actions". | `transient.rs:55-63` (feedback → message order); `normal/ex.rs:364`; cap `62-e-dirty` **[V]** | Severity-ordered echo row (§3.4); info feedback auto-expires after 4 s or first keypress; the devcontainer notice should not fire at all when zero configurations were discovered (`devcontainers/ui.rs:140`). |
| U2 | **`:e` relative paths resolve against a directory buffer's path.** From `crates/strop/src/render/` buffer, `:e crates/strop/src/render/statusline.rs` opened `…/render/crates/strop/src/render/statusline.rs` as a new empty file, no error. | cap `17-pending-d` statusline, `40-80x24-file` **[V]** | Resolve ex paths against `editor.cwd()` (vim semantics) unless prefixed `./`; show "new file" in the echo row when the path does not exist. |
| U3 | **Permanent failure badge on first run.** Without a working `rust-analyzer`, every screen for the whole session shows `lsp rust-analyzer@strop failed` (30 cells). | caps `02`–`43` **[V]** | 4-cell `LSP!` badge (0080 D intended this; the long form still wins at 140 columns); `:explain` already holds the detail. |
| U4 | **Esc in `/` and `:` does not cancel** — it enters "field NORMAL"; a second Esc cancels. Documented in README, but it is the single most-pressed key in vim's cmdline and this is a fidelity deviation the differential harness cannot see (it drives `:normal!`). | cap `05b-after-help`; README inspection table **[V]** | Make Esc cancel (vim); offer field-Normal on `ctrl-o` (vim's own "one normal command from insert" idiom) or `ctrl-f` (vim's cmdline window). |
| U5 | **`?` is search-backward, help is `Space ?`**, but the welcome card says `? keybindings`. | `render/mod.rs:350-357` vs `keymap` **[V]** | Welcome card: `space ?`. |
| U6 | **Picker count shows a cap as a total** (`38 / 999`, `999` at 80×24 with no query). | caps `08`, `42` **[V]** | `38 of 2,311` or `38 of 999+`. |
| U7 | **Grep rows duplicate headers; `[x]` checkbox vocabulary.** | §3.1 L11 | Group hits under one header; struck-through muted for excluded. |
| U8 | **Diagnostics picker shows the path, not the message.** | cap `57` **[V]** | `severity  message   file:line`. |
| U9 | **Which-key cards unsorted; `space w` lists nine identical rows for `1-9`.** | caps `03`, `51` **[V]** | Group by `sections`; collapse ranges (`1-9  focus pane`) the way `<a>`→`a-z` is already collapsed. |
| U10 | **Help is a table dump**, six colours, table order; `?` help contract is satisfied but not designed. | cap `06` **[V]** | §3.4 layout; curated section order in `keymap::SECTIONS` (`keymap/mod.rs:84`). |
| U11 | **No cursor line in file buffers**; Visual selection at 1.30:1. | §4.1 **[V]** | tokens + ladder. |
| U12 | **Terminal session numbering** starts at 27 in a fresh process (`terminal #27`). | cap `21` **[V]** | Per-session counter from 1 (cosmetic, but looks broken). |
| U13 | **Startup message in an empty scratch**: "Dev container discovery completed · Space a actions" on a repo with zero devcontainer configs. | cap `01` **[V]** | Only report discovery when something was found or failed. |
| U14 | **Config surface:** TOML at `$XDG_CONFIG_HOME/strop/config.toml`, 11 knobs, **no keymap customisation at all**, no project layer (0084 is untracked). | `strop-engine/src/config.rs` **[V]** | A `[keys]` table that remaps leaf bindings by `id` (the table already has stable `id`s) is the smallest honest step; document that operators/motions are not remappable by design. |
| U15 | **First run has no tutor** (0001 M4 "strop tutor"). The welcome card is the whole onboarding. | **[V]** | `:tutor` as a real buffer with the preview as the feedback loop — it tapes into the demo for free, as 0001 said. |
| U16 | Hints clip mid-chord in completion (`ctrl…`) and terminal (`alt…`). | caps `54`, `21` **[V]** | Whole-chord rule (§3.3 #10) everywhere. |
| U17 | `Space s` (symbols), `Space u` (undo tree), `Space g l` on a dirty buffer, `Space k` on a plain identifier: nothing opens **and nothing says why**. | caps `34`, `37`, `60`, `56` **[V]** | Every leader verb that declines must say so in the echo row ("no symbols: LSP not attached", "undo history is empty", "unsaved changes — :w first"). |

Positive: the `:s` live diff, the search card's match count, `d/pat` spec footer, `:explain`, the theme picker's applied mark, the blame column, and cell-safe clipping are all better than the Helix/Neovim baselines.

---

## 6. Architecture and code quality (new since 0085, plus follow-ups)

**CRITICAL**

- **A1. Render functions past the complexity ceiling.** `render/buffer/rows.rs:8 render_pane` 511 lines; `render/picker/mod.rs:40 render_picker` 436; `render/cmd_card.rs:15` ~275; `grammar/resolve/mod.rs:50 resolve_controlled` 506 (the pure resolver the preview doctrine rests on); `editor/explain.rs:18` 408; `strop-fs/src/local/execute.rs:59` ~404; `cli.rs:60 parse` ~345. Files pass the 800-line rule by packing responsibility into one function. **[V]** Fix: `render_pane` → `rows::{gutter, content, overlays, decorations}`; `render_picker` → `card`, `input`, `list`, `preview`, `hints` (the §3.5 primitives force this); `resolve_controlled` → one function per `Motion` family with a dispatch table.

**HIGH**

- **A2. Per-grapheme allocation in the hot render path.** `render/buffer/content.rs:165,206,254` build an owned `Span` per grapheme (`.to_string()` / `" ".repeat(w)`), `:237` collects spans into a `String` and re-segments; `rows.rs:374,376` `format!` per row; statusline rebuilds ~15 `String`s per frame (`statusline.rs:137-199,435-538`); `picker/mod.rs:28-38 dim_backdrop` walks every cell per frame. 253 allocation sites in non-test render code. AGENTS.md "no avoidable allocation on input→render" is not met. **[V]** Fix: emit runs (same style, contiguous graphemes) as `Cow<'a, str>` spans borrowing the rope chunk; cache the statusline model and re-gather only on epoch change; dim via a precomputed per-theme dimmed `Tokens` instead of per-cell mixing.
- **A3. Process-global mutable theme.** `theme.rs:229 static CURRENT`, 58 `theme::current()` reads per frame in `render/`, test isolation by re-exec (`theme/tests.rs:174-193`, `config.rs:494-505`, `explain.rs:726`; 13 `current_exe()` sites). **[V]** Fix: `ThemeId` on `Editor` (it already is in config), `Tokens` built per frame; `strop-core::theme` becomes data only. Deletes the re-exec tests.
- **A4. 0086 F2 regressing while scheduled.** `Editor` +3 fields; the four private channels remain (`editor/mod.rs:120-178`). **[V]** Keep the schedule but add a ratchet test now: `editor/mod.rs` field count must not increase (a `const` the PR must lower), same for `impl Editor` block count.
- **A5. Three path-display projections, two public `display_path`s with different contracts.** `render/statusline.rs:435-538 file_display`, `editor/document/mod.rs:48-70 Document::label`, `strop-workspace/src/directory.rs:45 display_path` (escaping) vs `strop-picker/src/source/selection.rs:12 display_path` (separator normalisation). Four clip helpers (`render/clip.rs:23,45`, `picker/rows.rs:443 elide_middle`, `statusline/layout.rs:26,194`, `strop-core/src/layout.rs:170`). **[V]** Fix: one `DisplayPath { dir, name, authority }` projection in `strop-workspace`, one `clip` module.
- **A6. No card/keycap primitive.** Nine `BorderType::Rounded` blocks in seven files, different title/hint styles, magic widths (`58u16,10u16` `mod.rs:312`; `MENU_WIDTH 58 / DOC_WIDTH 40` `completion.rs:11-13`; `.max(46)` `blame_card.rs:23`; `.max(30)` `cmd_card.rs:61`; `.max(40)` `hover_card.rs:23`; `.min(27)` `completion.rs:165`). **[V]** Fix: §3.5.

**MEDIUM**

- **A7. Ceiling violations without a 0028 entry:** `keymap/bindings_adapter.rs` 1234, `editor/picker/tests.rs` 1022, `editor/filesystem/tests.rs` 822, `strop-lsp/src/client/api.rs` 806; 0028:91 still says "the 972-line keymap". **[V]** Split the keymap by section (`keymap/{normal,visual,leader,git,ex,terminal}.rs`) — it is the file the `?` contract depends on and it is read by humans.
- **A8. Mega-tests:** `strop-grammar/src/tests.rs:127 doubled_operator_is_linewise` 552 lines; `strop-trace/src/tests.rs:301` 470; `picker/source/grep/e2e.rs:161` 488. One assertion failure hides the rest. **[V]**
- **A9. Silent SSH skips on host:** nine `return;` sites in `strop/tests/remote_ssh*.rs` gated on `STROP_REQUIRE_SSH_TESTS`; `STROP_REQUIRE_NVIM` does not exist though 0086 F3 names it. **[V]** Print a skip notice; add the env knob and set it in the Dockerfile.
- **A10. No `rust-version`, no `[workspace.lints]`** in `Cargo.toml:1-12`; lints are therefore per-crate and unenforced. **[V]**
- **A11. `strop-core` still does IO** (`buffer/io.rs:203 NamedTempFile`, `buffer.rs:178,203`) and depends on `strop-trace`; `tempfile` is a runtime dep. **[V]** (F2, acknowledged — listed so the scorecard is complete.)
- **A12. Unwrap debt is real but small:** ~45 sites; the eight `docs.get(id).unwrap()` in `collections/projection.rs:44-139` and the four `cmd.op.unwrap()` in `normal/execute.rs` are the ones worth typing. **[V]**

**LOW**

- **A13.** `#[allow(dead_code)]` in production `strop-worker-deploy/src/container.rs:99`; `#[allow(deprecated)]` `strop-lsp/src/client/spawn.rs:362`. **[V]**
- **A14.** `fade_mix` non-RGB branch unreachable; `cursor_fade` is the only animation in a "zero animation" editor (0001 §4) — consider deleting the feature and its config knob. **[V]**
- **A15.** `plans/0084` untracked; `proofs/__pycache__` on disk (ignored). **[V]**

Doctrine checks that **hold** **[V]**: 0 `.await` in strop-engine; blocking `recv()` only on dedicated threads; no process spawn on the input path; TLS rustls-only; preview and execute share `resolve_controlled`.

---

## 7. Proof ring (ring 0)

### 7.1 Verdict

The pilot is honest. **[V]**
- Kernel: `editmap.rs` (`map_position`, `validate_batch`), `frame.rs` (three admissions), `generation.rs` (two). `no_std`, `forbid(unsafe_code)`, zero deps, explicit loops.
- Production calls it: `strop-core/src/buffer/mutation.rs:6,164` (`validate_batch` before reverse application), `strop-engine/src/editor/{transact.rs:198, snippet.rs:208, collections/view_positions.rs:140, completion/acceptance.rs:370}`, `strop-worker-protocol/src/frame.rs:14-16`, `strop-core/src/id.rs:13`.
- Theorems are stated over the Aeneas translation (`Extracted/StropKernel.lean`, drift-checked by `gate.sh`); `Audit.lean` requires 11 named claims and axioms ⊆ {propext, Classical.choice, Quot.sound}; `grep sorry|axiom|admit` in `proofs/Kernel` → none.
- Mutants: 9, each mutates Rust, must fail exactly its theorem. Good calibration design.
- Meaningful content: `validate_batch_spec` (accept ⇔ `BatchOk`, each refusal has a witness) and `batch_map_composition` are real theorems; `header_admitted_spec`, `body_admitted_spec`, `next_generation_spec`, `index_for_len_spec` restate one-line functions (still worth having as drift guards, but they are not where value lies).

### 7.2 Gaps

- **P1. The composition theorem's subject is hand-written Lean, not extracted code.** `EditMap.lean:255 batchMap := es.foldr (mapNat …)` models "production's fold" by comment; the real fold is in Rust outside the kernel (`transact.rs:188-201` maps one change at a time as changes are journaled; `view_positions.rs:140`; `snippet.rs:208`). Principle 2 of 0086 ("no hand-written Lean copy of the Rust") is bent here, and **no mutant can target the fold** (mutating `map_position` breaks `map_position_spec`, not composition). Fix: move `pub fn map_through_batch(position, &[(start, old_end, new_end)]) -> usize` (reverse loop over the slice) into the kernel, call it from the three sites, restate `batch_map_composition` over the extracted function, add a mutant that reverses the loop direction.
- **P2. `validate_batch` is called after a `std` sort** (`mutation.rs:155`); `Unsorted` is a "caller defect". The sort is trusted. Either prove a kernel insertion sort (0086 §9 removed it as dead weight — fair) or add a `debug_assert!(pairs.windows(2).all(…))` and keep the typed refusal.
- **P3. The gate does not run in CI** (image ~20 GB). Acceptable per 0086 §9, but then `release-local.sh qualify` is the only place it runs, and nothing in `ci.yml` checks that `proofs/Extracted` matches the kernel. A cheap CI step can run Charon+Aeneas only (no mathlib) to catch drift on every push, and leave `lake build` to local qualification. **[S]** on image feasibility.
- **P4. `lean-toolchain` v4.35.0 and the aeneas path `/opt/aeneas`** are pinned in the image; `lake-manifest.json` pins mathlib. Good. The kernel's `[package.metadata.charon] start_from` lists modules by hand — a new module that is forgotten is silently unproved; have `gate.sh` assert every `pub mod` in `lib.rs` appears in `start_from`.

### 7.3 Next decisions to move inward (ranked by user-visible value)

Each item names the Rust that moves, the theorem(s) in Lean-ish form, the mutant, and the differential coverage that lands with it (0086 §3 rule). Aeneas constraints that shape the API: inputs are slices/`usize`/`u32`, no closures or iterator adaptors, no `&str` methods (work on `&[u8]`), no recursion needed (loops translate), return `Option`/`Result` rather than panicking, no floats, no `dyn`.

**K1. Batch mapping (closes P1).** `map_through_batch(p, edits: &[(usize,usize,usize)]) -> usize`.
```
theorem map_through_batch_spec (p) (es : Slice (Usize×Usize×Usize)) (h : Chain3 es) :
  map_through_batch p es ⦃ r => (r : Int) = directMap p es ⦄
theorem map_through_batch_monotone : a ≤ b → map_through_batch a es ≤ map_through_batch b es
```
Mutant: iterate forward instead of backward. Oracle: none needed (pure algebra); property test with random batches already exists in `transaction_conformance`.

**K2. Range → operated byte span** (the "linewise ⇒ whole-line bounds" theorem 0086 F3 promised; today in `strop-core/src/range.rs` + `grammar/resolve/mod.rs`). Move `MotionShape`, `Range { start, end, shape, inclusive }` and
`operated_span(range, line_starts: &[usize], len: usize, next_grapheme_end: fn? → pass as precomputed `end_after: usize`) -> (usize, usize)` into the kernel. Line starts come in as a sorted slice (the rope can produce it for the visible window; for whole-file ops pass the two relevant line starts only: `line_start_of(start)`, `line_end_of(end)` — so the API is `operated_span(start, end, inclusive, linewise, line_start, line_end_incl_newline, inclusive_end)`).
```
theorem operated_span_linewise : linewise → r.1 = line_start ∧ r.2 = line_end_incl_newline
theorem operated_span_charwise_exclusive : ¬linewise ∧ ¬inclusive → r = (min s e, max s e)
theorem operated_span_charwise_inclusive : ¬linewise ∧ inclusive → r = (min s e, inclusive_end)   -- inclusive_end = grapheme end after max
theorem operated_span_contains_cursor : r.1 ≤ cursor_or_start ∧ … ≤ r.2  ∨ r.1 = r.2
theorem operated_span_in_bounds : r.2 ≤ len
```
Mutants: drop the `+ newline` on linewise; swap min/max; use byte+1 instead of `inclusive_end`. Oracle: differential cases `d$ dj dk d} dG` + `v$d`, `Vjd`, `dl` on a multibyte grapheme — the preview footer's "`inclusive, 14 bytes`" is exactly this function's output, so `preview() == execute` for every case is the second half.

**K3. Count arithmetic and operator-pending state** (`grammar/parse.rs:14 parse`, 309 lines). Move the pure parts: `combine_counts(outer: Option<u32>, inner: Option<u32>) -> Option<u32>` (saturating multiply, vim: `2d3w` = 6), `apply_count_lines(line, count, total) -> usize` (saturate at last line), and the operator-pending step machine `step(state, key_class) -> Next` over a small `KeyClass` enum (digit, operator, motion, object-prefix, char, esc).
```
theorem combine_counts_mul : combine_counts (some a) (some b) = some (min (a*b) u32.max)
theorem combine_counts_id  : combine_counts none b = b ∧ combine_counts a none = a
theorem step_doubled_operator_is_linewise : step (Pending op) (Operator op) = Complete (Linewise op)
theorem step_esc_resets : step s Esc = Idle
theorem step_count_never_leads_zero : (state after digits) has count ≥ 1 or none
```
Mutants: non-saturating multiply; `0` accepted as a leading count. Oracle: differential `2d3w`, `d2d`, `3dd`, `d0`, `10dd` near EOF, `c3l` at line end — many already in the 68 cases; grow to the `Op × Motion` lattice.

**K4. Word / WORD scanners over `&[u8]`** (`grammar/resolve/motions.rs`). `char_class(b: u8) -> Class {Space, Word, Punct}` on ASCII with a precomputed class for the first byte of a non-ASCII grapheme (treat ≥0x80 as Word, vim's `iskeyword` default for identifiers is close enough for the kernel; the engine refines), and `word_forward(text: &[u8], pos, big: bool) -> usize`, `word_end`, `word_backward`.
```
theorem word_forward_progress : pos < len → word_forward t pos big > pos
theorem word_forward_lands_on_class_boundary : r = len ∨ class t[r] ≠ class t[r-1] ∨ (¬big ∧ …)
theorem word_backward_inverse_on_boundaries : is_word_start t p → word_forward t (word_backward t p) = p
theorem word_forward_bounded : r ≤ len
```
Mutants: skip the punctuation class (`w` behaves like `W`); off-by-one at EOL. Oracle: the harness is missing `h/k/F/T/?/W/B/E/ge/gE/{/_` (0085 H4) — add them in the same PR.

**K5 (later). Viewport scroll math** (`prepare/geometry.rs`, `scrolloff`, `zz/zt/zb`, `ctrl-d/u` half-page with count): `scroll_to_reveal(top, cursor_row, height, scrolloff) -> top'` with `theorem reveal_keeps_cursor_visible : top' ≤ row ∧ row < top' + height` and `theorem reveal_is_idempotent`. Cheap, visible every keystroke, and the kind of thing that regresses silently.

**K6 (later). Selection normalisation** (`strop-core/src/selection.rs:107 normalize`): sort + merge overlapping extra selections; `theorem normalize_sorted_disjoint`, `theorem normalize_preserves_primary`. Pairs with multicursor differential cases (nvim has no multicursor — so property tests, not the oracle).

**Not worth moving:** undo/redo invertibility (the undo tree is a data structure with allocation and IDs — property-test it with `transaction_conformance`); UTF-8/grapheme column math (depends on `unicode-width` tables; the kernel cannot carry them — test against the crate instead).

---

## 8. Website (strop.dev)

Findings (delegated review, spot-checked **[V]** unless marked):

- **Honesty.** index.html:209 and docs.html:63-64 say Paper is *not* an editor theme — it has shipped since 0.47.3 (`theme.rs:177-178`). docs.html:315 and roadmap.html:239-243 describe GUI work as "deferred" — it was deleted in 0.53. roadmap.html:70-84 "Next" names 0057 (retired), the native worker (shipped 0.36), completion (shipped), debugger under 0019 (superseded); "Shipped" stops at 0.32.4 (roadmap.html:88) — 21 releases missing. "23 languages" (index.html:37,113; docs.html:318) vs 26 in `strop-syntax/src/languages.rs:7-38`. **The demo tape sends `Ctrl+w w`** (`demos/demo.tape:147-152`) which `panes.rs:295` asserts is unbound — the GIF records dead keystrokes. The hero promise "Preview your next change … before you commit to it" exceeds the README's own inspection table (`ci[`/`dw` execute at the completing key).
- **IA.** Four flat pages; docs is one 400-line page titled "Configuring strop" that covers SSH, containers, terminals, scripting and keys; the TOC omits two H2s. The product is invisible above the fold: a stylised SVG editor with hard-coded Catppuccin traffic-light hexes (index.html:50-52) sits in the hero; the real recording is a 7.5 MB, 1400×760, ~2,650-frame GIF behind a closed `<details>` with no poster, no dimensions and no pause control.
- **Visual.** System fonts, hairline rules, 14 user-selectable palettes (default: green-on-cream Paper) — nothing on the first screen resembles the editor's default look; no `prefers-color-scheme`. Typography is good (clamp'd H1, 1.7 line-height); 10–11 px metadata text is too small. Contrast is AAA on the default palette (text 12.1:1).
- **Performance.** ~70 KB without the GIF; Prism (26 KB) loaded on index to colour one `curl` line; version chip is client-side via the GitHub API (no-JS visitors see "Latest release").
- **Build.** `pages.yml` path filter excludes `docs.html`, `roadmap.html`, `changelog.html`, `install.sh` — a docs-only commit does not deploy. `changelog.html` is generated manually by `build_changelog.py`; no check that it matches `CHANGELOG.md`. The local clone of the site repo is two commits behind origin.

### 8.1 Redesign proposal

**Sitemap:** `/` (one screen of product, then install, then three honest feature blocks, then "how it is checked"), `/docs/` (split by the editor repo's `docs/*.md`: editor, search, filesystem, remote, terminal-and-scripting, cli, vim-compat — generated from those files by a `build.py`, so the site can never diverge from the repo), `/changelog/` (generated), `/roadmap/` → **delete**; replace with a short "Status" section on `/` that renders `plans/STATUS.md`'s in-force table.

**Hero:** the real editor. A static PNG/SVG rendered from an actual `TestBackend` frame (the repo can emit the cell grid; a small script turns it into an SVG with the Hone palette) showing `d/width` composing with the preview range and the spec footer — the one thing no other editor has. Below it: `curl -fsSL https://strop.dev/install.sh | sh`, `Linux/WSL x86_64 · single static binary · v0.53.0` stamped at build time from the catalog. One CTA. The GIF becomes an `<video>` (WebM ~1 MB from the same tape) with poster and controls, in a "60-second tour" section.

**Sections:** (1) The preview — `d/pat`, `:s`, Visual: three frames, honest captions copied from the README inspection table. (2) One query language — files/grep/symbols pickers. (3) Git in the buffer — blame column, log, hunks. (4) Everything is a buffer — directory, terminal, `:explain`. (5) How it is checked — the Lean claim list (`AXIOMS …` lines from `Audit.lean`, generated), the nvim differential harness, the gates; link to 0086. (6) Install + update + supported platforms, with the retired channels sentence.

**Visual style:** the site uses the editor's token set literally — `Hone` as default (`prefers-color-scheme: dark`), `Vellum` for light; the palette switcher offers exactly the editor's six themes and nothing else, so "what you see is what you install". Keep system fonts; add `rust-version`-style version stamp at build; delete Prism on `/`, delete 11 of 14 palettes, delete the fake SVG editor, delete the roadmap page.

**Copy to change now (before any redesign):** remove the two "Paper is not an editor theme" sentences; remove "deferred GUI"; `23`→`26 languages`; hero line → "See the cut before you make it — composed searches, substitutions and selections show their exact range before Enter"; fix the tape (`Space w w`); widen the `pages.yml` filter.

---

## 9. Release / CI / docs (brief)

- README is honest and good (inspection table, supported target, retired channels). `CHANGELOG.md` entries are dense paragraphs; adopt one-line bullets with a bold lead so the generated website changelog scans.
- `install.sh` is byte-identical live vs repo **[V]**; `catalog.json` is not on strop.dev (404) — AGENTS.md gate 3 wording should say "the GitHub release catalog".
- `proofs` runs only locally; `ci.yml` has no drift check for `proofs/Extracted` (§7 P3).
- Host builds require Zig 0.16.0 for `strop-terminal` with no download (`build.rs:16`) — good for reproducibility, but `cargo build` on a fresh host fails with a one-line error and no pointer to `.github/scripts/install-zig.sh`; add the path to the message.
- `docs/vim-compat.md` and `?` help are generated from the same table — keep; `docs/editor.md:153` still says 23 languages.

---

## 10. Sequencing for ONE focused release (0.54) — do now / next / don't

**Do now (0.54 — "the look"):** one coherent change, all in `crates/strop/src/render`, `strop-core/src/theme`, and the site copy.
1. Token set (§4.2) replaces `Theme`; `render/style.rs` `Tokens` per frame; delete the global accessors and the re-exec tests (A3).
2. Six palettes: Hone (default), Slate, Carbide, Vellum, Paper, Paper Dark; delete Catppuccin/Nord + licences + `licenses` subcommand; contrast rules as a theme test.
3. `render/card.rs` + keycap primitive; migrate the nine blocks (A6); fixed-width chips; echo row for messages with severity order (U1); scrollbar thumb-only; pane title into the top border; one divider idiom; drop `Spaces:4`/scope row/services badge when default; `LSP!` badge.
4. Picker rows: grouped grep hits, no `[x]`, honest counts, diagnostics rows show the message (U6–U8); which-key grouped by section; help laid out by section order (U9–U10).
5. UX bugs with one-line fixes: `:e` cwd resolution (U2), welcome `space ?` (U5), terminal numbering (U12), no devcontainer notice on zero configs (U13), every declining leader verb speaks (U17), whole-chord hints (U16).
6. Site copy corrections + tape fix + `pages.yml` filter (§8 last paragraph). The visual redesign of the site follows the editor palette, so it is *next*, not now.
7. Split `render_pane`/`render_picker` as part of 3–4 (A1 for render) — it falls out of the primitives.
Gate: golden cell-grid snapshots for the eight mockup surfaces (§3.4) in two themes at 140×40 and 80×24 — this is the "golden snapshots" AGENTS.md promises and the regression net for every later UI change.

**Next (0.55):**
- Proof ring K1 (batch fold into the kernel) and K2 (`operated_span`) with the differential lattice growth and `preview()==execute` loop — the first grammar theorem, as 0086 F3 planned, but K2 is the better first grammar target than the operator parser because its output is literally the preview footer.
- Esc-cancels-prompt (U4) — decide it with the owner; it is a doctrine call.
- 0086 F2 as scheduled (Editor ratchet first, then subsystem structs; strop-core IO out), A2 allocation work, A5 path projection, A7 keymap split by section.
- Website redesign (§8.1) on the new palette; docs generated from `docs/*.md`.
- `[keys]` remap table (U14); `:tutor` (U15).

**Don't:**
- Don't add a 256-colour quantiser before truecolor detection exists — detect first, quantise only when the probe says so (§4.4); most users will never see it.
- Don't keep Catppuccin/Nord "for familiarity" — a zero-config static binary with its own design language should not ship other projects' brands.
- Don't build a notification centre: the echo row + `:explain` + `Space a` is the whole model.
- Don't prove undo/redo or grapheme width in Lean (§7.3 "not worth moving").
- Don't try to make `ci[` preview — the README's inspection contract is right; make the *site* match it instead.
- Don't add a roadmap page back; `STATUS.md` rendered on the front page is the roadmap.
