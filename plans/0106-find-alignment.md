# 0106 — Find alignment: one folder column in the file finder

- Status: **in force (0.70)**
- Source: the owner — "Align file / folder names in find file flow."
- Builds on [0088](0088-the-look.md) (the file-name idiom: the folder is
  context, the name is the identity and survives clipping), [0093](0093-find.md)
  (the finder) and [0102](0102-folder-view.md) §2.1 (a column computed once per
  listing; a longer name is shown in full and its row drops the column).
  Adds no colour, no glyph, no setting. Changes the renderer only.

## 1. What is on screen today

Ground truth is the 0.69.0 binary run in tmux on this repository, and
`crates/strop/src/render/picker/rows.rs` (`file_name_column` `:355`,
`file_row` `:379`). The finder lists files only; a row is the file's name,
then its folder:

| Cells | Holds | Ink |
|---|---|---|
| 0 | `▌` on the selected row, else blank | `accent` |
| 1 | one blank | — |
| 2… | file name; matched characters `accent` bold | `fg.text` |
| name column… | folder, ending in `/`, cut from the left with `…` | `fg.secondary` |

Rulers count cells from the first cell of the list (the `▌` cell). On this
repository the list is 71 cells wide at 140 columns and 41 at 80; the
scrollbar's column, the last of the list pane, is not counted.

### 1.1 `Space f`, `render`, 140 columns

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890
▌ render.rs                          …rop-engine/src/editor/filesystem/
  render.rs                          …engine/src/editor/changes/review/
  0027-search-input-and-render-hardening.md  plans/
  0075-discoverable-search-remote-and-terminal.md  plans/
  0074-input-registers-and-search-case.md  plans/
  0018-revision-native-git-and-services.md  plans/
  0092-registers-and-pinned-project.md  plans/
  0081-product-coherence-and-workspace-experience-handoff.md  plans/
  0077-in-process-search-and-responsive-pickers.md  plans/
  mod.rs                             crates/strop/src/render/
  card.rs                            crates/strop/src/render/
  clip.rs                            crates/strop/src/render/
  diff.rs                            crates/strop/src/render/
  echo.rs                            crates/strop/src/render/
```

### 1.2 `Space f`, `golden`, 80 columns

```
          1         2         3         4
01234567890123456789012345678901234567890
▌ golden.rs           …/strop/src/render/
  golden_tests.rs     …/strop/src/render/
  golden_git_tests.rs  …strop/src/render/
  golden_block_tests.rs  …rop/src/render/
  golden_modeline_tests.rs  …/src/render/
  echo-hone-80x24.golden  …render/golden/
  grep-hone-80x24.golden  …render/golden/
  help-hone-80x24.golden  …render/golden/
  echo-hone-140x40.golden  …ender/golden/
  files-hone-80x24.golden  …ender/golden/
  grep-hone-140x40.golden  …ender/golden/
```

### 1.3 `Space f`, empty query, 140 columns, pressing `Down`

The same rows before the list scrolls, after it scrolls by one, and rows
further down after it scrolls by twelve:

```
0         1         2         3         4
01234567890123456789012345678901234567890123456789
  release-catalog.py  .github/scripts/
  release-local.sh    .github/scripts/
  ci.yml              .github/workflows/

  release-local.sh  .github/scripts/
  ci.yml            .github/workflows/

  Cargo.toml            crates/strop/
  completion.rs         crates/strop/src/bench/
