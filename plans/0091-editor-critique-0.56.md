# 0091 — Independent critique of 0.56: the motto, the core workflows, what makes it click

- Status: **record** (a critique; adoption is a separate plan, as 0088 was for 0087)
- Date: 2026-11-10
- Base: `main` at 0.56.0 (`9ff9ee8`), release build of that tree
- Owner's brief: the motto is "see the cut before you make it". "I feel like the editor
  isn't doing that honestly and that is okay; if at the end we are Neovim reimplemented in
  Rust that should be fine. We should just double down on performant, high-quality
  functionality like global search and replace, find files, multibuffer edits — what makes
  an editor click."
- Not re-investigated (being fixed elsewhere): registers not pasting into pickers/prompts;
  project root drifting after go-to-definition outside the project.
- The plan ledger (`plans/STATUS.md`) needs a Record row for this plan; this change
  commits the plan file only.

## 0. Method

Everything marked **[V]** was run against the release binary, not read from source.

- **Headless driver** (`strop headless SCRIPT FILE`, `frame`/`state`/`settle`) for
  behaviour and cell grids, with a private `HOME`/`XDG_*`.
- **A pty driver** (the real TUI in a 160×45 pty, bytes timestamped) for latency. "first"
  is the time from writing a key to the first output byte; "settled" is the last byte
  before 250 ms of silence.
- **Repos**: this one (4,094 files); a shallow clone of `torvalds/linux` (97,256 files,
  the finder counts 97,349); an 11.4 MB / 103,633-line generated header from it.
- **Machine**: 32 cores, WSL2, warm page cache unless stated. `rg` 14.1.0 and `nvim` 0.9.5 on
  the same box are the comparison.
- **An ad-hoc differential**: 151 everyday key sequences on one 10-line buffer, fed to
  strop headless and to `nvim --clean` (`sw=4 et`), final text compared (§2).

Not run: SSH/remote (0090 covered it), the Docker gates, the debugger (not built), any
language server but rust-analyzer.

## 1. Executive summary

1. **The motto is true on four surfaces and false on the one it was written for.** `d/pat`,
   `:s`, `Space R` and filesystem review show the change before Enter. `dw`, `ci[`, `dap`,
   `.` and every other completed chord — the grammar, i.e. the product — show nothing,
   and cannot: the preview needs a complete command, and a complete command executes
   (§3). The README says so; the welcome card and 0001 §2.1 still say the opposite.
   **Retire it as the headline.**
2. **The bigger honesty gap is "Neovim's hands".** 80 of 151 everyday sequences produce
   different text than Neovim **[V]** (about 70 once harness artefacts and deliberate
   reassignments are removed). No `ip`/`ap`, `gU`/`gu`/`g~`, `R`, `Ctrl-A`/`Ctrl-X`, visual
   `p`/`J`/`U`/`r`, numbered registers, insert-mode `Ctrl-W`/`Ctrl-U`/`Ctrl-R`/`Ctrl-O`,
   `d'a`, `:g`, `:norm`, `:'<,'>`, `:b`, `:wa`, `Ctrl-W`. The differential corpus that
   gates "fidelity" is 72 cases. "Neovim reimplemented in Rust" is a fine goal; it is
   not yet the product.
3. **The finder opens the wrong file.** Type `Space f Makefile` and Enter on a large tree
   before the walk settles: the selection bar is on row 2 and `Documentation/Makefile`
   opens, not the top-ranked `Makefile` **[V]** (§4.1 B1).
4. **Search-and-replace is the best surface and the most over-built.** The live per-hit
   `-`/`+` rows in `Space R` are better than Telescope and equal to grug-far. Then come
   three stages and two typed ex commands (`:apply-change`, `:save-change`), `Space a`
   does not offer Apply, the receipt never leaves "saving", and undo after save leaves
   34 dirty buffers with no `:wa` **[V]** (§4.3).
5. **There is no editable multibuffer.** Collections are read-only by contract
   (`docs/search.md:103`). `dd` in one says "collection is read-only". This is the one
   feature in the owner's list that does not exist.
6. **Speed is good where it is in-memory and average where it walks.** Keys echo in
   0.2–0.7 ms; the finder paints in 8 ms on 97K files. Project search is single-threaded
   and path-ordered: 0.84 s to the first hit and 1.2–1.3 s to finish on linux, against
   `rg` at 0.10 s **[V]** (§4.2).
7. **`:%s/edge/rim/g` rewrites `Edge`**, because search is case-insensitive by default,
   and the preview card clips the row so the changed word is off-screen **[V]** (§3.2).
   The one place the motto works can still mislead.
8. **Ranking is walk order.** `sched.h` lists twelve exact matches alphabetically by
   directory; `include/linux/sched.h` is fourth. No frecency, no open-buffer boost, no
   shorter-path tie-break, an empty query shows `.dockerignore` first **[V]**.
9. **`Space a` is a filesystem menu.** Twelve file operations lead on every buffer,
   including a change proposal and a diagnostic line **[V]**.
