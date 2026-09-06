# Changelog

## 0.11.0 — 2026-09-06

The transaction gateway + verified state machine (plan 0024).

### Added

- **The gateway**: `apply(document, base_revision, changeset)` — typed
  validation (stale revision, readonly, invalid range, missing
  document), one undo unit, and every side effect (history, anchors,
  tree bridge, clock) in one place. Project replace and git discard
  flow through it; the typing path flows through the same commit seam.
- **`specs/EditorProtocol.tla`** — the editor's document/pane/
  transaction/service protocol model-checked by TLC: NoStalePane,
  NoMisapply, NoPartialCommit, monotonic clock. The kept mutant (an
  unguarded Deliver) is caught by the gate — the invariants have teeth.
  In the docker gate (`compose run model`) and CI.
- **The conformance harness** — a String reference model and the real
  editor run the same generated streams through the production path;
  text equality + invariants asserted every step (found vim's
  insert-Esc cursor move and the welcome card's first key on day one).

## 0.10.3 — 2026-09-06

The consolidation round (plan 0023): review 4's 18 contract probes ship
in the repo as the acceptance suite (`editor/contract_probes.rs`) — all
green.

### Fixed

- **Split from scratch no longer strands the pane** (panes rebind before
  the scratch drops).
- **Clipboard replies retain their destination document** — a switch
  mid-read drops the paste with a message instead of inserting into the
  wrong buffer.
- **Saving through a symlink writes through** (vim's rule: the link
  survives).
- **More panes than cells renders** instead of panicking.
- **`df2` deletes through `2`** (a digit after `f` is the target).
- **`:1y` fills the register `p` reads.**
- **Stale replacement ranges over Unicode edits report stale**, never
  panic; search previews never start mid-char.
- **Unicode picker rows match** (the ASCII-only scorer input was library
  misuse); match columns are char indices.
- **Tab glyph and caret read one layout** (config's tab width drives
  both; one LineLayout per line, not per grapheme).

### Changed (architecture)

- **`epoch` is THE text clock** — syntax invalidation, the tree bridge,
  and LSP freshness all read the counter that moves on every mutation,
  including mid-insert-session and through undo. Anchor mapping runs on
  undo/redo too, and its watermark is per-document.
- **Git gutter results carry their document id** — a snapshot can't
  populate the wrong buffer.
- **The release target builds from the test stage** — a tag that fails
  the gate can't ship. The installer's checksum verification is
  mandatory, matching the updater.
- **Preview claims are precise**: composition windows (search, replace,
  git) preview live; instant operator+object chords execute at the
  completing key. The site says so.
- The scorebench example measures the real `Picker::refilter` path.

## 0.10.2 — 2026-09-06

### Fixed

- **A crash (and a release-build garbage-read) in incremental syntax
  highlighting**: the rope-chunk parse callback assumed tree-sitter
  requests bytes monotonically. On error recovery in large
  template-heavy files it backtracks; `byte - offset` underflowed and
  panicked in debug, and could feed garbage slices in release. Repro:
  gd into `/usr/include/c++/13/optional` and back. The callback now
  uses ropey's random-access `chunk_at_byte`. Regression test pins
  backtracking-heavy inputs.

## 0.10.1 — 2026-09-06

The 1.0-hardening remainder (plan 0022): incremental tree-sitter and
the nucleo decision.

### Changed

- **Syntax highlighting parses incrementally**: the highlighter keeps
  the parse tree; every transaction (typing, undo, redo, git discard,
  ex edits) applies its ops as tree-sitter InputEdits at commit time
  — a cheap pointer walk — and the reparse runs from rope chunks
  against the old tree, never a from-scratch parse and never a
  materialized document String. Correctness is property-pinned:
  incremental spans equal a fresh parse across edit scripts.
- **The picker's scorer is nucleo-matcher** (docs/nucleo-decision.md):
  42–50ms → 12ms on a 100k-item refilter, identical hit sets,
  boundary-preferred match columns for the accent render. Adopted on
  numbers, not principle — the bench lives at
  `cargo run -p strop-picker --example scorebench --release`.

## 0.10.0 — 2026-09-06

Document identity and the last 0018 seams (plan 0021). No new
user-visible features — the document model is formalized and the render
path is clean.

### Changed (architecture)

- **The gutter never diffs on the render path**: edits mark the hunk
  snapshot stale; a worker computes against a shared rope clone and
  posts the revision-keyed result. Stale snapshots drop and re-enqueue;
  signs clear honestly for the frame instead of painting wrong lines.
- **Every interactive LSP reply is stale-droppable by construction**:
  goto/locations answers carry the asking document's revision; a reply
  against another edit state never navigates.
- **`Buffer.path` is `PathBuf`** (0020 review): the filesystem model
  isn't UTF-8 — a file with non-UTF-8 bytes in its name opens, edits,
  and round-trips (test-pinned).
- **`DocumentSource`** (File / Scratch / Surface / Output): readonly
  and save refusal derive from the source at construction — no more
  `readonly`/`name`/path conventions kept in sync by callers. The
  document layer splits into `document/{mod,surfaces}.rs`; the git
  surface model lives with the payload, and `:w` on a readonly buffer
  names it.

## 0.9.2 — 2026-09-05

The structural half of plan 0020.

### Changed (architecture)

- **Universal anchor mapping**: every committed transaction emits its
  ops through one ChangeMap applied to marks, jumplists, and every
  OTHER pane's selections of the document (the active pane's cursors
  remain each command's own job; a watermark guarantees an op never
  shifts an anchor twice, even across multi-commit revisions).