```

Defects:

- **A1 — a long name pushes its folder out of the column.** `file_row` takes
  `name_column.max(name_w + 2)` (`rows.rs:396`): a name wider than the column
  starts its folder two cells after itself. In §1.1 seven rows have seven
  folder positions; in §1.2 nine of eleven rows are out of line. At 80
  columns this is the usual case, not a corner.
- **A2 — the column moves while scrolling.** `file_name_column` measures the
  rows in the window (`skip(start).take(height)`), so a `Down` that brings a
  longer name in, or lets one out, shifts every folder on screen (§1.3: the
  folder starts 20, then 18, then 22 cells after the name). The folder view
  already decided this: its column is computed once per listing "so it does
  not move while scrolling" (0102 §2.1).
- **A3 — one long name sets the gap for every row, for as long as it is on
  screen.** In §1.1 `mod.rs` is 29 blanks from its folder because of plan
  files a few rows up; when they scroll away the gap closes.
- **A4 — a long name is cut although no folder follows it.** At 80 columns
  `0027-search-input-and-render-hardeni…` stops two cells before the edge:
  the cells are kept for a folder the row has no room to draw (§4.2).

The name-first order, the two inks, the two-cell gap, the left cut of the
folder and the match emphasis are right and stay.

## 2. How others lay it out

- **VS Code quick open, Zed, Telescope's `filename_first`** (from use, not
  from their sources). Name, then the folder dimmed right after it. No column:
  the folders are ragged by design. *Take:* name first, folder as context.
  *Leave:* the ragged edge — on a cell grid it is what the owner asked to
  remove.
- **fzf, Helix.** The path as written, one column. Folders line up where rows
  share a prefix; names never do. *Leave:* 0093 ranks by name and the finder
  reads by name.
- **strop's folder view (0102 §2.1).** Names at the text column; size and age
  at `clamp(longest name + 2, 26, 50)`, once per listing; "a longer name is
  shown in full and its row drops the metadata". *Take:* the whole rule. The
  bounds follow the list's width here, because the finder's list is 27 to 72
  cells wide where a folder pane is usually the whole terminal.

## 3. Verdict

**One folder column per list. A name that does not fit before it is shown
alone.**

1. The folder starts `C` cells after the name's first cell, on every row:
   `C = clamp(widest name + 2, W / 3, W / 2)`. `W` is the list's width in
   cells; *widest name* is over the first 256 rows of the list, in ranked
   order.
2. `C` depends on the list and its width, never on the scroll position or
   the selection. It changes when the query, the set of files or the terminal
   width changes.
3. A name of at most `C − 2` cells is followed by blanks up to the column,
   then its folder. The folder is cut from the left with `…` when it is longer
   than the cells that remain, and left out when fewer than four remain
   (today's rule).
4. A longer name is shown alone and whole. It is cut at the end with `…` only
   when it is longer than the row. The selected row's full path is in the
   preview header, as today.
5. A file at the project root has no folder and shows its name alone, as
   today.

Why these numbers:

- **At most half the row** is today's cap. The folder keeps at least half the
  row, and on this repository 95 % of source file names (18 cells or fewer)
  keep their folder at 80 columns.
- **At least a third of the row** is new. Without it the column follows the
  longest name of each query and jumps on every keystroke (`mod` gives 8,
  `golden` gives 26). With it, lists of ordinary names — up to 21 cells at
  140 columns — all give the same column, so folders stand still while you
  type.
- **256 rows** is the bound the engine already uses to size the list pane
  (`PICKER_MEASURE_ITEMS`). The cost per frame is fixed on a 100,000-file
  tree. A name past row 256 that is wider than the column is shown alone, by
  rule 4.

## 4. Targets

### 4.1 `render`, 140 columns (§1.1 after)

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890
▌ render.rs                          …rop-engine/src/editor/filesystem/
  render.rs                          …engine/src/editor/changes/review/
  0027-search-input-and-render-hardening.md
  0075-discoverable-search-remote-and-terminal.md
  0074-input-registers-and-search-case.md
  0018-revision-native-git-and-services.md
  0092-registers-and-pinned-project.md
  0081-product-coherence-and-workspace-experience-handoff.md
  0077-in-process-search-and-responsive-pickers.md
  mod.rs                             crates/strop/src/render/
  card.rs                            crates/strop/src/render/
  clip.rs                            crates/strop/src/render/
  diff.rs                            crates/strop/src/render/
  echo.rs                            crates/strop/src/render/
```

Name from cell 2, folder from cell 37 (`2 + 35`). The seven plan files are
shown alone; nothing is cut.

### 4.2 `render`, 80 columns