10. **The website sells the proof and the process, not the editor.** The hero is
    `d/width`; a third of the page is Lean theorem names and the plan ledger; there is
    no picture of the finder, search or replace; three statements are stale (§6).

What I would do, in order: stop claiming, fix the keys, make finding right, make changing
simple, then git and the site (§7).

## 2. Fidelity: the claim nobody tested

AGENTS.md: "the vim grammar is the product. Deviations are bugs, and bugs are found by the
differential harness". The harness is `crates/strop/tests/differential.rs`: **72 cases,
one known divergence**. `docs/vim-compat.md` lists only what exists, each marked `✓ ships
exactly`, so a reader cannot learn what is missing.

151 sequences on one buffer, strop headless against `nvim --clean` **[V]**. 80 differ in
final text. Excluding my own artefacts (two nvim timeouts, a typo, two duplicates) and
deliberate reassignments (`gI`, `K`, `gd` without a server), about 70 are real.

**Missing outright** (key falls through, often into another command):

| Class | Missing |
|---|---|
| Text objects | `ip` `ap` (so `dip`, `vip`, `>ip`, `=ip`, `gqip`), `ib` `iB`, `it` `at`; `viwiw` / `vawaw` do not extend |
| Operators | `gU` `gu` `g~` (and `gUU`, `guu`, `g~~`), `!`, `gq`/`gw`, `=` with a motion, `g?` |
| Normal | `R`, `Ctrl-A`, `Ctrl-X`, `U`, `gJ`, `gn`, `g&`, `&`, `@:`, `-`, `+`, `N%`, `(` `)`, `[{` `])`, `g_`, `g0`, `g*`, `]p`, `gp`, folds (`zf` …) |
| Marks as motions | `d'a`, `` d`a ``, ` `` ` |
| Operator modifiers | `dvj`, `dVw`, search offsets (`d/pat/e`) |
| Visual | `p` ("not an editor command: p"), `J`, `U`/`u`, `r`, `=`, `*`, `I` in charwise, `Ctrl-V` `r`, `Ctrl-V $A` |
| Registers | `"0` after a delete, `"1`–`"9`, uppercase append (`"Ayy`) |
| Insert | `Ctrl-W`, `Ctrl-U`, `Ctrl-R {reg}`, `Ctrl-O`, `Ctrl-T`, `Ctrl-D` |
| Ex | `:g`, `:v`, `:norm`, `:m`, `:t`/`:co`, `:>`/`:<`, `:sort`, `:b`, `:bn`/`:bp`, `:ls`, `:reg`, `:marks`, `:wa`, `:cd`, `:tabnew`, `:set` beyond `ro`; ranges `'<,'>` and `.,+1`; `:2d a` |
| `:s` replacement | `&`, `\1`, `~` are literal (documented, `docs/search.md:82`) — the most common reason to type `:s` |
| Windows | `Ctrl-W` is unbound (`demos/demo.tape:152` says so) |

**Wrong, not missing** (repro = keys on any multi-line buffer):

| Keys | strop | Neovim |
|---|---|---|
| `2D` | deletes to end of this line | also deletes the next line |
| `3J` | joins four lines | joins three |
| `dk` on line 1 | deletes line 1 | fails, no change |
| `qaA;<Esc>jq2@a` | `;;` on line 2, `;` on line 3 | `;` on lines 2 and 3 |
| `yyjdd"0p` | `"0` no longer holds the yank | pastes the yanked line |
| `Vj:s/e/E/g` | `:` opens empty; substitutes on the cursor line only; stays in V-LINE | `:'<,'>`; both lines |
| `Ctrl-W v` | `Ctrl-W` dropped, `v` enters Visual | vertical split |
| `:%s/edge/rim/g` | also rewrites `Edge` | case-sensitive |

`Ctrl-W` deserves its own line: the prefix is swallowed and the second key runs as a
Normal command (`Ctrl-W v` entered Visual **[V]**). By the same path `Ctrl-W c` is a
change operator, `Ctrl-W o` opens a line and `Ctrl-W q` starts recording a macro. Muscle
memory damages the buffer.

**Judgement.** This outranks every feature below. A vim user hits `dap`, `gUiw`, visual
`p` or `Ctrl-W` in the first five minutes, and from then on does not trust the keys. The
remedy is cheap relative to what the repo has already built: the resolver is pure, the
harness exists, and most of the list is table entries.

## 3. The motto, audited

### 3.1 Where the cut is shown

| Surface | Shown before commit? | Verdict |
|---|---|---|
| `d/pat` composing | Yes — range overlay plus `delete, search /pass, 30 bytes · exclusive` **[V]** | Real, and nobody else has it. Used a few times a week. |
| `:s` composing | Yes — count, changed lines, `-`/`+` rows **[V]** | Real. Neovim has had `inccommand=split` since 2017; strop's is a floating card over the text rather than in place (§3.2). |
| `Space R` | Yes — every hit as `-`/`+`, live as the With field is typed **[V]** | The best expression of the motto in the product. |
| Change proposal (replace, filesystem) | Yes — a unified diff buffer | Real, heavy (§4.3). |
| Visual then operator | Yes | This is vim. Every modal editor does it. |
| `f`/`t` candidates | Candidates light up after `df` | A target hint, not a preview of the cut. |
| `dw`, `ci[`, `dap`, `dd`, `x`, `.` | **No.** `d`, `di`, `d2`, `df` render only `PENDING` and the keys **[V]** | The grammar. |
| Macros, `:!`, `Space \|` | No | Documented. |
| LSP rename | **No** — `:rename sharpen` applied at Enter: "rename: applied to 1 buffer(s)" **[V]** | Inconsistent with replace, which reviews. |
| LSP code actions, format | No | — |
| Completion Tab-cycle | Yes (live preview of the candidate) | Standard. |

Why the grammar row is "no" by construction: `preview_command`
(`crates/strop-engine/src/editor/normal/preview.rs:103`) only yields a command when
`grammar::parse(op + motion)` is `Complete`, and a complete operator+motion has already
executed. Outside the search prompt there is no state in which the preview can be on
screen. 0001 §2.1's table ("motions + text objects + counts: perfect, updates per
keystroke") describes frames that are never drawn. 0081 amended the wording; the
mechanism was never the problem — vim's grammar executes at the completing key, and
fidelity (doctrine 1) forbids a pause.

### 3.2 Where it is decoration, or misleads

- **The welcome card** says `ci[ previews the cut` (`crates/strop/src/render/mod.rs:200`).
  The README's own table says it does not. It is the first sentence a new user reads.
- **The logo alt text** describes "an editor card mid `ci[`, the pending cut tinted"
  (`README.md:2`) — a frame the binary cannot produce.
- **The spec footer** (`30 bytes · exclusive`) is shown only for `d/pat`. As a teaching
  tool for the grammar it teaches one motion.
- **The `:s` card clips.** At 90 columns, `:%s/edge/rim/g` shows
  `-fn sharpen(blade: &mut Blade, passes: &[Pass]) ->` and an identical `+` row: the
  changed word (`Edge` → `rim`) is past the card's edge **[V]**. The preview shows that a
  line changes and hides how. With more than eight changed rows the card also covers the
  statusline (`cmd_card.rs:66-72`: height is capped at `rows − 1` from `y = 1`).
- **"The preview cannot lie"** is a true statement about an implementation property
  (one resolver). Users read it as a product promise; the clipped row and the
  case-insensitive surprise are how a truthful preview still misleads.

### 3.3 Recommendation: retire as headline, keep as a feature

Keep the behaviour. Stop leading with it. The reasons, ranked:

1. It is unfalsifiable-in-use for the 95 % of edits that are completed chords, and the
   site already spends a table explaining what it does not mean. A headline that needs a
   disclaimer table is the wrong headline.
2. The surfaces where it is true are not why someone switches editors. Nobody leaves
   Neovim for `d/pat` highlighting.
3. The owner's instinct is the stronger pitch and it is the one 0001 §1 opened with:
   **Neovim's grammar on a fast core with search, git and LSP built in, zero config.**

Do **not** try to make the motto true by adding a delay or a confirm step to chords.
That trades doctrine 1 for doctrine 2.

Positioning to adopt — only after §7's first release makes the first clause true:

> **Vim's keys. Nothing to configure.**
> strop is a modal editor in one static binary. Find a file, search a project, replace
> across it, read the blame, jump to a definition — with the keys already in your hands
> and no plugin to install.

Until the fidelity gate passes, the honest form is "Vim's grammar — a growing, tested
subset" with a link to a compat table that lists what is missing (§6).

Where the old motto survives, narrowed:

> **Review before you replace.** A project-wide replace shows every hit as a diff while
> you type, and `:s` and `d/pattern` show their exact range before Enter.

The name still works: a strop is the last pass on an edge. "The last pass" is a better
frame for a finishing-quality editor than a preview claim.

## 4. Core workflows against the best in class

### 4.1 Find files (`Space f`) — vs fzf-lua / Telescope / Zed / VS Code

Measured on linux, 97K files **[V]**:

| | strop |
|---|---|
| Picker first paint | 8 ms |
| Catalogue complete | ~450 ms |
| Key → first byte | 0.2–0.7 ms |
| Key → repaint settled | 9–45 ms (one 307 ms outlier while the walk streamed) |

That is as fast as anything shipping. What is wrong is what it shows.

- **B1 — wrong file on Enter (ship-stopper) [V].** linux tree, fresh start: `Space f`,
  type `Makefile` at once, Enter → `Documentation/Makefile`. The bar sits on row 2; row 1
  is the root `Makefile`. Same with `schedcore` (bar on row 5 of 11) and `core.c sched`
  (row 11). Typed at 150 ms per key, or after the catalogue settled, the bar is on row 1.
  Also seen after settle with `hidden:include ignored:include vmlinux` (bar on row 12).
  The selection is tracking a row across re-ranks when the user has not moved it. Rule:
  until the user moves the selection, it is row 1.
- **Ranking is walk order within a score.** `sched` → `arch-support.txt` (matched in a
  directory name) leads; `sched.h` and `kernel/sched/` are not on the first page.
  `sched.h` → 12 exact basenames in alphabetical directory order;
  `include/linux/sched.h` is fourth. No basename-over-directory bonus, no shorter-path
  tie-break. (`sort_by_file_name`, `crates/strop-picker/src/source/selection.rs:76`.)
- **No frecency, no recents.** Empty query: `.dockerignore`, `.github/scripts/…`. After
  opening `dispatch.rs` and reopening the finder: the same list. fzf-lua `oldfiles`,
  Telescope `smart-open`/`frecency`, Zed and VS Code all lead with what you touched.
  Sessions already persist per project; the data has a home.
- **Space is not AND.** `core.c sched` is one fuzzy string and returns junk
  (`scd30_core.c` first). fzf semantics (space-separated terms, each must match) are what
  hands expect; the qualifier grammar (`fpath:`) is a poor substitute for it.
- **`Ctrl-U` does not clear the field** — it scrolls. In a query field, readline wins.
- **Hidden/ignored** are typed qualifiers (`hidden:include`) or `:search-options`. No
  toggle key, and the state is shown only once it differs. Acceptable; a key is cheaper.
- **Buffers (`Space b`)** is MRU with the current buffer first, so `Space b Enter` is a
  no-op. Preselect the alternate.
- `:e path<Tab>` completes to an absolute path.

### 4.2 Global search (`Space /`) — vs Telescope live_grep / VS Code / Zed

Works well: grouped by file with counts, preview pane with context, resumable
investigation, per-hit and per-file exclusion (`Ctrl-X`, `Ctrl-D`), literal by default,
no `rg` needed locally.

| Query on linux, warm **[V]** | first hit | complete | `rg` (32 threads) | `rg -j1` |
|---|---|---|---|---|
| `schedule_timeout_idle` (34 hits, 17 files) | 837 ms | 1,152 ms | 0.10 s | — |
| `kmalloc` (9,068 hits, 3,753 files) | — | 1,313 ms | 0.10 s | 0.86 s |
| `open_picker` on this repo's `crates/` (104 hits) | — | 27 ms | — | — |

- **Single-threaded, path-ordered walk.** Headless `kmalloc`: 1.45 s wall, 1.30 s user.
  The first hit for a rare term arrives when the walk reaches its directory (837 ms for
  `drivers/md/`). On a 100K-file repo every keystroke restarts a one-second scan; on a
  1M-file monorepo it is ten. `rg` is 10× faster on the same box because it is parallel.
  Deterministic order was chosen for golden stability (0088 §6); pay for it by sorting
  results, not by serialising the search.
- **No toggle keys.** Regex is `regex:"…"` typed into the query; case is
  `case:sensitive`; whole-word is `\b` inside a regex. VS Code's three toggles
  (`Alt-C`/`Alt-W`/`Alt-R`) are the baseline, and Telescope is regex by default. The
  qualifier language is expressive and slow to reach for.
- **Globs** are `glob:`/`path:`/`-path:` qualifiers. Fine.
- **Results are not a buffer** while in the picker; `Ctrl-O` collects them into one, which
  is read-only (§4.4).
- **SSH scopes cannot replace** (`docs/search.md:133`).

### 4.3 Search and replace across files (`Space R`) — vs grug-far / VS Code / `:cdo`

The good part first, because it is very good: with the With field visible, every hit is
drawn as a `-`/`+` pair that updates per keystroke, with per-hit and per-file exclusion
**[V]**. That is grug-far's core idea, built in, on a fast engine.

The flow after that, replacing `open_picker` in 34 files **[V]**:

```
Space R  open_picker  Tab  open_the_picker  Enter      -> "change proposal 1" (RO diff buffer)
:apply-change                                           -> 34 buffers edited, proposal becomes a receipt
:save-change                                            -> "save receipt" buffer, files written
```

- **Three stages, two typed ex commands.** grug-far: edit, one key. VS Code: one chord.
  The proposal buffer has no key that applies it.
- **B3 — `Space a` on a proposal has no Apply [V].** `docs/search.md:117` says it exposes
  Apply/Save/Cancel/Inspect. It lists 12 filesystem operations; filtering by `apply` says
  "No actions match". Pressing Enter on the default row ("New file") and typing the next
  command created a filesystem review to create a file named `:save-change`.
- **B4 — the save receipt never updates [V].** Every row still reads `saving` after
  `settle 30000`; the files are on disk.
- **B5 — undo does not round-trip [V].** After save, `:undo-change` reports "undid replace
  across 34 buffer(s)"; the disk still has the replacement, 34 buffers are dirty,
  `:save-change` says "no changed files to save" and `:wa` is "unknown ex". The only way
  back is `:w` in each of 34 buffers. Plain `u` in the receipt buffer says "readonly
  buffer".
- **Replacement is literal only.** No `$1`. Regex search with literal replace covers
  renames and nothing structural.
- **36 buffers open** after a 34-file replace. Expected for `:cdo`, noise for a rename.

### 4.4 Multibuffer / quickfix-style editing — vs Zed multibuffers / quickfix + `:cdo`

- **Collections** (`Ctrl-O` from search or references) are the right shape: file cards,
  excerpts with context, `]f`/`[f`/`]e`/`[e`, `+`/`-` context, syntax highlighting.
- **They are read-only by contract**: "never write through to source buffers"
  (`docs/search.md:103`); `dd` → "collection is read-only — g<Space> opens the source";
  `:w` → "collection files are read-only; no collection save" **[V]**.
- There is no quickfix list, no `:cdo`/`:cfdo`, no `:g`, no `:norm`. So there is **no way
  to make a non-uniform edit at N search hits** other than visiting each.

This is the gap between "search is good" and "the editor clicks". Zed's answer is that
the excerpt view *is* the editing surface; vim's is quickfix + `:cdo` + macros. strop has
the view and neither answer. Since the kernel already proves `map_through_edits` and
batch validity, write-through excerpts are the feature the verified core was built for.

### 4.5 Buffers, jumplist, marks

- Jumplist: per pane, 100 entries, persisted, `Space j` picker with preview **[V]**. Good.
- Marks: `ma`, `'a` work; **marks are not operator targets** (`d'a` does nothing), no
  `:marks`, no ` `` `.
- Buffers: `Space b` only. No `:b name`, `:bn`, `:bp`, `:ls`. `Ctrl-^` works.
- No tabs (deliberate, 0001 §3). No folds.
- Changelist `g;`/`g,` present.

### 4.6 LSP navigation — vs Neovim 0.11 defaults / Zed

rust-analyzer on a two-file crate **[V]**:

- `gd` works, `Ctrl-O` returns.
- **B8 — `gr` rows are unreadable.** The Locations picker prints absolute paths clipped at
  the tail: three identical rows of `/tmp/claude-1000/-home-tarek-workspace-strop/1b68…`.
  File name, line and source text are not visible; only the preview pane distinguishes
  them. Rows should be `src/main.rs:15  let n = hone(&mut blade, 4);` like grep rows.
- References → `Ctrl-O` gives a (read-only) collection. Right idea; see §4.4.
- **Rename has no preview** and no prompt: `:rename NEW` typed in full, applied at Enter.
  There is no `grn`/`Space r`. Neovim 0.11 binds `grn`, `gra`, `grr`, `gri`.
- `Space S` workspace symbols: `stro` → `stroke`, then five `pkg std / core / alloc …`
  rows that do not match the query.
- `Space a` on a diagnostic line (cursor in column 1) showed the 12 filesystem rows and
  nothing else. Filesystem rows are appended to every list
  (`editor/actions/mod.rs:58-62`); code actions are requested afterwards and rank above
  them only if the server offers one at the cursor.
- A stray `.rs` outside a Cargo project shows `LSP!` and three echo rows of
  rust-analyzer prose about `linkedProjects`.

### 4.7 Git — vs gitsigns + fugitive/neogit / lazygit

- Present and solid: gutter, `]c`/`[c`, hunk stage/unstage/restore/preview, graph log,
  commit → files → delta, file history, blame gutter with dive, permalinks **[V]** (log
  and commit files).
- **Missing: status.** No list of changed files, no way to jump between modified files,
  no commit, no branch switch, no stash, no diff of the working tree as a whole. The
  working-tree changed-files picker is the most-used git surface in every editor and it
  is the one strop lacks. Staging hunks without being able to commit is half a loop.

### 4.8 Large files

11.4 MB, 103,633 lines **[V]**: first paint 13 ms, settled 414 ms; `G` settles in 192 ms;
`dG` 532 ms; `:%s/mm/MM/g` preview resolves in 1.14 s off the input path while keys keep
echoing in under 1 ms. No complaint.

## 5. Bloat, gaps, half-built

### 5.1 Cut

| What | Why |
|---|---|
| **Review → Apply → Receipt for text changes** as three buffers and three ex commands | Collapse to: review buffer, one key applies and writes, `u` in that buffer undoes and rewrites. Receipts, "Inspect result", "Verify operation outcome", "Operations and receipts" go. |
| **The 12 filesystem rows in `Space a`** on non-directory buffers | They belong in the Directory buffer (oil-style edit) and under `:fs`. `Space a` on a source buffer is code actions and nothing else. 0089 already said "simplify to an oil-style edit-buffer"; 0.56 extended the machinery to SFTP instead. |
| **Read-only collections** as a separate concept | Become the editable multibuffer, or go. A second read-only results view beside the picker earns nothing. |
| **`:explain`, retained-notice multiplexer** for local editing | With the worker gone, what is left to explain is LSP start failures; `:messages` covers it. |
| **Trace/replay/export from the README and CLI help** | A developer tool on the front page of a user document. Keep the code, move the docs. |
| **Lean claims and the plan ledger on the landing page** | §6. |
| **`g<space>`, `]e`/`[e` in the Moving table** | Collection-local keys listed among `h j k l`. |
| **Release cadence** | 91 changelog entries since 2026-09-03; 0.55.0 and 0.56.0 share a date. The changelog page is 200 KB. Batch to the themed releases in §7. |
| **0001 §2.1's coverage table and "(the soul)"** | Mark superseded. It describes frames that are not drawn. |

Not cutting: the query language (keep, add toggle keys in front of it), the undo tree,
multicursor, the terminal, SSH, the kernel proofs (they are small and they are the right
foundation for §4.4 — they just do not belong in the pitch).

### 5.2 Missing — in the order a user notices

1. The keys in §2.
2. A finder that leads with what you used (§4.1).
3. Editing at search hits (§4.4).
4. Git status and commit (§4.7).
5. One-step replace with real undo (§4.3).
6. Regex captures in replacement, in `:s` and `Space R`.
7. `:g`, `:norm`, `:wa`, `:b`.
8. Rename with a preview and a key.
9. A `[keys]` remap (deferred since 0087 U14). With `Ctrl-W` unbound and `Space w`
   mandatory, there is no escape hatch.
10. Folds; `:set` for `wrap`, `nu`, `rnu`, `ts`.

### 5.3 Half-built

- Operator preview: the resolver, overlay and footer exist and are reachable from one
  prompt.
- Collections: the view without the write.
- Change undo: reverts buffers, not files.
- `Space a`: one list, wrong contents.
- `strop compat` / `docs/vim-compat.md`: a list of what exists presented as a
  compatibility statement.
- Echo row: untyped messages (0088 §6), so server prose and one-word notices share a row.

### 5.4 Bugs hit, with repro

| # | Severity | Repro | Observed |
|---|---|---|---|
| B1 | high | linux tree: `strop README`, `Space f`, type `Makefile` immediately, Enter | opens `Documentation/Makefile`; bar was on row 2 |
| B2 | high | 9-line Rust buffer with `Edge` and `edge`: `:%s/edge/rim/g` at 90 columns | `Edge` → `rim`; preview row clipped before the change; card covers the statusline |
| B3 | high | `Space R`, find, Tab, with, Enter, `Space a`, type `apply` | "No actions match"; 12 filesystem rows only |
| B4 | medium | …`:apply-change`, `:save-change`, wait | receipt rows stay `saving` |
| B5 | high | …then `:undo-change` | buffers reverted and dirty, disk unchanged, no `:wa` |
| B6 | low | headless: `resize 90 8`, `keys :%s/edge/rim/g`, `frame` | panic `index outside of buffer … (20, 8)` in `render_cmd_card`; the TUI in an 8-row pty did not crash |
| B7 | high | `:vs` then `Ctrl-W v` (or `c`, `o`, `q`) | prefix dropped, second key runs in Normal |
| B8 | medium | rust-analyzer project under a long path: `gr` | rows are identical clipped absolute paths |
| B9 | medium | `2D`; `3J`; `dk` on line 1; `qaA;<Esc>jq2@a`; `yyjdd"0p` | §2 table |
| B10 | medium | `strop` with no file | welcome card: "ci[ previews the cut" |
| B11 | high | `Vj:s/e/E/g` Enter | substitutes on one line; still in V-LINE |
| B12 | low | `Space f`, type, `Ctrl-U` | query not cleared |
| B13 | low | `yiwwviwp` | "not an editor command: p" |

## 6. Website (strop.dev)

The live page is byte-identical to the repo's `index.html` **[V]**, loads in 0.36 s, has
no trackers, and the palette switcher is a nice touch. The craft is fine. The message is
aimed at the author.

**Structure today:** hero (`d/width`) → install → four feature blurbs + an inspection
table + the grammar card + "How it is checked" (ten Lean theorem names) → a GIF behind a
click → "Plans in force" (the ledger) → palettes.

1. **The hero shows the rarest interaction.** A real frame of `d/width` with a caption
   about `114 bytes · exclusive`. A visitor learns nothing about what editing in strop
   is like. Replace with three real frames in tabs: the finder on a large tree, `Space R`
   with `-`/`+` rows, the blame gutter. The first two already exist as golden surfaces (0088 §5).
2. **The headline needs a disclaimer table** ("there is no inspection interval, and the
   site does not claim one"). Retire it per §3.3.
3. **"How it is checked" outweighs "what it does".** Ten theorem identifiers
   (`map_through_edits_composition` …) on a landing page read as insecurity to a user
   and as noise to everyone else. One sentence and a link to an `/engineering` page.
4. **"Plans in force" is not a status.** A table of plan numbers ("0008 — the input
   walker and the one keymap table") tells a visitor nothing about what works. Replace
   with what a user asks: platforms, languages with LSP, what vim features are missing,
   what is next.
5. **Stale or false [V]:**
   - `Space f files · o remote/containers` — containers were cut in 0.55.
   - "host adapters by fault injection and end-to-end runs over a real sshd and Docker"
     — the container lane is gone.
   - "Each claim below is a tested behaviour of the shipped binary" next to "Neovim's
     hands" — see §2.
   - The demo tape renders in `Catppuccin Mocha` (`demos/demo.tape:9`), a palette 0088
     deleted, under the heading "What you see is what you install".
6. **"Neovim's hands, Helix's spine, rootle's eyes, GitLens' memory"** assumes the reader
   knows rootle. Say what it does.
7. **Install** is the strongest section: one line, checksum, `strop update`. But the
   platform line (Linux/WSL x86_64 only; macOS and aarch64 "historical") is the single
   most disqualifying fact and sits in paragraph three. Put it beside the install
   command. If macOS is not coming back, say so; if it is, say when.
8. **Docs** are the repo's `docs/*.md` rendered: accurate, reference-shaped, and written
   in the implementation's vocabulary ("owned jobs", "admission", "revision-owned worker
   results", "descriptor-pinned parents"). There is no "first ten minutes" page and no
   page titled "Coming from Neovim". Add both; the second is the compat table with a
   *missing* column.
9. **The demo** is a 7.5 MB GIF behind a click, first frame a plain buffer. Three short
   loops (5–8 s, < 500 KB each, autoplaying muted video) beat one tour.
10. **Changelog page** is 200 KB of 91 releases. Show the last three, link the rest.

**Proposed copy** (after §7's first release; before it, drop "the keys you know"):

```
strop
Vim's keys. Nothing to configure.

A modal editor in one static binary. Find a file, search a project, replace across it,
read the blame, jump to a definition — no plugins, no setup.

  curl -fsSL https://strop.dev/install.sh | sh        Linux and WSL, x86_64

[ find ]  [ search & replace ]  [ git ]        <- three real frames

Find            Space f — 100,000 files, first results in 10 ms, the file you had open first.
Search          Space / — live, literal by default, regex and case one key away.
Replace         Space R — every hit as a diff while you type. Enter applies. u undoes.
Edit the hits   Collect results into one buffer and edit them there.
Git             Hunks, blame, log and history as buffers.
Language        Definitions, references, rename, diagnostics, completion — servers you
                already have, found automatically.

Coming from Neovim?   What works, what differs, what is missing ->
How it is built       A Lean-verified edit kernel and a differential harness against Neovim ->
```

Every line there is a promise §7 has to make true; three of them are not true today
(recents first, `u` undoes a replace, edit the hits).

## 7. Roadmap

Four releases, ordered by what a user feels first. Each has one theme and a gate; nothing
from a later release is pulled forward, and nothing else ships between them.

### 0.56.1 — stop claiming (days)

Scope: welcome card line; README logo alt; site items in §6.5; mark 0001 §2.1's table
superseded; `docs/vim-compat.md` gains a **Missing** section generated from a list in the
keymap; fix B1 (selection is row 1 until moved), B3, B7 (an unbound prefix consumes its
next key and says so).

Acceptance:
- Picker test: catalogue streamed in three batches while the query is typed in one burst;
  the selected row is index 0 at every drain; Enter opens the top-ranked path.
- Keymap test: every `ctrl-w {key}` either dispatches a window command or changes
  nothing and reports "ctrl-w {key} is not bound".
- Golden: the welcome card.
- Site check (`tools/check_site.py`) fails on the strings `containers` and `Docker`.

### 0.57 — the keys

Theme: a Neovim user can work for a day without a key doing something else.

Scope: everything in §2's "missing" table except folds and tabs; the §2 "wrong" table;
`Ctrl-W` window commands as vim's (keep `Space w`); `:'<,'>` prefilled from Visual;
`&`/`\1`–`\9`/`~` in `:s` replacement; `:g`/`:v`, `:norm`, `:m`, `:t`, `:b`, `:bn`/`:bp`,
`:ls`, `:reg`, `:marks`, `:wa`, `:wqa`; numbered and append registers. **Owner decision
needed:** case sensitivity. My recommendation: `:s` and `/` follow vim (`noignorecase`)
with `smartcase` as the configured default for `/` only; `:s` must never be
case-insensitive by default.

Also: the `:s` preview renders the changed span in view (scroll the row horizontally to
the first change) and never covers the statusline.

Acceptance:
- The differential corpus grows from 72 to ≥ 600 cases, generated: each operator × each
  motion/object × {no count, count} × {Normal, Visual, dot-repeat}, plus the 151 from this
  critique. Zero unexplained divergences; `KNOWN_DIVERGENCES` ≤ 10, each with a reason.
- `strop compat` prints a Missing section; the test that every dispatchable binding
  renders in `?` also asserts every Missing row does *not* dispatch.
- A register test for `"0`, `"1`–`"9`, `"A`.
- Golden: `:%s/edge/rim/g` at 80×24 shows `Edge` and its replacement in the card.

### 0.58 — find

Theme: the first row is the file you meant, and search is as fast as `rg`.

Scope: basename-over-path scoring and shorter-path tie-break; per-project frecency
(visits + recency, stored with the session) and open buffers first on an empty query;
space-separated AND terms; `Ctrl-U` clears; `Space b` preselects the alternate; parallel
content search with results merged in path order; toggle keys for case, word and regex in
the search field, shown as a status row only when non-default (0088 rule 4); one key each
for hidden and ignored; relative `path:line  text` rows for LSP locations (B8); `:e`
completes relative paths.

Acceptance:
- Ranking tests on a fixed 100K-path corpus (the linux file list, checked in as a
  fixture): `sched.h` → `include/linux/sched.h` first; `sched` → a file named `sched*`
  first; `core.c sched` → `kernel/sched/core.c` first; empty query after opening three
  files → those three, most recent first.
- `strop bench` scenarios with budgets that fail the gate: finder first paint ≤ 20 ms and
  per-key settle ≤ 50 ms at 100K files; content search on the linux fixture first hit
  ≤ 150 ms and complete ≤ 400 ms on 8 threads.
- Golden: locations picker rows.

### 0.59 — change

Theme: every way of changing many places is one surface with one undo.

Scope:
- **Editable multibuffer.** A collection's excerpts write through to their source
  buffers; edits in either are visible in both; `:w` in the collection writes the touched
  files; `u` is the collection's own history. Search hits, references and diagnostics all
  collect into it. Move the excerpt↔source mapping decision into `strop-kernel` over the
  existing `map_through_edits` theorems, with a mutant (ring rule, AGENTS.md).
- **Replace is two steps.** `Space R`, type, Enter → review diff; Enter (or `Space a`
  first row) applies **and writes**; `u` in the review reverts buffers **and files** while
  their revisions still match, and says which files it could not revert. `:apply-change`
  / `:save-change` / `:undo-change` and the receipt buffers are deleted (no aliases).
- **Captures** (`$1` / `\1`) in the With field when the query is `regex:`.
- **Rename** opens the same review for multi-file edits; `grn` prompts with the current
  name; single-buffer renames apply directly and flash.
- **`Space a`** on a source buffer: code actions only. Filesystem verbs live in the
  Directory buffer and `:fs`.

Acceptance:
- Journey (real binary, tempdir): search → collect → `cw` on three excerpts in two files →
  `:w` → both files on disk changed, both source buffers clean, one `u` in the collection
  reverts one edit in its source.
- Journey: replace across 34 files → Enter → Enter → 34 files changed on disk, zero dirty
  buffers; `u` → 34 files restored byte-identical, zero dirty buffers.
- Property test: random edit streams applied through a collection and directly to
  sources converge (the conformance harness in `editor/conformance.rs` already has the
  shape).
- Lean: the excerpt mapping claim in `proofs/Audit.lean` with its mutant.
- `Space a` golden on a source buffer with a diagnostic: quick fixes first, no filesystem
  rows.

### 0.60 — git, and the site that can now be honest

Scope: `Space g g` changed-files buffer (staged / unstaged / untracked; Enter opens,
`s`/`u` stage and unstage a file, `]c` continues into hunks); `Space g c` commit with the
message in a buffer; branch switch picker; the §6 site rewrite with three recorded loops
and the "Coming from Neovim" page generated from `strop compat`.

Acceptance:
- Journey in a temp repo: edit two files, `Space g g`, stage one file and one hunk of the
  other, commit, `git log -1 --stat` matches.
- Site check asserts the hero frames are generated from the release binary and that every
  key named on the landing page dispatches in that binary.

### Do not

- Do not add a confirm step or delay to chords to rescue the motto.
- Do not add another review/receipt surface. Two were one too many.
- Do not start the debugger (0060), tabs, or a plugin runtime before 0.60.
- Do not ship point releases between these; the changelog should read as four entries.

## Appendix — measurements

| Measurement | Value |
|---|---|
| Startup to first paint, small file | 2–4 ms; settled 32–55 ms |
| Key → first output byte, any surface | 0.2–0.7 ms |
| Finder, 97K files: first paint / complete | 8 ms / ~450 ms |
| Finder: key → settled | 9–45 ms |
| Search `kmalloc`, linux, warm: complete | 1,313 ms (TUI); 1.45 s wall / 1.30 s user (headless) |
| Search rare literal: first hit / complete | 837 ms / 1,152 ms |
| `rg -F -i kmalloc`: 32 threads / 1 thread | 0.105 s / 0.856 s |
| First search on a cold tree | ~21 s to settle (disk-bound; not compared) |
| 11.4 MB file: first paint / settled | 13 ms / 414 ms |
| `:%s` preview over 103K lines | 1.14 s, off the input path |
| Differential smoke | 151 cases, 80 differing texts |
| Repo corpus | 72 cases, 1 known divergence |
| Release tarball | 12.2 MB |