- **Project-config trust gate**: a project's `.strop/languages.toml`
  that supplies the server `command`/`args` is executable content —
  attach is refused until `:trust` (once, remembered per project root
  in the state dir). Init-options-only configs (pythonPath-style) stay
  free.

## 0.9.1 — 2026-09-05

The third review round (plan 0020): fourteen contract fixes, each
with its regression test.

### Fixed

- **`:w {path}` honors overwrite policy** — an existing target needs
  `:w!`; identity adopts only after a successful write; a failed write
  changes nothing.
- **Streaming pickers no longer die on the second keystroke** (a 0.9.0
  regression): respawned rg workers forward to the same event source,
  tagged (picker id, query generation); stale streams are dropped.
- **Project replace is byte-exact past multibyte text** (Ropey's
  char-indexed slice was being fed byte offsets).
- **`d/foo<Enter>` works in normal mode** (motion text is literal —
  spaces stay spaces, Enter is `\r`), and search composition previews
  its target as you type, from the typed plan.
- **Syntax highlighting invalidates on the document revision**, not a
  len+first+last-byte hash that kept stale colors on same-length edits.
- **LSP diagnostics compare against the server's own version clock**
  (the buffer's edit epoch was the wrong counter); position encoding is
  read per event, not captured possibly pre-negotiation.
- **Git hunk discard is one committed undo transaction** (`u` restores
  byte-exact), restoring from the hunk's own CRLF/newline-precise lines.
- **Unopened files in project replace become real buffers** — one
  transaction model, one atomic writer, real undo, permissions kept.
- **Grep rows are never fuzzy-filtered by the regex query** — rg's
  matches are the apply set, exactly.
- **UTF-8 boundaries**: charwise paste and repeat-search step whole
  chars; charwise visual spans whole chars (three panic classes closed).
- **A failed `:e` leaves the current document intact** (I/O before any
  ownership change).
- **Resize events reach the event loop**; an idle terminal redraws.
- **Sessions persist for every launch mode**, including `strop file.rs`.
- `strop-picker` is a workspace member.

## 0.9.0 — 2026-09-05

Revision-native git and services (plan 0018): git content is
byte-precise, index mutation is structured, and every async result
wakes the UI.

### Changed (architecture)

- **Byte-precise git**: `DiffLine` holds raw bytes + a newline flag;
  git's "\\ No newline at end of file" marker is data now, never
  text that leaks into a staged blob.
- **Structured index mutation**: staging/unstaging edits the index
  blob directly through libgit2 — no hand-serialized patches, no
  shell-outs, no path-quoting or CRLF hazards. Stage/unstage round-trips
  a missing final newline exactly (new tests pin it).
- **`GitRevision` spans the four states**: Head, Commit, Index,
  Worktree, MergeBase — with byte-level readers; permalinks resolve
  merge-bases.
- **Unified event source**: one app channel; a reader thread translates
  terminal input; every job's results forward the moment they land.
  The 500ms idle poll is gone — hover/diagnostics/shell/git results
  paint immediately.
- **Per-workspace language config**: the process-global OnceLock is
  gone; `languages.toml` merges are cached per workspace root (a second
  project no longer reads the first project's config).
- **Stale-answer guards**: diagnostics carry the server's document
  version (older batches are dropped, not misplaced); a hover that
  arrives after an edit never opens over changed text.

## 0.8.0 — 2026-09-05

Text and terminal correctness (plan 0017): one layout layer owns the
byte↔cell seam, and the editor is honest on real human text.

### Added

- **`LineLayout`** (strop-core): per-visible-line grapheme spans with
  byte↔display-cell maps — the single translation seam every
  visible-line consumer now reads.
- **Visual block mode** (`ctrl-v`): rectangle select on CELL columns
  (wide chars and tabs can't skew the columns apart), `x`/`d`/`y` on
  the rectangle, `c`/`I`/`A` with per-row text replication at Esc.
- **Bracketed paste**: pastes insert as one undo unit of TEXT — no key
  interpretation, no `:` puns; normal-mode paste behaves like `p`.
- **The caret is cell-accurate**: wide chars and tabs place it right,
  and splits place it in the RIGHT pane (the pane origin was never
  applied).

### Fixed

- The paint loop walks graphemes with byte offsets — syntax, search,
  selection, diagnostic, and preview overlays no longer drift after
  the first multibyte char on a line.
- `l`/`h` step char boundaries (a byte step from a multibyte lead
  clamped straight back — the "stuck at é" bug).
- `r` counts characters, not bytes (`2rX` over ü is two chars).
- `*`/`#` classify word chars by Unicode, not ASCII — café, münchen,
  and 変数 are words.

## 0.7.0 — 2026-09-05

One input machine (plan 0016, landed fully). Every normal-mode key
event — letters, arrows, ctrl keys, Enter, Tab — traverses a single
typed state machine whose trie IS the command table; the machine emits
typed actions (a table row with count/register, or a grammar Command)
and never an assembled string.

### Added

- **Macros**: `q{reg}` records, `@{reg}` replays (counts work,
  `@@` repeats) — replay feeds the same machine, so a macro is exactly
  as capable as hands on the keyboard.
- **vim Enter**: `[count]` lines down to the first non-blank (blame
  gutter keeps its dive).
- **`strop --dump-compat`**: the vim-compatibility report, generated
  from the command table; `docs/vim-compat.md` is pinned fresh by a
  test. 106 live bindings listed.

### Changed (architecture)

- The walker is table-driven: prefixes derive from BINDINGS, never a
  hardcoded list; `Command.count` is `Option<usize>` — bare `G` vs `1G`
  is typed now, and the digit-sniffing hack is gone.
- Aliases are semantic: `D`/`C`/`Y`/`S` map to grammar commands with
  the walker's count/register merged — nothing replays through input.
- Model-based tests pin the machine's invariants: every live row
  completes or pends VISIBLY; junk never poisons the next count; counts
  cap under adversarial input.
- The dead string-token dispatch path is deleted (clean cutover).

## 0.6.1 — 2026-09-05

The trust release (plan 0015): the second review round's destructive
paths close, and the vim canon fills in.

### Fixed (safety)

- **ctrl-c is a quit intent, not a hidden force-quit** — dirty work
  warns once ("unsaved — ctrl-c again to force-quit"); a second press
  exits. No exit path silently discards buffers anymore.
- **A pathless `:w` errors** ("no file name — :w {path}") instead of
  reporting "written"; `:wq` on a dirty scratch stays open; `:w {file}`
  names and adopts a path.
- **A failed shell filter never touches the source** — `| false` used
  to erase the selection; now stderr explains itself in the statusline
  and the text stands.
- **`:wq` closes the window, not the shared document** — the
  stale-DocumentId panic path with one document in two panes is gone.
- **Session undo histories carry a content identity** — a history
  captured against dirty text never replays onto changed disk content.
- **`didClose` completes the LSP lifecycle** — close/reopen in one
  process no longer leaves the server holding stale text.
- **`Buffer::byte()`** no longer panics on an empty rope.

### Fixed (vim canon)

- `d0` is a motion again (contextual zero); counts cap at 99 999
  instead of wrapping; arrows ride the walker (`2 <Right> x` moves
  twice, deletes one); aliases keep their counts (`2D` ≡ `2d$`).
- Edit cascades no longer stack a phantom second cursor on the
  primary's landing.
- The pending-keys display shows the full structural state
  (`"a2d3`), never a blank.

### Added

- **Viewport canon**: `ctrl-d/u/f/b` (a count is the scroll size),
  `zz/zt/zb`, `H/M/L` (counts are line offsets), `ZZ`, `ctrl-^`
  alternate buffer, `gv`, `gi`, and `g;`/`g,` over a changelist
  derived from the undo history.
- **Grammar motions**: `{`/`}` paragraphs and `ge`/`gE` — they compose
  with operators and preview like every other motion.
- **Ex ranges**: `:N` goto, `%`/`N,M` with +/- offsets, ranged `:d`
  (yanks) and `:y`, and literal `:[range]s/a/b/[g]` as one undo unit.
- **LSP navigation**: `gr` references, `gI` implementation, `gy` type
  definition, `gD` declaration — many results land in a picker with
  previews, one jumps directly; pre-init requests queue and flush.
  `]d`/`[d` traverse diagnostics with the message in the statusline.
- **Commit-viewer file sidebar is a tree**: directories group as dim
  header rows, files indent by depth.

## 0.6.0 — 2026-09-05

The input layer lands (plan 0008, fully): key events walk a typed
state machine, and the doc table IS the dispatch table.

### Changed (architecture)

- **The input walker** (`editor/input.rs`): counts, registers,
  operators, and prefixes are typed parser state — pending strings
  survive only as free-text line content (`:` `/` `?` `|`, the modal
  text fields of 0003 §1) and as the grammar's assembly interchange.
- **One command table**: every BINDINGS row carries a stable `id` plus
  its dispatch handler — `Space ?`, which-key cards, and dispatch read
  the same rows. Parameterized rows (`r<c>`, `m<a>`) match by
  placeholder; the coverage tests tokenize exactly like dispatch.
- **Panes own their selections and scroll** (the ViewModel split):
  `Pane { doc, sels, view_top }` — switching panes never copies state
  back and forth; the active pane's state IS the editor's.

### Fixed

- Found by the new dispatch-coverage tests: `,` didn't reverse the
  find (shared row handlers ignored the completing key — `;`/`N`/`#`/
  `[c`/`P` all key-aware now); `ctrl-w` pane keys route through the
  walker; counts reach leaf commands through the table (2x, 3rx, 2p).

## 0.5.0 — 2026-09-05

The restructure release. All four waves of `plans/0014` landed: the
editor's centre is identity-typed and selection-unified, the input layer
is becoming data, and git is a data model, not a sidebar.

### Changed (architecture)

- **Generational document ids** (`strop_core::id::Arena`): buffers,
  panes, marks, jumplist, surfaces, hunk origins, picker payloads all
  hold stable `DocumentId`s — closing a buffer can never alias another.
  The parallel-vectors alignment invariant is one `Document` struct now.
- **One selection model** (`strop_core::selection::SelectionSet`):
  normal = collapsed primary, visual = stretched primary, multicursor =
  extras. The three old fields could disagree; the set owns the
  invariants.
- **`MotionShape`** replaces `Range { linewise: bool }`:
  `Characterwise { inclusive }` carries what dfx-vs-dtx need; Blockwise
  is the enum's extension point for visual block.
- **LSP server pool**: one client per (workspace root, server) — Rust +
  Python + C++ in one session, each with its negotiated encoding.
- **Leaf commands are data** (`editor/registry.rs`, plan 0008 stage 1):
  44 normal-mode leaves dispatch from a static table; a parity test
  fuses dispatch and docs. Caught a dormant bug on day one: `W`/`B`/`E`
  never moved the cursor (the grammar's cursor_after lacked BigWord
  arms).
- **`editor/mod.rs` split** (1817 → ~400 lines): document, cursor,
  diagnostics, registers modules; tests live beside the behavior they
  pin. git_memory split into dive/blame/permalink.

### Fixed

- **Paste lands the cursor on the last pasted char** (vim), found by the
  new differential harness.
- **`f`/`t` find multibyte chars** (`f é` works; found bytes before).
  All dynamic command args are `char`, never `u8`.

### Added

- **The nvim differential harness** (0006 tier 1): 40 cases drive strop
  headless and pinned nvim over the same keys; text + cursor must agree.
  Runs in the docker gate.
- **Semantic dot-repeat**: `.` re-resolves the parsed command from the
  current position instead of replaying a key string.
- **ActionPlan**: `strop_grammar::plan(buf, cursors, cmd)` is the single
  object preview renders and execute applies — multicursor previews
  highlight every cursor's target now.
- **Git as four states**: HEAD → index → worktree → live document are
  separate diffable edges. The gutter shows staged lines in the
  committed-adjacent tint; `Space g s` stages (live→index), `Space g S`
  unstages (index→HEAD), `Space g u` discards unstaged (restore from
  index, not HEAD).
- **Selection history**: visual mode `Space g h` = `git log -L` on the
  selected lines. Permalinks from commit surfaces pin that commit.

## 0.4.0 — 2026-09-04

The safety release. An external architecture review (now `plans/0014`)
audited the mutation and protocol boundaries; every confirmed P0 is
fixed and pinned by a failing-first test.

### Fixed — safety

- **`:wq` could close after a failed save** — disk error, full
  filesystem, or an externally modified file meant silent data loss.
  A failed save now keeps the buffer open and dirty; `:w!`/`:wq!`
  force past an external change.
- **Saves are atomic** (temp file + rename in the same directory,
  permissions preserved) and refuse to overwrite a file that changed
  on disk since load.
- **Readonly is enforced at the mutation boundary**, not by caller
  discipline: `Buffer::insert`/`delete` refuse on readonly buffers;
  job-owned surfaces refresh through an explicit `replace_all_system`.
- **Staging a hunk no longer silently saves the buffer** (writing every
  unrelated unsaved edit to the worktree): dirty buffer →
  "unsaved changes — :w first".
- **Sessions persist on exit**, not only on `:w`.
- **rg failures surface**: a bad `-t`/`--glob` filter posts rg's stderr
  to the picker instead of looking like "no matches".

### Fixed — protocol

- **LSP document versions** strictly increase per document (was: `2`
  forever — pyright rejects stale versions).
- **LSP position encoding** is negotiated (utf-8 offered, utf-16
  honored, the spec default) with one tested conversion module —
  hover/goto/diagnostics land on the right column with emoji,
  combining marks, and astral-plane chars before the point.

### Changed (breaking)

- **`|` is vim's column motion again**; pipe-through-shell moved to
  `Space |` (normal and visual). Restored before the deviation became
  muscle memory — see `plans/0014` §"compatibility policy".

## 0.3.9 — 2026-09-04

Headless QA sweep (the harness now drives the real binary across
60+ scenarios); every crash class found is fixed and pinned by a test.

### Fixed

- **Unicode crash**: word motions classified *bytes*, not chars — `w`
  could park the cursor inside a multibyte char and the next `x`
  panicked ropey. Word classes are char-aware now (é is a word char,
  🦀 isn't), and `x`/`a`/`~` round to char boundaries. Root cause was
  deeper: ropey's `try_byte_to_char` maps mid-char bytes silently, so
  the old `clamp_boundary` never clamped anything — it now verifies via
  the byte↔char roundtrip.
- **Arrow keys did nothing anywhere**: the crossterm translation dropped
  `KeyCode::Up/Down` at the catch-all. Arrows now speak hjkl in normal,
  visual, insert, and on read-only git surfaces; in pickers Up/Down walk
  results and Left/Right move the caret.
- **Picker navigation**: after Esc (normal mode on the field) `j`/`k`
  walk the results instead of editing the query; the prompt glyph shows
  the mode (`❯` insert, `▮` normal).
- **`0` went to line-start nowhere** — the count parser ate the bare
  zero (vim: leading `0` is a motion, later digits are counts).
- **Empty `/` / `?`** repeat the last search (vim) instead of erroring
  with a raw `\r` in the message.
- **Flaky pane tests**: parallel tests shared `/tmp` fixture files;
  fixtures are per-test tempdirs now.

### Added

- **`gs` switches source ↔ header** via clangd's
  `textDocument/switchSourceHeader` — the C/C++ header jump. No server
  or no counterpart says so in the modeline, never silent.

## 0.3.8 — 2026-09-04

### Fixed

- **The lingering welcome screen**: the untouched initial scratch
  buffer is replaced by the first real thing you open (vim's
  `[No Name]` rule) — no more welcome card reappearing mid-quit, no
  keep-quitting to exit.

### Added

- **Readonly mode** for real: `strop -R file` (vim's `-R`), `:view`
  marks the current buffer readonly, `:view file` opens readonly.
  Modeline shows `[RO]`. (Research note: no editor — vim, nvim, helix —
  opens goto-definition targets readonly by default; editing the
  destination is intentional. `:view`/`-R` is the standard answer.)
- **The modeline grows up**: mode chip · git branch (`*` marks a dirty
  worktree) · file · `[RO]` · cursor count (2×) · error/warning chips ·
  line:col · percent.

## 0.3.7 — 2026-09-04

### Added

- **Jumplist** (vim's ctrl-o / ctrl-i — and ctrl-i is Tab in a
  terminal, same as vim): jumps record on `gg`/`G`/`%`, searches, `n`/`N`,
  marks, buffer switches, `:N`, and every surface dive; ctrl-o walks
  back across buffers, ctrl-i/Tab forward; a new jump truncates the
  forward path. On commit-diff surfaces Tab keeps its sidebar-focus job.

### Website

- `what's new` links removed (the changelog page serves it);
  docs/changelog content no longer hides under the fixed topbar;
  footers unified with the homepage's.

## 0.3.6 — 2026-09-04

Input boxes get modal, replace gets vscode-grade exclusion, and the LSP
wire learns its manners.

### Fixed

- **didOpen raced initialize** (found while verifying a pyright
  override): the first document opened could hit the wire before
  initialize completed — rust-analyzer tolerates it, strict servers
  (pyright) drop the document. Opens now queue until Initialized; the
  wire reads initialize → initialized → didOpen, textbook.

### Changed

- **Grep is a popup card again**; global **replace keeps the full
  frame** — the quick-lookup and the power-tool get different shapes.

### Added

- **Modal input boxes** (rootle's): picker fields and the `:` `/` `|`
  line land in insert mode; Esc enters vim normal mode *on the field*
  (h/l/0/$/w/b move, x/X delete, i/a/A return), the cursor changes
  shape, Esc again closes. One shared `LineEdit`, no forks.
- **Replace exclusion per file**: ctrl-x excludes a match, ctrl-d
  toggles the row's whole file (vscode's file toggle); the count chip
  counts both; the hint line documents the `-t rs` / `--glob` filters.
- **strop.dev/docs + strop.dev/changelog**: how to configure
  `languages.toml` (project `.strop/` over XDG, helix-shaped, the
  pyright venv+extraPaths recipe) and `config.toml`, plus the full
  per-release changelog.

## 0.3.5 — 2026-09-04

The first-external-review release: a week of real use, seven confirmed
bugs, all fixed.

### Fixed

- **Counts on non-operator commands** never abort anymore: `2x`, `3p`,
  `2u`, `4.`, `3rx`, counted inserts (`3iX` types X three times, `2o`
  repeats the opened line) — vim's multiply rule. (Highest-ranked by
  the reviewer: silent keystroke loss is the worst kind.)
- **`{count}G`** jumps to that line instead of the end of file; bare
  `G` still goes to the end. The two used to disagree silently.
- **`cc`/`S`** clear the line's content and keep the newline + indent
  — no more merging with the next line; counted `2cc` collapses N
  lines into one fresh line (vim semantics).
- **`C`** enters insert at the deletion start, not one column early
  (typed text used to land before the last surviving char).
- **Permalinks with ssh host aliases** (`bbgithub:org/repo.git`)
  resolve via `~/.ssh/config` Host blocks, with a bare-host fallback.
- **Permalink errors say what happened**: not a repo / no remote
  configured / `unsupported remote URL: <it>` — the old
  `no remote / not a repo` conflated all three.
- **`--headless` with a missing script** prints a clean error (exit 2)
  instead of panicking.

### Added

- **`^`** (first non-blank, works with operators: `d^`, `y^`), **`I`**
  (insert there), **`~`** (toggle case, advances), **`S`** (= `cc`).
- **Unknown bare keys are loud**: `not an editor command: <key>` —
  silent swallowing made absent features indistinguishable from broken
  ones.
- **Release attestation**: every release tarball now gets signed
  build provenance (`gh attestation verify <file> -R stropdev/strop`),
  ported from gripsack's pipeline.

### Notes

- The reviewer's https-enterprise permalink failure (their row 5) does
  not reproduce on 0.3.4 — verified with the exact release artifact and
  their exact URL shape. If it still fails for them, the actual remote
  string is the next datum needed.

## 0.3.4 — 2026-09-04

Crash fix and a vim-gap sweep.

### Fixed

- **Replace-picker crash** (`index out of bounds, len 0`): typing in
  `Space R` respawns rg per keystroke; the respawn cleared items but
  not rows, and the renderer indexed a stale row. Rows clear with the
  respawn now, plus defensive lookups (regression test included).
- **Sidebar `│` alignment**: non-current file rows were one column
  wider than the current row — the divider wobbled in and out.
- **`:30`** jumps to a line (clamps to the last content line, never the
  phantom past a trailing newline); **`:noh`** clears the persistent
  search highlight.
- **`30j`-style counts**: `0` after a count digit is a digit, not the
  line-start motion — counts above 9 work now.
- **Visual `<` / `>`** indent and dedent the selection (one undo unit).
- **Visual pending no longer swallows keys**: invalid sequences clear
  with a message instead of accumulating forever (which also made later
  keys vanish).

### Verified

- `.` repeats deletes and change+insert (dot-repeat tests pinned).

## 0.3.3 — 2026-09-04

The git-flow and command-line round, plus a vim-fidelity sweep.

### Added

- **Tab focus-cycle in commit diffs** (tuicr's model): Tab/Shift-Tab
  hops between the file sidebar and the diff content; focused j/k steps
  files in place; the current file wears rootle's `▸` when focused.
- **Syntax highlighting in diff views**: commit deltas highlight code
  under the origin tint (delta's look); the highlighter follows `]f`
  file steps.
- **`:` autocomplete**: ex candidates render under the command card
  with doc strings; Tab cycles them.
- **`:!cmd`**: run a shell command in a job, output opens as a real
  readonly buffer (search it, yank it, q closes).
- **`|cmd` (helix's pipe)**: visual selections pipe through a command
  and get replaced by stdout (one undo unit, never-clobber verified);
  bare `|cmd` pipes the current line.
- **`*` / `#`**: whole-word search for the word under the cursor,
  wrapping; `;` / `,` repeat f/F/t/T finds (`, ` inverts).
- **Persistent search highlight**: matches stay lit after the search
  commits (rootle rule); the current match is underlined. Works on
  readonly surfaces too, with n/N.

### Fixed

- **Undo**: a lone paste never committed its revision — `u` after
  yank+paste claimed "already at oldest change". Every command now
  commits exactly one undo unit (nvim's rule).
- **Ctrl-W on some terminals**: keys that arrive as raw control bytes
  (`\x12`/`\x17`/`\x18`/`\x03`) now map to Ctrl-R/W/X/C — C-w pane
  cycling works on Windows Terminal → WSL.

## 0.3.2 — 2026-09-04

The finish-line polish round: dots, gaps, hues, and a diff that reads
like delta's.

### Added

- **Intra-line emphasis in diffs** (delta-style, two-tier): del/add
  runs pair line-by-line within a hunk, shared prefix/suffix trims, and
  the changed span gets a brighter background + bold over the row tint.
  Pure adds/deletes keep the quiet full-row tint.
- **Blame gaps** (rootle rule): a commit's gutter cell prints only on
  the first line of its run — the blame column breathes.
- **Diagnostics**: severity `●` dot in the gutter (color reads faster
  than letters), colored underline on the offending span
  (`underline_color`; wavy undercurl lands when ratatui bumps to 0.30),
  multiline messages join with ` · ` in the EOL note.
- **`:help` palette**: section-hued key columns (blue normal, purple
  visual, green insert, amber leader, cyan git, yellow ex), section
  headers with a trailing rule, bold keys.

### Fixed

- **Demo hiccup**: the multicursor tape section typed `strop
  proj/demo.rs` into the still-open editor (the splits `:q` closes a
  pane, not the app) and mangled line 1. The tape continues in the
  existing buffer now.

### Decided (research)

- Side-by-side diff (JetBrains-style) is rejected for now — it pairs
  two logical rows into one display row, breaking the surface-as-buffer
  contract (cursor/search/yank mirror). git-delta and tuicr made the
  same call; the run pairing built for emphasis is the alignment engine
  if a split view ever lands (0010 amendment).

## 0.3.1 — 2026-09-04

Polish round: the demo is clean again, help wears color, and the E now
tells you what it is.

### Added

- **Diagnostics UX**: the cursor line shows its worst diagnostic as an
  end-of-line note (severity-colored, italic, scoped to one line — no
  inline-hints machinery); severity colors unified across the gutter
  and the note.
- **`:help` decoration**: section headers and key columns in accent,
  planned `(soon)` rows muted.
- Seeded key-soup fuzz test (12k keystrokes across buffer shapes + a
  frame render per shape) — it pays rent immediately (see fixes).

### Fixed

- **async-lsp panic in the demo** ("Sender is alive"): quitting dropped
  the client socket while the server mainloop was still running. The
  runtime thread is now joined on quit (with a leak-not-panic timeout
  fallback).
- **Byte/char units in rope mutations** (found by the fuzz): `insert`,
  `delete`, and history replay passed byte offsets to ropey's char-index
  APIs — any multibyte content (our own undo-tree buffer uses ↵/←/⑂)
  could panic. `delete` also clamps stale ranges instead of panicking.
- **Paste mutated readonly buffers** (found by the fuzz): `p` on a
  git surface or the undo tree now refuses like every other edit.
- Multiline diagnostic messages join with ` · ` in the EOL note.

## 0.3.0 — 2026-09-04

Multicursor lands, help becomes a buffer, the demo's LSP section is
real again.

### Added

- **Multicursor** (plan 0013, nvim-0.13 interaction over a cascade
  executor): `Q` toggles a cursor at point, `Space c` stacks one onto
  the next line (helix's `C`). Motions, `n`/`N`, operators, yank, paste,
  and insert mode all cascade — deletes apply bottom-up, mirrored edits
  shift-remap, stacked cursors edit once. Normal-mode Esc collapses to
  the primary cursor; `u` reverts a whole cascade as one unit. Secondary
  cursors render as solid blocks. v1 deferrals (visual-mode multi-range,
  mouse placement, select-next-match) are documented in 0013.
- **`:help`** — the keybinding table as a real readonly buffer: `/`
  searches it, motions walk it, `q` closes. `Space ?` opens the same.
  The floating keybinds popup is gone (a buffer you can search beats a
  card you can only scroll).
- Headless `state` now reports `message` and `extra_cursors`.

### Fixed

- **Demo LSP section**: the vhs image now installs a rustup toolchain
  with the `rust-analyzer` + `rust-src` components (apt's rust lacks
  rust-src; the standalone binary half-worked and then errored).
- **Graceful LSP shutdown**: quit sends the `shutdown`/`exit` sequence —
  every session used to end with the server dying "client exited
  without proper shutdown" and a fake failure on the statusline.
- **vim fidelity: `cw`/`cW`** resolve like `ce`/`cE` — the trailing
  whitespace is no longer eaten, and at a word's last char only that
  word changes (pinned in the grammar contract tests).
- Commit-diff sidebar width fits the file list (clamped 12–24) instead
  of a fixed 28 columns.

## 0.2.2 — 2026-09-03

Hardening + daily-driver release. The crash-on-quit class is dead, the
editor opens directories, the system clipboard works, and the git
surfaces got the rootle-grade navigation treatment.

### Fixed (trust)

- **Quit crash**: `:q`/`:q!`/`:wq` on the last buffer with an LSP
  attached panicked in the post-feed drain (`lsp_sync_changed` indexed
  an emptied buffer list). The TUI breaks on `should_quit` before the
  drains now, and the sync is empty-list-safe. This was visible in the
  demo tape.
- Panic hook restores the terminal (raw mode + alt screen) on any crash.
- Hunk-restore underflows at file top (`Space g u` on a hunk at line 0).
- Undo cursor lands at the start of the undone change (vim semantics),
  not the tail of the replayed op list; redo distinguishes insert/delete
  placement. History replay clamps both bounds to char boundaries.
- `n`/`N` actually work now — the keymap advertised search repeat with
  no dispatch behind it (found by the new coverage test).

### Added

- **Directory open**: `strop dir/` cds and lands on the file picker
  (helix's `hx .`) instead of dying on EISDIR.
- **System clipboard**: `Space y` / `Space p` / `Space P` (helix-style)
  on top of vim's `"+` register — yank stages OSC52 (works over ssh),
  paste reads via wl-paste/xclip/xsel/pbpaste off the input path.
- **Global replace** (plan 0007): `Space R` — two fields, live
  replacement preview per row, `ctrl-x` excludes a match, Enter applies
  with one undo revision per buffer, span-verified and mtime-guarded,
  atomic file writes. Grep queries take rg filters (`-t rs`,
  `--glob '!target/*'`). Grep and replace render full-frame.
- **Undo-tree browser**: `Space u` — the revision tree as a real
  readonly buffer; Enter restores any revision (branches included), q
  closes.
- **Blame gutter** (0011): `Space g b` toggles a left-margin blame
  column (`sha · author · age`); Enter on a line dives into that line's
  commit. The blame card stays as the loading fallback.
- **Commit diff sidebar**: a commit's file delta shows the changed-files
  list in a left sidebar (tuicr-style); `]f` / `[f` step through the
  commit's files in place.
- **Surface stack** (0011): q/close on any git surface restores the
  origin buffer unconditionally, works in splits, and stale job results
  are generation-guarded.
- **Project LSP config** (0012): `.strop/languages.toml` over XDG
  `languages.toml` over the embedded registry — helix-flavored
  `[language-server.NAME.config]` passthrough (pyright `extraPaths`
  works), absolute commands skip the PATH probe, server capabilities now
  gate hover/goto.
- **Syntax**: fish, lua, sql grammars + vendored highlight queries; cpp
  uses the vendored query now; extensionless shell scripts resolve by
  basename (`.bashrc`, `PKGBUILD`) and shebang.
- **Commit graph lanes**: the log surface colors each graph lane
  distinctly, nodes bold in their lane color.

### Changed

- The keybinds popup (`Space ?`) and all which-key cards render from the
  one `keymap.rs` table; a coverage test pins every dispatchable
  sequence to a row (0003 §5.7). "(soon)" rows render muted as planned,
  never as live.
- Picker previews read files on worker threads — no more blocking IO in
  the render path (0001 §3).

### Plans

- New: 0011 surface stack, 0012 project config, 0013 multicursor
  (nvim-0.13 interaction over helix machinery — the next big rock).
- Amended: 0005/0009 (config filename reconciliation), 0007 (status +
  form-factor notes).