```
          1         2         3         4
01234567890123456789012345678901234567890
before
▌ render.rs           …editor/filesystem/
  render.rs           …or/changes/review/
  0027-search-input-and-render-hardeni…
  0075-discoverable-search-remote-and-…
  0074-input-registers-and-search-case…
  0018-revision-native-git-and-service…
  0092-registers-and-pinned-project.md
  0081-product-coherence-and-workspace…
  0077-in-process-search-and-responsiv…
  mod.rs              …/strop/src/render/
  card.rs             …/strop/src/render/
  clip.rs             …/strop/src/render/
after
▌ render.rs           …editor/filesystem/
  render.rs           …or/changes/review/
  0027-search-input-and-render-hardening…
  0075-discoverable-search-remote-and-te…
  0074-input-registers-and-search-case.md
  0018-revision-native-git-and-services.…
  0092-registers-and-pinned-project.md
  0081-product-coherence-and-workspace-e…
  0077-in-process-search-and-responsive-…
  mod.rs              …/strop/src/render/
  card.rs             …/strop/src/render/
  clip.rs             …/strop/src/render/
```

Folder from cell 22. A long name uses the whole row before it is cut (A4).

### 4.3 `golden`, 80 columns (§1.2 after)

```
          1         2         3         4
01234567890123456789012345678901234567890
▌ golden.rs           …/strop/src/render/
  golden_tests.rs     …/strop/src/render/
  golden_git_tests.rs
  golden_block_tests.rs
  golden_modeline_tests.rs
  echo-hone-80x24.golden
  grep-hone-80x24.golden
  help-hone-80x24.golden
  echo-hone-140x40.golden
  files-hone-80x24.golden
  grep-hone-140x40.golden
```

This is the plan's cost at 80 columns: the column is 20 cells, so nine names
of 19 to 24 cells stand alone, where today each drags a scrap of its folder
to a different cell. A cap of three fifths would keep the folder on five of
them, cut to 15 cells (§9.2).

### 4.4 Short names: the column rests at a third of the row

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890
▌ mod.rs                 src/render/
  mod.rs                 crates/strop/src/render/block/
  mod.rs                 crates/strop-engine/src/editor/normal/
  lib.rs                 src/

          1         2         3         4
01234567890123456789012345678901234567890
▌ mod.rs       src/render/
  mod.rs       …s/strop/src/render/block/
  mod.rs       …engine/src/editor/normal/
  lib.rs       src/
```

Folder from cell 25 at 140 columns (`2 + 71 / 3`), from cell 15 at 80.

### 4.5 Everything at once (the new golden, §7.3)

A root file, short names, a deep folder, a name wider than the column, a name
wider than the row; the third row selected.

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890
  Makefile
  main.rs                            src/
▌ mod.rs                             src/render/
  statusline.rs                      src/render/
  render.rs                          …engine/src/editor/changes/review/
  remote.md                          docs/
  0081-product-coherence-and-workspace-experience-handoff.md
  collection-blocks-vellum-200x50.golden
  a-file-name-that-is-longer-than-the-whole-row-of-the-list-and-is-cut…

          1         2         3         4
01234567890123456789012345678901234567890
  Makefile
  main.rs             src/
▌ mod.rs              src/render/
  statusline.rs       src/render/
  render.rs           …or/changes/review/
  remote.md           docs/
  0081-product-coherence-and-workspace-e…
  collection-blocks-vellum-200x50.golden
  a-file-name-that-is-longer-than-the-wh…

the narrowest list, 27 cells
          1         2
012345678901234567890123456
  Makefile
  main.rs      src/
▌ mod.rs       src/render/
  statusline.rs
  render.rs    …ges/review/
  remote.md    docs/
  0081-product-coherence-a…
  collection-blocks-vellum…
  a-file-name-that-is-long…
```

### 4.6 Inks (no change, stated so the column is defined)

| Part | Ink |
|---|---|
| `▌` | `accent`, selected row only |
| name | `fg.text`; matched characters `accent` bold |
| blanks up to the column | the row's background: `bg.surface`, or `bg.raised` across the whole selected row |
| folder | `fg.secondary`; matched characters `accent` bold while the folder is whole |
| `…` after a cut name | `fg.muted` (`finish`, unchanged) |

## 5. Edge cases

- **Scrolling.** `C` is the same at every scroll position (rule 2). This is
  the test that would have caught A2.
- **Streaming.** While the tree is still being walked, each new ranking may
  change the first 256 rows and so the column. It settles with the list.
- **Remote files.** The row splits at the last `/` whatever the label is; an
  `ssh://host/…` folder is cut from the left like any other. No change.
- **Wide characters.** Widths are terminal cells (`clip::width`), as today.
- **A very narrow list.** When fewer than four cells remain after the
  column, every row is a name alone. The existing width sweep (`0..30`)
  keeps asserting that no row is wider than the list.
- **Other pickers.** Buffers, search headers and location rows show the path
  as written (`dir/name`) and are not touched.

## 6. Considered and rejected

- **Cut the long name to keep its folder** (a strict two-column table). The
  name is the identity and survives clipping (0088, File names); a long name
  is nearly always unique, so its folder is the less useful half. See §9.1.
- **Let the long name overrun and put its folder after it.** Today's rule;
  it is defect A1.
- **Right-align the folder of an overrunning row.** A second alignment in one
  list.
- **Measure only the first page.** The column would depend on the terminal's
  height, and names on page two would lose folders that fit.
- **A fixed column** (always half the row). Stable, but `mod.rs` would sit 29
  blanks from its folder in every list.
- **Measure in the engine and cache it per ranking.** It needs a ranking
  counter on the picker for a number only the row painter reads. 256 width
  sums per frame allocate nothing, and they replace the per-window sum that
  runs today.
- **Path as written** (`crates/strop/src/render/mod.rs`). Names stop lining
  up.
- **A setting.** Zero-config; one look.

## 7. Implementation

One change, renderer only. No engine, kernel or proof file moves.

### 7.1 `crates/strop/src/render/picker/rows.rs`

| Site | Change |
|---|---|
| `:351-439`, the files section | move `file_name_column`, `file_row` and `split_match_cols` to a new `rows/file.rs`, beside `rows/location.rs` and `rows/search.rs` (`rows.rs` is 694 lines; this takes it to about 600). `rows.rs` keeps `mod file;` and imports the two functions it calls |
| `file_name_column` `:355-374` | signature `(picker, width: u16) -> usize`; `start` and `height` go. Sample `picker.rows.iter().take(NAME_SAMPLE)`, `const NAME_SAMPLE: usize = 256;`. Return `(widest + 2).clamp(w / 3, w / 2)` with `w = usize::from(width)`; `w / 3 <= w / 2` always, so `clamp` cannot panic. Doc comment: the folder column, once per list, never per window |
| call site `:95-99` | `file_name_column(picker, area.width)` |
| `file_row` `:379-423` | delete `let column = name_column.max(name_w + 2).min(budget);` (`:396`). Two shapes, chosen by one test. `name_w + 2 <= name_column`: the name, `name_column − name_w` blanks, the folder cut to `budget − name_column` cells and left out under four. Otherwise: the name alone. The name is not cut with `clip_end` in either shape; `finish` already cuts a row wider than the list and appends `…` |
| doc comment `:376-378` | the folder stands at the list's one column; a name too wide for it is shown alone |

`finish`, `emphasis`, the logic of `split_match_cols`, `file_name.rs`, the
preview and `prepare/geometry.rs` need no change.

### 7.2 Order

1. Move the three functions to `rows/file.rs` with no behaviour change; the
   suite and the goldens stay green.
2. Change `file_name_column` and its call site; add the scrolling test (§7.3
   item 2). It fails before this step and passes after.
3. Change `file_row`; update the unit test (§7.3 item 1); add items 3–5.
4. Add the golden scene; generate with
   `STROP_UPDATE_GOLDEN=1 cargo test -p strop-editor` and read the new files
   against §4.5.
5. Words (§7.4). Done when `docker compose run --build --rm test` is green.

### 7.3 Tests

Existing and expected to pass unchanged — a diff there is a bug:

- Goldens `files-{hone,vellum}-{80x24,140x40}`: the list is 27 cells and
  the names are `mod.rs` and `remote.md`, so `C = clamp(11, 9, 13) = 11`,
  today's value.
- `workspace_tests.rs`,
  `long_file_paths_keep_basename_and_preview_identity_visible`.
- Every other picker golden (`grep*`, `locations*`, `git-branches*`,
  `actions*`): their rows do not go through `file_row`.

To change:

1. `rows/tests.rs:462`,
   `file_rows_put_the_directory_after_the_name_at_a_shared_column`: keep its
   assertions. Add a third row whose name is wider than the column
   (`plans/0081-product-coherence-and-workspace-experience-handoff.md`,
   column 14, width 44): its text is the marker cell, a blank, 41 cells of the
   name and `…`; it has no `/`; its width is 44.

To add:

2. **The column does not move while scrolling** (`workspace_tests.rs`): seat
   60 items with `picker_items_fixture`, short names first and one 30-cell
   name at row 40; draw at 140×30 and read the cell where the first row's
   folder starts; move the bar down 45 rows one at a time, drawing each time.
   In every frame, every row that shows a folder starts it at that cell.
3. **The bounds** (tests beside `file_name_column`, on a fixture picker):
   6-cell names at width 71 give 23; a 24-cell name gives 26; a 60-cell name
   gives 35; at width 41 the three give 13, 20 and 20; a 60-cell name at row
   300 changes nothing.
4. **A long name is alone and whole**: at width 71 and column 35 the row of a
   58-cell name is the marker cell, a blank, the 58 cells and blanks, with no
   `/`; at width 41 it is 38 cells of the name and `…`.
5. **Match emphasis keeps its cells**: a row with matches in the name and in
   the folder has `accent` bold on exactly those characters at the new
   column. Assert by cell, not by span.
6. **Golden `files-columns`** (`golden_tests.rs`, through `each`, so hone and
   vellum at 80×24 and 140×40): open the finder, `picker_reset_fixture`, seat
   the nine paths of §4.5 with `picker_items_fixture`, put the bar on the
   third row. The list pane is sized from these labels, so its width may not
   be 71 and 41; the rule is the same at whatever width it gets. Check each
   generated grid by hand against §3 — folder at
   `2 + clamp(widest + 2, W / 3, W / 2)`, long names alone — and paste the
   two hone grids into §4.5 when the plan goes in force.

### 7.4 Words

- `plans/STATUS.md`: 0106 under Proposed now; in force with the release.
- `plans/0088-the-look.md`, the File names amendment, one line: "*In the
  file finder the folder stands at one column per list; a name too wide for
  it is shown alone ([0106](0106-find-alignment.md)).*"
- `CHANGELOG.md`: "The file finder keeps folders in one column: it no longer
  shifts while scrolling, and a long file name no longer pushes its folder
  out of line."
- `docs/`: nothing describes the column; no change. The site's frames are
  recaptured from the release binary as usual.

## 8. Left out, on purpose

- **Sizing the list pane for two columns.** The engine sizes the list from
  the widest path plus three cells (`measure_picker` in
  `prepare/geometry.rs`). A row needs the widest name, the gap and the widest
  folder, which can be a few cells more, so in a small project a folder may be
  cut while the preview has cells to spare. That is true today as well. Fixing
  it is an engine change with its own tests (`prepare/picker_tests.rs`) and it
  moves the existing `files` goldens; it is the follow-up if cut folders show
  up in small projects.
- **Folders cut at a `/`.** See §9.3.

## 9. Taste questions for the owner

1. **A name too wide for the column: alone, or cut?** Plan: alone and whole
   (`0081-product-coherence-and-workspace-experience-handoff.md`), as the
   folder view does. The alternative is a strict table,
   `0081-product-coherence-and-work…  plans/`: every row has a folder and
   long names lose their end.
2. **The bounds.** Plan: the column is between a third and a half of the
   list. A smaller floor puts short names nearer their folders and lets the
   column move as you type. A larger cap (three fifths) keeps folders for
   names up to 22 cells at 80 columns and cuts more folders at 140.
3. **Cut folders at a `/`?** Today a folder is cut wherever the cells run
   out: `…rop-engine/src/editor/filesystem/`,
   `…engine/src/editor/changes/review/`. Cutting at a separator gives
   `…/src/editor/filesystem/` and `…/src/editor/changes/review/`: every cut
   folder starts `…/`, at the cost of up to one folder name of cells. It is
   not in this plan because the same cut is used by search headers, pane
   titles and the modeline (0088, File names) and should change everywhere or
   nowhere.
4. **Name first stays.** The finder is the only list that puts the name
   before the folder; buffers and search headers show `dir/name`. Plan: keep
   it. The finder ranks by name (0093 §3.2) and the name column is what the
   eye scans.
