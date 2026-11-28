---
rfc: "2026-11-28-measured-performance"
title: "Close the measured gaps to Helix and Neovim: allocator, structural indent, in-buffer search, finder, status, large files"
status: "accepted"
implementation: "in-progress"
authors: ["Fable"]
created: "2026-11-28"
updated: "2026-11-28"
issue: "https://github.com/stropdev/strop/issues/307"
milestone: "v0.79.0"
supersedes: []
superseded_by: []
blocked_on: ["Read status through libgit2 or through git status --porcelain=v2?", "Accept the v0.79.0 amendment: kernel viewport and rank modules before D3 and D5, D8, and the D4 engine swap with its listed divergences?", "Apply the proposed AGENTS.md doctrine 1 wording with the D4 engine swap?"]
---

# RFC: Close the measured gaps to Helix and Neovim

## Problem and observable outcome

Tracking issue: [#307](https://github.com/stropdev/strop/issues/307).

strop 0.77.0 was measured against Helix 25.07.1 and Neovim 0.12.6 on the same machine, driving each through a real pty. strop is ahead on startup, opening files, long lines and project search. It is behind in five places a user feels, and one of them breaks the input contract outright:

1. **Every keystroke costs 2–3 ms in the shipped binary and 1 ms in the same code built against glibc.** The static musl allocator is the difference.
2. **`o`, `O` and `Enter` wait on a second full parse and a walk of the whole syntax tree.** The first `o` in a 4.5 MB file takes 3.5 s; in a 15 MB file it had not happened after 12 s. Keys typed meanwhile are queued, so nothing is misread, but the editor does not answer.
3. **A lower-case `/` search runs at roughly 3–10 MB/s.** Finding the last line of a 500 MB file takes 55 s; Helix shows it while the pattern is still being typed.
4. **The file finder rescans and re-ranks every path on every key** (22 ms per key on 97k paths; Helix 7 ms) and takes 1.1 s to list the tree (Helix 0.64 s).
5. **The status view scans the working tree twice with libgit2** (0.92 s on the Linux tree; `git status` 0.13 s).

The outcome proposed: each of these gets a budget in `strop bench`, measured on the musl artifact, and the fix that meets it.

### Method

- Host: WSL2, 32 threads, 30 GB. Quiet machine for the final runs.
- Binaries: the public `strop-0.77.0-x86_64-unknown-linux-musl` tarball; Helix 25.07.1 with its default configuration; Neovim 0.12.6 `--clean`. Each in an empty `HOME`, with a `PATH` holding only `git`, so no language server starts.
- A pty harness (160×50) answers the terminal queries editors send at startup, writes one key, and timestamps output at `read()`. *First byte* is key written → first output byte. *Burst end* is the last byte before 30 ms of silence. These measure the editor, not a terminal's paint.
- Fixtures: `small.rs` 20 KB; `medium.rs` 300 KB / 10k lines; `big.rs` 15 MB / 500k lines of nested Rust; `longline.json` 10 MB on one line; `huge.log` 500 MB / 4.3M lines; the Linux tree (97,387 files, shallow clone); git.git (83,155 commits).
- Medians unless noted. n is 200 keys for typing, 5–15 runs for timings, 3 for the 500 MB file.
- Not measured: LSP round trips, SSH, macOS, cold disk cache, terminal paint time, energy.

### Numbers

Bold marks where strop is behind the better of the two.

| Scenario | strop 0.77.0 | Helix 25.07.1 | Neovim 0.12.6 |
|---|---|---|---|
| **Startup, content visible (ms)** | | | |
| no file | 3.3 | 19.9 | 7.5 |
| `small.rs` | 17.9 | 192 | 39.7 |
| `big.rs` 15 MB | 48 | 651 | 185 |
| `longline.json` 10 MB | 92 | 1,203 | 575 |
| `huge.log` 500 MB | **1,411** | 1,075 | 599 |
| **Resident memory after open (MB, process tree)** | | | |
| no file | 10 | 18 | 20 |
| `small.rs` | 16 | 26 | 22 |
| `big.rs` 15 MB | 163 | 167 | 40 |
| `longline.json` 10 MB | **449** | 187 | 60 |
| `huge.log` 500 MB (peak) | 663 (**1,100**) | 729 (729) | 561 (561) |
| **Insert a character, first byte (ms, p50 / p95)** | | | |
| `small.rs` | **2.8 / 6.8** | 4.5 / 9.4 | 0.8 / 1.4 |
| `medium.rs` | **2.0 / 3.3** | 41.5 / 60.6 | 0.8 / 6.3 |
| `big.rs`, middle | **2.3 / 4.2** | 0.7 / 4.1 | 0.7 / 1.1 |
| `huge.log`, middle | **1.4 / 3.7** | 1.0 / 1.5 | 0.5 / 0.8 |
| `longline.json`, start of line | 2.3 | 571 | 334 |
| `longline.json`, end of line | 1.9 | 2,031 | 228 |
| 300 cursors, `medium.rs` | 18.9 | 45.0 | — |
| `x` / `u` in `big.rs` | **2.0 / 5.0** | 0.8 / 0.6 | 0.3 / 0.3 |
| **`o` until Insert mode shows (ms): first press / later press** | | | |
| `small.rs` | **15.6 / 12.0** | 19.0 / 11.9 | 3.0 / 0.5 |
| `medium.rs` 300 KB | **70.7 / 32.6** | 87.9 / 45.0 | 2.4 / 0.3 |
| 1.5 MB Rust | **219 / 25** | 375 / — | 1.6 / 1.4 |
| 4.5 MB Rust | **3,496 / 77** | 11.7 / 4.2 | 2.1 / 0.3 |
| `big.rs` 15 MB | **not within 12 s** | not measured | not measured |
| **`/pattern` whose only match is the last line: first key → match on screen (ms; the pattern takes ~480 ms to type)** | | | |
| `medium.rs` | 475 | 349 | 349 |
| `big.rs` 15 MB | **5,906** | 440 | 449 |
| `huge.log` 500 MB | **55,405** | 422 | 792 |
| **File finder, Linux tree** | | | |
| open → first rows (ms) | 12.9 | 35.9 | — |
| open → list complete (ms) | **1,115** | 638 | — |
| per key while filtering (ms, p50) | **21.8** | 7.3 | — |
| open and type `schedcore` → `kernel/sched/core.c` shown (ms) | **873** | 429 | — |
| memory with the list loaded (MB) | 55 | 81 | — |
| **Project search, Linux tree** (`rg` alone: 146 ms) | | | |
| rare literal: first hit / settled (ms) | 54 / 127 | 488 / 876 | — |
| common literal: first hit / settled (ms) | 19 / 1,082 | 355 / 1,016 | — |
| **Git** (reference: the `git` CLI) | | | |
| log, first rows, first ever press | 483 | — | `git log -n 200`: 8 |
| log, first rows, later sessions | 19 | — | |
| file history `Makefile`, first rows, first / later | 441 / 30 | — | `git log -- Makefile`: 548 |
| blame gutter, 4.1k-line file | 812 | — | `git blame`: 823 |
| status, clean Linux tree, first rows / settled | **498 / 924** | — | `git status`: 125 |

Helix's slow rows (typing in the 300 KB file, long lines, `o` in mid-sized files) are its own; they are recorded, not relied on. Both strop and Helix highlight the 4.5 MB file; neither highlights the 15 MB one, where Helix therefore types at Neovim's speed.

The built-in `strop bench all` on the musl artifact agrees where it overlaps (`input_frame` key+frame p50 0.87 ms at 120×40; `nav_100k /needle` 5,950 ms; `find key_list` 7.9 ms on synthetic paths).

### The allocator experiment

A throwaway build of the same commit with `mimalloc` as the global allocator, built by the unchanged Dockerfile `release` stage (static check included):

| | musl malloc (shipped) | musl + mimalloc | glibc host build |
|---|---|---|---|
| insert, `small.rs`, first byte p50 / p95 (ms) | 2.77 / 6.81 | 1.00 / 1.54 | 1.04 / 1.60 |
| insert, `medium.rs` | 2.01 / 3.26 | 0.98 / 4.73 | 1.01 / 4.02 |
| insert, `big.rs` | 2.28 / 4.23 | 1.00 / 1.66 | 0.94 / 4.02 |
| insert, `huge.log` | 1.43 / 3.71 | 1.16 / 1.77 | 1.04 / 1.44 |
| finder, Linux tree: open → list complete (ms) | 1,115 | 329 | — |
| finder: open and type → top hit (ms) | 873 | 338 | — |
| finder: per key while filtering, p50 (ms) | 21.8 | 16.3 | — |
| open `small.rs`, content visible (ms) | 17.9 | 6.8 | — |
| open `huge.log` 500 MB (ms) | 1,411 | 1,039 | — |
| first `o`, 300 KB / 1.5 MB (ms) | 70.7 / 219 | 54.1 / 162 | — |
| `strop bench`: `input_frame key+frame` p50 (ms) | 0.87 | 0.41 | — |
| `strop bench`: `find catalog` p50 (ms) | 579 | 139 | — |
| `strop bench`: `search complete` p50 (ms) | 53.7 | 15.3 | — |
| resident memory, no file / 20 KB / 15 MB file (MB) | 10 / 16 / 163 | 24 / 35 / 214 | — |

Of the 42 `strop bench` scenarios with a p50 of at least 0.5 ms, 31 are 1.25× faster or more with mimalloc. Three are slower in a single run and need explaining before the change lands: `picker_100k stream+settle` 6.3 → 12.2 ms, `replace_project save+settle` 136 → 237 ms, `symbols build` 52 → 78 ms.

Binary size: 54.18 MB → 54.43 MB.

## Proof and test obligations

Nothing in this RFC changes a kernel decision, and no proof is weakened. (True of D1, D2, D4 and D7 as shipped in v0.78.0. [The v0.79.0 amendment](#amendment-for-v0790-2026-11-28-frames-lists-themes-the-test-suite-and-the-search-engine) adds kernel obligations for D3, D5 and D8.)

- **Caches reuse the proved admission.** Where a fix keeps derived state (a shared syntax tree, ranked survivors, a status snapshot), freshness is decided by the existing kernel functions: `cache::find`/`place`/`retire` for slots and `admission::admit` for answers. A new cache that needs a different key is a kernel change and gets its own theorem; none is proposed here.
- **Budgets are tests.** Each fix adds or tightens a `strop bench` scenario with a budget, run on the musl artifact by release qualification (the existing rule for `find`, `search`, `symbols`, `history`). Proposed budgets are in each issue.
- **Behaviour is unchanged and stays under the differential harness.** Search results, indentation and finder ranking are observable: the Neovim corpus, the indentation goldens and the ranking oracles must be byte-identical before and after. A faster path that changes a result is a bug.
- **Structural indent while the tree is not ready** is the one place a contract is stated anew: see D2.

## Decision and alternatives

### D1. Use mimalloc in the shipped binary — [#308](https://github.com/stropdev/strop/issues/308)

`mimalloc = { version = "0.1", default-features = false }` in `crates/strop`, one `#[global_allocator]` line in `main.rs`. It is C built by `cc`, which the tree already needs for libgit2 and the tree-sitter grammars; the result is still one static binary with no `NEEDED` entries.

Why not only "allocate less": the per-frame churn is real (D3 removes most of it) but the allocator also sits under ropey, tree-sitter, libgit2 and every worker. One line moves the whole product to parity with a glibc build; D3 then moves both.

Rejected: jemalloc (needs make and autoconf in the build image); shipping a glibc binary (gives up the static artifact); a pure-Rust allocator (none with a comparable record).

### D2. Structural indent reads the document's one syntax tree, near the cursor — [#309](https://github.com/stropdev/strop/issues/309)

Today ([`resolution/indentation.rs`](../../crates/strop-engine/src/editor/resolution/indentation.rs), [`strop-syntax/src/indentation.rs`](../../crates/strop-syntax/src/indentation.rs)):

- past the first 4,096 bytes every `o`, `O`, `Enter`, closing bracket and label colon is deferred to the grammar-resolution worker;
- that worker keeps its **own** `Highlighter` per document, so the file is parsed a second time, from scratch, on the first such key;
- `indentation_contexts` then walks the tree **from the root to the cursor**, visiting every node before it, with about ten substring tests on the node kind per node.

Decision:

1. One tree per document. The analysis owner already keeps an incrementally reparsed tree for highlighting; it publishes `(revision, Tree)` (a `tree_sitter::Tree` clone is a reference count). The resolution worker's parser cache is deleted.
2. The context is found from the cursor outward: `descendant_for_byte_range`, then ancestors, looking at each ancestor's bracket tokens before the cursor. Work is bounded by depth, not by file size. The literal and error-recovery rules keep their meaning; the indentation goldens are the oracle.
3. When the published tree is for the current revision the lookup runs on the input path under a fixed bound (depth and children examined); past the bound, or when the tree is behind, the key is deferred exactly as today.
4. Keys typed while an indent is pending stay queued behind it, as today (checked: `o` followed at once by text puts the text on the new line).
5. A file too large for the syntax owner to parse (the 15 MB case, which is not highlighted either) indents from the previous line and does not start a parse.

Budget: `Enter` and `o` at any cursor position of a 15 MB file, tree warm: under 2 ms to the frame. First `o` after opening: no second parse.

As built (amended with the implementation):

- **The bound.** Per node on the path, the children before the cursor that are examined are the first 32 (where a node's opening bracket is) and the 32 just before the cursor; the run between them (a file's items, a block's statements) is complete syntax whose brackets close inside it. Complete preceding subtrees are passed over; preceding subtrees with errors are read in full, as before. The lookup stops after 4,096 nodes (`INPUT_BUDGET`) and defers. Checked against the old root-to-cursor walk on 330 files in 11 languages (3.3 M positions): identical on every tree without errors. On trees with errors the new answer is local: a malformed literal or an unclosed bracket now affects its own neighbourhood (up to 32 siblings below), where the old walk let it change every position to the end of the file.
- **Deferred keys run on the tree's owner.** A deferred indentation is a job on the analysis actor, which reparses incrementally from the edit journal; the resolution worker keeps no parser. A scratch buffer (shown plain, indented as Rust) keeps a second tree on the same actor, under the same journal.
- **The size rule is 12 MiB (`PARSE_LIMIT`)**, the same for highlighting and indentation: past it a file is shown plain, indents from the previous line, and says so once. The 10 MB budget file is parsed; the 15 MB one is not.
- **Replay.** The published tree is native state and never on the tape; each input-path lookup is recorded as its result (`indent.contexts`).
- `strop bench indent` holds the budget (10 MB file, line 10, the middle and the end, `o` and `Enter`, p95 ≤ 2 ms; the first `o` after open admits no resolution job) and release qualification runs it.

Alternative considered: copy the previous line's indent immediately and correct when the tree arrives. Rejected: the correction would move text the user is already typing after.

### D3. Paint rows straight into the cell buffer — [#312](https://github.com/stropdev/strop/issues/312)

> Amended for v0.79.0: order, kernel obligations and budgets are in [the amendment](#amendment-for-v0790-2026-11-28-frames-lists-themes-the-test-suite-and-the-search-engine).

Stack samples of the main thread while typing (400 samples, glibc build): `ratatui::widgets::Paragraph::render` and its grapheme iteration 33 %, `unicode-width` inside `Buffer::diff` 12 %, the diff itself 9 %, stdout 5 %. Every pane builds `Vec<Line>` of `Vec<Span>` with owned strings and hands it to `Paragraph` ([`render/buffer/rows/mod.rs`](../../crates/strop/src/render/buffer/rows/mod.rs)), which segments and measures text strop already laid out.

Decision: a row painter that writes cells from strop's own layout; no `Paragraph` for buffer rows; row scratch reused across frames. Then decide, with [#72](https://github.com/stropdev/strop/issues/72), whether the ratatui upgrade removes the per-cell width call in `diff` or strop diffs runs itself. Goldens must not change by a cell.

Budget: `input_frame` key+frame p50 ≤ 0.5 ms at 160×50 on the musl artifact.

### D4. In-buffer search: a fast path for the common pattern — [#310](https://github.com/stropdev/strop/issues/310)

> Amended for v0.79.0: step 3 and the "Alternative" paragraph below are replaced by [the engine swap](#amendment-for-v0790-2026-11-28-frames-lists-themes-the-test-suite-and-the-search-engine) ([#320](https://github.com/stropdev/strop/issues/320)).

The streaming literal path exists only for a case-sensitive literal ([`query/parse.rs`](../../crates/strop-grammar/src/query/parse.rs): `whole_word || fold` disables it). Under smartcase an all-lower-case pattern folds case, so the most common search goes to the general engine, which the numbers put at roughly 3–10 MB/s. The literal path itself is a byte-at-a-time KMP.

Decision, in order:

1. Replace the KMP by `memchr::memmem` over rope chunks (carry `needle.len() - 1` bytes across a chunk boundary). `memchr` is already in the lock file; it becomes a direct dependency of `strop-grammar`.
2. A case-folding literal path for ASCII needles: `memchr2` on the rarest needle byte in both cases, verify by folded compare. Non-ASCII folded needles keep the general engine.
3. For the general engine, a required-literal prefilter where the program has one.
4. The same resolver serves preview and execute, so the incremental preview gets the same speed-up; nothing is special-cased for either.

Budget: last-line match in 15 MB under 50 ms and in 500 MB under 1 s for a lower-case literal; `nav_100k /needle` under 20 ms.

Alternative: `regex-cursor` (regex-automata over rope chunks, what Helix uses). Not now: Vim's `\zs`, look-behind and back-references have no translation, so it would be a second engine beside the bounded one, and two engines are how preview and execute come apart. Revisit only if step 3 misses the budget for patterns people actually type.

### D5. File finder: score first, explain later; narrow, do not rescan — [#313](https://github.com/stropdev/strop/issues/313)

> Amended for v0.79.0: the steps are reordered by measurement in [the amendment](#amendment-for-v0790-2026-11-28-frames-lists-themes-the-test-suite-and-the-search-engine).

[`rank`](../../crates/strop-picker/src/rank.rs) scores every catalog entry on one thread on every key and computes match columns for every admitted row, although about fifty are shown.

Decision:

1. Score without indices; compute match columns only for the rows a frame shows.
2. When the query extends the previous one, rank only the previous survivors (a fuzzy match of a longer query is a match of its prefix). The survivor list is derived state keyed by catalog generation and query; a catalog change drops it.
3. Split the catalog into chunks across the picker's worker threads.
4. Listing the tree (1.1 s) is allocator-bound: 329 ms with D1, against Helix's 638 ms. No separate change.

Budget: `find key_list` p50 ≤ 3 ms and p95 ≤ 8 ms on 100k paths (today 7.9 / 16.7 ms on the synthetic set, 21.8 ms p50 on the Linux tree).

### D6. Status: one working-tree scan — [#314](https://github.com/stropdev/strop/issues/314)

[`status()`](../../crates/strop-git/src/status.rs) runs libgit2 `statuses()` (493 ms on the Linux tree) and `line_stats()` then diffs tree→index and index→workdir again (another 420 ms). Decision: line stats are computed only for the paths `statuses()` reported, by pathspec. Then measure again; if a clean large tree is still several times `git status`, move the local status read to `git status --porcelain=v2 -z`, the same CLI the log, blame and commit paths already use, and delete the libgit2 one.

### D7. Open large files without a second copy — [#311](https://github.com/stropdev/strop/issues/311)

[`open_local_file`](../../crates/strop-engine/src/editor/io/open.rs) reads the whole file into a `Vec<u8>` and then builds the rope from it: peak memory is file plus rope (1.1 GB for 500 MB) and the time includes growing a 500 MB vector. Decision: build the rope from a cancellable reader over the file.

### Crates considered

| Crate | Verdict | Reason |
|---|---|---|
| `mimalloc` | **adopt** | D1, measured |
| `memchr` (direct) | **adopt** | D4; already in the tree through `regex` and `grep` |
| `ropey` 1.x | keep | already the text store; no measured gap traces to it |
| `nucleo-matcher` | keep | already the scorer |
| `nucleo` (the parallel engine) | skip | D5 closes the gap on the existing ranking actor; `nucleo` owns the item list and snapshot model, which strop's catalog, frecency priors and folder columns would have to be rebuilt around |
| `ignore`, `grep-searcher` | keep | already in; project search settles in 127 ms where `rg` alone takes 146 ms |
| `helix-core`, `helix-stdx` | skip | not published as libraries (git dependencies on the monorepo); a selection-first editing model would be a second grammar owner |
| Helix's tree-sitter layer (`tree-house`) | skip | the measured problem is a duplicate parse and a root walk in strop's own code |
| `regex-cursor` | **adopt** (amended 2026-11-28) | D4 amended: owner decision, speed over full Vim regex fidelity; pinned `=0.1.5`. The original verdict was "not now" |
| `imara-diff` | skip | the gutter diff's 140 ms per refresh in a large repository is the index re-read, not the diff ([#315](https://github.com/stropdev/strop/issues/315)); libgit2's buffer diff is already in use |
| `gix` | skip | the status gap is two scans, not the library; `gix` adds on the order of a hundred crates against a standing `git2`, `default-features = false` decision |

### Caching and indexing

The owner's guidance for this round: memory may be spent where it buys speed; a cache must never show a stale answer.

- **Already in place:** the prepared project inventory ([project index RFC](2026-11-21-project-index.md)); project search answers in 54 ms with it (not measured without it). The private commit-graph ([history RFC](2026-11-12-history-and-limits.md)); the log answers in 19 ms from the second session on, 483 ms on the first press, as that RFC says it would.
- **Proposed here:** the shared syntax tree (D2), finder survivors (D5), the gutter's base texts ([#315](https://github.com/stropdev/strop/issues/315)). Each is keyed by revision, generation or object id and dropped on change.
- **Worth a design, not in this round:** the finder reading its catalog from the prepared inventory instead of walking again; a blame result kept per `(blob, HEAD)` so the gutter returns instantly after an unrelated edit; a status snapshot shared with the folder view ([#93](https://github.com/stropdev/strop/issues/93)). Each needs its invalidation stated first.
- **Not proposed:** a history row cache (rejected in the history RFC, and the numbers above do not reopen it); a trigram index for search (search is not the bottleneck).

## Compatibility and delivery

No key, file format, session or trace format changes. Replay hashes are unaffected by D1, D4–D7; D2 and D3 must keep frames identical.

Order: D1 first (smallest change, largest effect, and it changes every later measurement), then D2 and D4 (the two waits a user cannot work around), D7. D3, D5, D6 and the gutter diff follow in the next cut. Each issue carries its budget and its `strop bench` scenario.

The harness that produced these numbers is not part of the repository. If the maintainer wants it kept, it belongs under `tools/` as a counterexample finder with these recorded catches; otherwise the budgets in `strop bench` are the surviving artefact.

## Amendment for v0.79.0 (2026-11-28): frames, lists, themes, the test suite, and the search engine

Outside feedback asked four things: paint only the viewport and never rebuild styled spans for lines off screen; keep filtered and sorted list indices in a reusable vector so a frame allocates nothing but the diff; resolve theme tokens once into plain styles; keep golden snapshots out of the normal test run. Each was checked against the code at `cdbfadf` (v0.78.0) and measured. Separately, the owner approved running Vim-syntax patterns on Rust's regex engine ([#320](https://github.com/stropdev/strop/issues/320)); that is the D4 amendment below.

The owner's rule for this amendment: **no optimisation ships without its proof.** Every adopted change names the decision it rests on, moves that decision into `strop-kernel`, states the theorem and the mutant, and lands the kernel work before or with the change. Where the property cannot live in a `no_std` kernel (ratatui, the terminal, the regex library, the fuzzy matcher), the outer-ring check that stands in for the proof is named.

### What was measured

A scratch build of `cdbfadf` (host glibc, thin LTO; not committed) with a counting allocator wrapped around mimalloc and a `frames` scenario: one persistent `Terminal` over the production presenter (`terminal/present.rs`) writing to a sink, 160×50, each frame split into engine preparation, paint and ratatui's diff plus output. Allocations are allocator calls on the main thread. The machine carried other load (load average 5–10); times are ±10 %, allocation counts are exact.

| View, steady frame with nothing changed | frame p50 (µs) | allocations | bytes requested | of which paint (µs / allocations) | diff + output (µs) |
|---|---|---|---|---|---|
| Rust file, 10k lines, highlighted | 377 | 1,785 | 533 KB | 305 / 1,784 | 69 |
| the same, cursor moving so every row scrolls | 480 | 1,851 | 626 KB | 319 / 1,782 | 131 |
| the same, one character inserted | 486 (+36 for the key) | 1,871 | 563 KB | 326 / 1,806 | 73 |
| the same file in two panes | 516 | 3,901 | 760 KB | 439 / 3,900 | 73 |
| plain text, 10k lines of prose | 487 | 4,209 | 813 KB | 414 / 4,208 | 70 |
| help | 457 | 3,707 | 728 KB | 381 / 3,706 | 70 |
| status view | 352 | 1,393 | 199 KB | 277 / 1,392 | 72 |
| commit log | 790 | 8,813 | 778 KB | 713 / 8,812 | 73 |
| commit files | 464 | 2,503 | 513 KB | 389 / 2,502 | 72 |
| file with the blame gutter | 434 | 3,233 | 691 KB | 359 / 3,232 | 69 |
| folder view | 482 | 2,407 | 517 KB | 409 / 2,406 | 69 |
| file finder, 97,374 paths, `sched` typed | 757 | 3,053 | 651 KB | 680 / 3,040 (panes 290, picker 379, dim 18) | 72 |
| project search, 625 hits | 786 | 3,473 | 680 KB | 710 / 3,465 (panes 296, picker 404, dim 18) | 72 |

Other figures from the same build:

- `Tokens::new` (the per-frame style table): 23 ns.
- ratatui `Buffer::diff` alone on an unchanged 160×50 frame: 61 µs; it measures every cell's width whether or not the cell changed.
- An unchanged frame writes 54 bytes to the terminal; a frame where every row scrolled writes 18.7 KB.
- Finder, Linux tree, one key (`sched` ⇄ `schedc`): 10.4 ms to the settled list, of which the scoring loop over all 97,374 paths is 8.4 ms and the sort 0.1 ms; 3,049 paths are admitted. Scoring without match columns changed nothing measurable (`find key_list` p50 5.77 → 5.69 ms).
- On the public 0.78.0 musl artifact: `input_frame key+frame` p50 0.41 ms (120×40), `find key_list` p50 5.56 / p95 9.89 ms.
- `cargo test --locked --workspace`, 29 test binaries, debug build: 208 s of test time. The Neovim differential is 92 s, the `strop` unit binary 33 s, `terminal_editor` 28 s, `strop-engine` 10 s, `strop-grammar` 9 s, `strop-picker` 7 s. Inside the `strop` unit binary one test, `tabline_tests::the_current_label_always_shows_and_the_row_fits`, runs 31.4 s and sets the binary's wall time; the 68 golden tests (376 files, two palettes, Hone and Vellum) use 25 CPU-seconds and finish in about 2 s of wall time on this host because they run in parallel.

### Verdict on the four points

1. **Paint only the viewport; do not rebuild spans for lines off screen.** Already true for which lines are painted: [`render_pane`](../../crates/strop/src/render/buffer/rows/mod.rs) walks `view_top .. view_top + rows`, analysis is admitted for that window only ([`prepare.rs`](../../crates/strop-engine/src/editor/prepare.rs) `admit_visible_work`), and [`content_spans`](../../crates/strop/src/render/buffer/content.rs) starts from a layout checkpoint at the horizontal origin and stops at the right edge. A real gap inside the window: every visible grapheme becomes its own `Span` holding an owned `String`, and `Paragraph` then segments and measures the text again. That is D3; the table above is its evidence in every view, not only while typing. A second gap was found: with the finder, search or symbols picker open the panes are painted in full (290 µs, 1,700 allocations) and then erased by the picker's `Clear` ([`picker/mod.rs`](../../crates/strop/src/render/picker/mod.rs)). That is D8.
2. **Reusable filtered and sorted indices for lists.** Partly true. A frame never filters or sorts: ranking runs on the picker's worker, the installed `Ranking` (`rows`, `columns`) is kept between frames, and the list paints `height` rows from it. Not true per key: each key builds a new `Ranking` and scores the whole catalog. The measurement moves D5's steps (below). List rows have the same per-frame allocation as buffer rows (1,300–1,700 per frame in the picker card).
3. **Resolve theme tokens once.** Already true. The seven palettes ([`strop-core/src/theme/*.rs`](../../crates/strop-core/src/theme.rs)) are `const` tables of RGB triples: no parsing and no colour arithmetic at run time. [`Tokens::new`](../../crates/strop/src/render/style.rs) copies 45 of them into ratatui colours once per frame and costs 23 ns. The one piece of colour arithmetic in a frame is the dimmed backdrop behind a picker (18 µs per frame, 2 % of that frame). Nothing is adopted here and nothing needs a proof ([#318](https://github.com/stropdev/strop/issues/318), closed with this measurement).
4. **Keep golden snapshots out of the normal test run** ([#319](https://github.com/stropdev/strop/issues/319)). Not adopted: it would not speed a normal run. The goldens are about 2 s of wall time; the time is one width sweep (31 s) and the Neovim differential (92 s, already parallel inside). Rendering lives in the binary crate, so a separate golden test binary would need a library split for no gain, and AGENTS.md has the compose gate run everything either way. The sweep is fixed instead ([#324](https://github.com/stropdev/strop/issues/324)).

### Proof and test obligations (amended)

The original section said nothing in this RFC changes a kernel decision. With D3, D5 and D8 as amended that is no longer so. Two new kernel modules, written before the changes that use them; plain safe Rust, explicit loops, no closures; every function total (saturating or compare-before-subtract arithmetic) unless noted, so [#282](https://github.com/stropdev/strop/issues/282)'s audit lists it `TOTAL`.

#### `strop-kernel/src/viewport.rs` — [#321](https://github.com/stropdev/strop/issues/321), before D3 and D8

The decisions moved (moved, not copied; production calls them):

```rust
/// The buffer lines a pane of `height` rows shows from `top`: `first..end`.
pub fn rows(top: usize, height: usize, lines: usize) -> (usize, usize);

/// The top a pane keeps over a document of `content` rows.
pub fn clamp_top(top: usize, height: usize, content: usize) -> usize;

/// The first list row shown so that `selected` stays on screen.
pub fn follow(top: usize, selected: usize, visible: usize, len: usize) -> usize;

/// The cells of one glyph that fall in the window `origin..origin + columns`.
pub struct Clip { pub x: usize, pub width: usize, pub complete: bool }
pub fn clip(cell: usize, width: usize, origin: usize, columns: usize) -> Option<Clip>;

/// Whether rectangle `a` holds every cell of rectangle `b`.
pub struct Rect { pub x: usize, pub y: usize, pub width: usize, pub height: usize }
pub fn covers(a: Rect, b: Rect) -> bool;
```

Where they are today: `rows` is written twice, in `prepare.rs` (`admit_visible_work`) and in `render/buffer/rows/mod.rs`, and paint is only right while the two agree; `clamp_top` is in `prepare.rs` (`adjust_active_viewport`); `follow` is `Picker::reveal_selected` in [`strop-picker/src/lib.rs`](../../crates/strop-picker/src/lib.rs) and a second expression in `render/picker/rows.rs`; `clip` is [`strop_core::layout::clip`](../../crates/strop-core/src/layout.rs), the one seam every glyph, overlay and caret projects through; `covers` is new.

Theorems (`proofs/Kernel/Viewport.lean`, each added to `claims` in `proofs/Audit.lean`):

| Claim | Statement |
|---|---|
| `rows_spec` | `rows top h n = (min top n, min (top + h) n)`; so `first ≤ end ≤ n`, `end - first ≤ h`, and a line `l` is in `first..end` exactly when `top ≤ l < top + h` and `l < n` |
| `rows_crop` | the lines of the window are the lines of the whole document, in order, with the first `top` dropped and at most `h` kept: painting the window equals painting every line and cropping to the pane |
| `clamp_top_spec` | the result is `min top (content - height)` (0 when the document is shorter than the pane); it never exceeds `top`; from it the pane shows `min height content` content rows |
| `follow_spec` | when `visible > 0` and `selected < len`: `top' ≤ selected < top' + visible`; when `selected` was already in `top..top + visible` and `top < len`, `top' = top` |
| `clip_spec` | with `G = cell..cell + width` and `W = origin..origin + columns`: `none` exactly when `G ∩ W` is empty; otherwise `x = max cell origin - origin`, `width = |G ∩ W|`, `complete ⇔ G ⊆ W`; so `x + width ≤ columns` |
| `clip_crop` | over a row modelled as glyph widths laid end to end: the cells `clip` gives the row's glyphs are pairwise disjoint, in order, and are exactly the row's cells inside `W` moved left by `origin`; a glyph cut by either edge is never `complete`. The visible row equals the whole row laid out and then cropped |
| `covers_spec` | `covers a b` exactly when every cell of `b` is a cell of `a` (an empty `b` is covered) |

Mutants (`proofs/mutants.py`), each of which must break the named claim and no other:

| Mutant | Change | Breaks |
|---|---|---|
| `viewport-window-past-end` | `rows` drops the `min` with `lines` | `rows_spec` |
| `viewport-window-one-short` (paired) | `rows` ends at `top + height - 1`, model patched the same way | `rows_crop` |
| `viewport-top-past-content` | `clamp_top` returns `top` | `clamp_top_spec` |
| `viewport-selection-off-screen` | `follow` uses `selected - visible` in place of `selected + 1 - visible` | `follow_spec` |
| `viewport-clip-no-left-clamp` | `x: cell - origin` without the `max` | `clip_spec` |
| `viewport-half-glyph` (paired) | `complete` drops `right == end`, bridge patched | `clip_crop` |
| `viewport-covers-origin-only` | `covers` compares corners' origins and not the far edges | `covers_spec` |

Outer ring, for what the kernel cannot see (the cell buffer is ratatui's):

- every golden cell grid byte-identical before and after D3 and D8;
- a property test in `render/buffer/tests/` over seeded random buffers (ASCII, wide, combining, tabs, control characters, CRLF), random `top`, horizontal origin and pane size, in every one of the seven palettes: an inactive pane painted at `(top, origin)` with `W×H` text cells equals the same document painted at `(0, 0)` into a `(origin + W)×(top + H)` pane and cropped, with a glyph cut at an edge shown as blanks. This is "optimised paint equals naive paint" without keeping a second painter;
- before the `Paragraph` path is deleted, one differential run of old against new over the golden scenarios and the same random buffers, its counts recorded in the as-built note; the old path is then deleted in the same change (no pair is left behind).

#### `strop-kernel/src/rank.rs` — [#322](https://github.com/stropdev/strop/issues/322), before D5

```rust
/// A ranked row's order key.
pub struct Key { pub score: u32, pub length: u32, pub item: u32 }

/// Whether `a` is listed before `b`: higher score, then shorter, then catalog order.
pub fn before(a: Key, b: Key) -> bool;

/// Append the merge of two runs, each already in `before` order.
pub fn merge(a: &[Key], b: &[Key], out: &mut Vec<Key>);
```

This is the order [`rank`](../../crates/strop-picker/src/rank.rs) sorts by today (`(Reverse(score), length, at)` for paths; `length` is 0 elsewhere). `merge` is conditional on `out.len() + a.len() + b.len() < usize::MAX` (Aeneas models `Vec::push` as failing there), like `lines::runs`.

| Claim | Statement |
|---|---|
| `before_spec` | `before a b` exactly when `(b.score, a.length, a.item) < (a.score, b.length, b.item)` lexicographically; on keys with distinct `item` it is a strict total order |
| `merge_spec` | `merge a b out` appends a list that is a permutation of `a ++ b`, and that is in `before` order when `a` and `b` are |
| `merge_is_sort` | for runs with pairwise distinct `item`: folding `merge` over any split of a catalog into consecutive chunks, each chunk's rows in `before` order, gives the same list as ordering all rows at once. Ranking in chunks on several threads lists exactly what ranking on one thread lists |

| Mutant | Change | Breaks |
|---|---|---|
| `rank-lower-score-first` | `before` compares scores ascending | `before_spec` |
| `rank-merge-prefers-right-on-tie` | `merge` takes from `b` when neither is `before` the other | `merge_spec` |
| `rank-merge-drops-tail` (paired) | `merge` stops when `a` is exhausted, bridge weakened to match | `merge_is_sort` |

Outer ring: scoring is `nucleo-matcher` and stays outside the kernel. The ranking oracle (`scorebench`, the hermetic ranking tests and `rank/corpus_tests.rs`) must return identical rows and columns for one thread and for 2, 3, 8 and 32 chunks, over seeded random catalogs and queries, including a catalog that grows between keys.

Not adopted, and why:

- **A cache of painted rows** (skip a visible row whose inputs did not change). The owner's suggested theorem, "the dirty set covers every row whose content or style changed", is only as strong as the list of inputs a row is keyed by, and a row's cells depend on text, syntax, search hits, selections, the cursor line, diagnostics, git signs, the blame column, the pair highlight, the palette, the origin and the widths, all read by paint outside the kernel. A missed input is a stale row, which is the lie the cache rule forbids, and no kernel theorem can state that the key is complete. Every frame therefore stays a full repaint of the visible window into a fresh cell buffer; the only incremental step is ratatui's cell diff of two complete frames, which cannot drop a changed cell by construction. Revisit only if D3 lands and misses its budget.
- **Narrowing the finder to the previous key's survivors** (D5 step 2), for this cut. Its soundness rests on a property of the matcher (a path that matches a longer query matches the shorter one, across smart case, added terms and `/` terms) that can be tested but not proved in the kernel. It moves to v0.80.0 ([#327](https://github.com/stropdev/strop/issues/327)) and is taken only if chunked ranking misses D5's budget; its obligations are written in that issue.
- **A resolved style table**: point 3 above.

### D3 as amended — paint cells from strop's layout — [#312](https://github.com/stropdev/strop/issues/312)

Order: the frame scenario and allocation test first ([#323](https://github.com/stropdev/strop/issues/323)), the `viewport` kernel module second ([#321](https://github.com/stropdev/strop/issues/321)), the painter third.

1. A row painter in `render/buffer/` that writes symbol and style into the frame's cells at the position `viewport::clip` gives. No `Vec<Line>`, no `Span`, no `String` per glyph, no `Paragraph`, for every row owner (`rows/owner.rs`: file, help, directory, review, collection, diff, status, list) and for the fixed margins (sidebar, blame, numbers).
2. `render_pane` and `prepare.rs` take the line window from `viewport::rows`; `strop_core::layout::clip` becomes the typed call into `viewport::clip`.
3. The picker card's rows and preview are the same change in a second file set and move to v0.80.0 ([#326](https://github.com/stropdev/strop/issues/326)): a key in the finder is bound by ranking (8.4 ms), not by its 0.76 ms frame.
4. The per-cell width call inside ratatui's diff (61 µs a frame) is decided with the dependency upgrade ([#72](https://github.com/stropdev/strop/issues/72), now v0.80.0), not here.

The original budget (`input_frame key+frame` p50 ≤ 0.5 ms at 160×50) no longer binds: that scenario builds a new terminal per frame over an unhighlighted scratch buffer and already reads 0.41 ms at 120×40. It is kept and tightened to 160×50, and the budgets that bind are new, in `strop bench frame` ([#323](https://github.com/stropdev/strop/issues/323)), on the musl artifact at 160×50:

| Scenario | today (host glibc) | budget, p50 |
|---|---|---|
| highlighted Rust file, unchanged frame | 377 µs | ≤ 200 µs |
| the same, one character inserted, key + frame | 522 µs | ≤ 300 µs |
| commit log, unchanged frame | 790 µs | ≤ 350 µs |

and as a deterministic unit test (allocation counts do not depend on the machine): painting the panes of an unchanged frame allocates **0** times for a file buffer, with and without the blame gutter and in a split; the whole frame allocates at most 64 times (today 1,785); a decorated owner (help, status, log, commit files, folder) allocates at most 16 times per visible row (today 28–176). The test build of the binary crate counts allocations per thread; the shipped binary's allocator is unchanged.

### D5 as amended — the finder — [#313](https://github.com/stropdev/strop/issues/313)

The measurement reorders the three steps. The per-key cost is the scan of every path (8.4 of 10.4 ms on the Linux tree), not the match columns and not the sort.

1. **Rank in chunks across the picker's worker threads** (was step 3), each chunk's rows ordered, merged by `rank::merge` ([#322](https://github.com/stropdev/strop/issues/322), which lands first). This helps the first key and backspace as much as an extending key, and its only new decision is the merge.
2. **Match columns for shown rows only** (was step 1) is dropped as a speed step: measured, it changes nothing. `Ranking.columns` stays as it is.
3. **Survivors** (was step 2) moves to v0.80.0 ([#327](https://github.com/stropdev/strop/issues/327)), as above.

Budget unchanged: `find key_list` p50 ≤ 3 ms and p95 ≤ 8 ms on 100k paths (5.56 / 9.89 ms on the 0.78.0 artifact).

### D8 — do not paint what an opaque layer covers — [#325](https://github.com/stropdev/strop/issues/325)

With a picker that owns the frame (`prepare::picker_owns_frame`), `render` skips the pane layer when `viewport::covers(cleared, panes)` holds; the statusline row still paints. After [#321](https://github.com/stropdev/strop/issues/321) and with or after [#312](https://github.com/stropdev/strop/issues/312). Outer ring: the picker goldens are byte-identical, and a test paints each owning picker over two different documents and requires identical cells above the statusline. Budget: the finder's unchanged frame drops by the pane layer's share (290 of 757 µs today).

### D9 — one width sweep is the `strop` unit binary's wall time — [#324](https://github.com/stropdev/strop/issues/324)

`tabline::fit` measures every label's name again for each of forty candidate caps; the sweep test calls it 18,000 times. Label widths become data on `Tab`, computed once. No split of the test suite. Budget: the `strop` unit binary under 12 s in a debug build on this host (33 s today), every test still in `cargo test --locked` and in the compose gate.

### D4 amended — Vim's pattern syntax on Rust's regex engine — [#320](https://github.com/stropdev/strop/issues/320)

Owner decision, 2026-11-28: speed over full Vim regex fidelity, as Helix does. This reverses the "not now" verdict on `regex-cursor` in the crates table, replaces D4 step 3 (a hand-built required-literal prefilter), and needs AGENTS.md doctrine 1 amended in the same change. No Lean proof applies: the matcher is a library outside the kernel, so nothing is moved inward and nothing is weakened; the control is a differential against the current engine before it is deleted, plus the Neovim corpus.

**Decision.** The parser in [`strop-grammar/src/query/parse`](../../crates/strop-grammar/src/query/parse.rs) keeps Vim's syntax (magic and `\v`, groups, `\|`, `\<` `\>`, `\+` `\=` `\?` `\{n,m}` `\{-n,m}`, `\%[...]`, classes, `\_x`, `\n`, `\zs` `\ze`, `\%^` `\%$`, character codes, `\c` `\C`, smart case). It lowers its AST to a `regex_syntax::hir::Hir` in place of the instruction list; `regex-cursor`'s meta engine runs that over the rope's chunks. The backtracking machine ([`query/exec.rs`](../../crates/strop-grammar/src/query/exec.rs)), the step budget, and the literal scanner from D4 steps 1–2 ([`resolve/search/literal.rs`](../../crates/strop-grammar/src/resolve/search/literal.rs), `parse/literal.rs`) are deleted; the library's own literal prefilters take the scanner's place. Nothing keeps the old engine as a fallback.

**Crate facts** (checked against the registry and the 0.1.5 source, 2026-11-28):

| Question | Finding |
|---|---|
| Published and maintained? | `regex-cursor` 0.1.5 on crates.io, MIT OR Apache-2.0, MSRV 1.65; six releases from 2024-01-25 to 2025-02-26; about 390k downloads, 140k of them recent; written by a Helix maintainer and it is what Helix searches with. It calls itself a prototype for upstream streaming support, and its last release is 21 months old: pin `=0.1.5` |
| Rope compatibility | optional `ropey ^1.6.1` with `default-features = false`; the lock file has ropey 1.6.1, so there is one ropey and the workspace's rule against `unicode_lines` holds |
| Leftmost-first with captures | yes: `engines::meta::Regex`, `MatchKind::LeftmostFirst`, `search_captures`, `Builder::build_from_hir`. Engines present: lazy DFA, full DFA, PikeVM. Absent: the one-pass DFA, the bounded backtracker and the reverse-DFA strategies (commented out), so captures always resolve on the PikeVM |
| musl / static fit | pure Rust, no build script, no C; dependencies `regex-automata ^0.4.7`, `regex-syntax ^0.8.4`, `memchr`, `log`, all already in the lock file (0.4.18, 0.8.11). One new crate in the tree. 16 `unsafe` blocks, all in the DFA search loops it carries over from `regex-automata` |
| Can project search share the compiled program? | No. Project search does not read Vim patterns: it speaks the filter language of the [one-filter-language RFC](2026-11-17-one-filter-language.md) with `regex:` terms in Rust regex syntax, compiled by `regex` and by `grep-regex`'s `RegexMatcherBuilder::build(&str)`, which takes a pattern string, not an `Hir` or a built program. After this change both searches run on the same library (`regex-automata`); they keep two front ends. The sentence in [#320](https://github.com/stropdev/strop/issues/320) that project grep consumes the same compiled query is corrected to: preview, `n`/`N`, incsearch, the highlight and `:s` share one `CompiledQuery`; project search is already on Rust's regex |

A spike (not committed) built `Hir` by hand and searched a 500 MB, 4.3-million-line rope whose only match is its last line, host glibc: `[A-Z]\w\+Error` 0.55 s; a literal 49 ms; `\<needle_\w\+` 50 ms; three alternated literals 68 ms; a folded literal 64 ms; `\w\+_zzz` (no usable prefix) 0.55 s; `\d\{4}-zzz` with no match in a digit-heavy log 1.04 s; `\(a*\)*b` over 200,000 `a` 0.3 ms.

**Constructs the current parser accepts that have no exact translation.**

| Construct | Today | After | Disposition |
|---|---|---|---|
| Back-references `\1`…`\9` in a pattern | matched | no translation | typed refusal, "back-references are not supported in search". `\1`…`\9` in a `:s` replacement are unaffected (capture groups) |
| A pattern whose preferred match at a position is empty while a later alternative at the same position is not: `a\{-}`, `x*\|y`, `\(\)\|a` | the engine keeps looking and reports the non-empty match (the corpus pins `a\{-}` on `aaa` to `0..1`) | leftmost-first reports the empty match; with `x*\|y` the `y` is never found | listed divergence; the corpus row moves to the divergence list |
| `\<`, `\>` and the whole-word flag of `*`/`#` | the editor's word model: `char::is_alphanumeric` or `_` | the library's Unicode word class: adds combining marks and connector punctuation other than `_`, drops letter-like and other numbers (`²`, `½`, `Ⅷ`) | listed divergence; ASCII and ordinary letters agree. `w`/`b`/`*` keep the editor's model for motion |
| Case folding of non-ASCII text | equal lower-case forms | Unicode simple case folding: `s` also matches `ſ`, `σ` also matches `ς` | listed divergence; the Kelvin-sign special case is deleted |
| The step budget (`TooComplex`, Vim's E363) | a pattern over 100,000 steps per attempt is refused at run time | matching is linear; only a pattern too large to compile is refused | improvement; `QueryError::TooComplex` keeps one meaning, at compile time |
| Capture values inside a zero-width loop, `\(^\)*` | the loop guard decides | the library's rule | checked by the differential; listed only if it differs |

Translated exactly, and worth stating because the obvious translation is wrong: `.` and negated classes do not match a line break but do match a `\r` that is not followed by `\n` (class without `\r` `\n`, or `\r` followed by the `StartCRLF` look); `^` is `StartLF`; `$` is `EndLF` or `EndCRLF`, so `\r$` still finds the CR of a CRLF ending; `\n` is `\r\n` or `\n`; `\zs` and `\ze` are empty capture groups whose positions give the reported span, and iteration resumes from the reported end; the whole-word flag is the two half-boundary looks. All of these were checked in the spike. Already refused and still refused: lookaround (`\@=` `\@!` `\@<=` `\@<!` `\@>`), `\&`, `~`, `\%V`, `\%#`, `\%l` `\%c` `\%v`, `\%=`, `\M`, `\V`, `\z(`, `[a&&b]`.

**Steps** (one lane; touches `strop-grammar` and the help and compatibility text only):

1. `crates/strop-grammar/Cargo.toml`: `regex-cursor = { version = "=0.1.5", default-features = false, features = ["perf-inline", "ropey"] }` and `regex-syntax = "0.8"`.
2. New `query/parse/lower.rs`: `Node` → `Hir`. Remove `Inst`, `Class`, the loop guards and `MAX_INSTS` from `query/parse.rs`; `Program` holds the built regex, the group count, the two marker groups for `\zs` `\ze`, `fold` and `whole_word`. `parse/atoms.rs`: the back-reference arm returns `QueryError::Unsupported`.
3. New `query/cursor.rs`: strop's `regex_cursor::Cursor` over the rope's chunks that also reads the work owner's cancel flag at each chunk; a cancelled search returns `QueryError::Cancelled`, never a partial answer.
4. `resolve/search/mod.rs`: `forward`, `backward`, `all`, `visit_all`, `visit_captures` and `literal_from` on the new program, with today's iteration rules written once: matches do not overlap, an empty match advances one character, an empty match at the end of the buffer is not reported, `backward` is the last match starting before the cursor.
5. Before deleting anything, the differential (below) runs old against new in the same test binary. Its result goes into this RFC's as-built note.
6. Delete `query/exec.rs`, `query/parse/emitter.rs`, `query/parse/literal.rs`, `resolve/search/literal.rs`, `STEP_BUDGET` and the differential's old side, in the same change. The public API of `strop_grammar::query` (`CompiledQuery`, the five search functions, `SearchMatch`, `QueryError`) keeps its shape, so the callers in `strop-engine` (`normal/search.rs`, `analysis/search.rs`, `cmdline/accept.rs`, `cmdline/parsed.rs`, `visual/verbs.rs`, `ex/substitute/mod.rs`, `ex/range.rs`) do not change.
7. Text: `crates/strop-grammar/corpora/vim-query-corpus.txt` (divergent rows move to a listed-divergence section the corpus test also checks), `strop-engine/src/keymap/lookup.rs` (`compat_report`), `strop-engine/src/editor/help/content.rs`, `CHANGELOG.md`, and AGENTS.md doctrine 1.
8. `crates/strop/src/bench/search_500mb.rs` and `bench.rs`: the budgets below.

**Gate.**

- *Differential before deletion.* Seeded random patterns from the parser's own grammar (every construct in the table and in the "translated exactly" list, both magic levels) × seeded random buffers (ASCII, multibyte, CRLF, a lone CR, the Kelvin sign, matches across rope chunk edges, empty lines, a buffer with no final newline), for `search_all`, `search_forward`, `search_backward` and the capture groups. Every difference is either fixed or is one of the listed divergences; an unlisted difference blocks the change.
- *Neovim.* `corpora/vim-query-corpus.txt` (174 rows) passes with only the listed rows moved; the differential harness (`crates/strop/tests/differential`) stays clean.
- *Existing behaviour tests* in `strop-grammar/src/tests.rs` and `resolve/search/tests.rs` pass unchanged except the step-budget test, which becomes "a pathological pattern finishes".
- *Preview and execute.* One `CompiledQuery` for both, as now; no consumer gets its own path.

**Budgets** (`strop bench`, musl artifact, held at p95 like the others):

| Scenario | budget |
|---|---|
| `search_500mb`: `/[A-Z]\w\+Error`, only match on the last line of 500 MB | ≤ 1 s |
| `search_500mb`: lower-case literal (D4's existing budget) | ≤ 1 s, expected about 0.1 s |
| `nav_100k /needle` (existing) | ≤ 20 ms |
| new `search_pathological`: `\(a*\)*b` on a 1 MB line of `a` | ≤ 50 ms, and no refusal |

**AGENTS.md doctrine 1, proposed wording** (for the implementation change to apply; only the owner changes doctrine):

> 1. **Fidelity**: the vim grammar is the product. Deviations are bugs, and bugs are found by the differential harness (0001 §5.10), not by vibes. One stated exception: search patterns keep Vim's syntax but are matched by Rust's regex engine, for speed. Back-references in a pattern are refused, and the listed divergences in `strop compat` (empty-first alternatives, the Unicode word class, Unicode case folding) are the whole exception; a difference from Neovim outside that list is still a bug.

### Order for v0.79.0

Four lanes that do not share files; inside a lane, in order. A kernel item lands before the change that calls it.

| Lane | Order |
|---|---|
| Kernel and proofs (`strop-kernel`, `proofs/`; one lane, because every kernel change regenerates `proofs/Extracted/`) | [#321](https://github.com/stropdev/strop/issues/321) viewport → [#322](https://github.com/stropdev/strop/issues/322) rank order and merge → [#305](https://github.com/stropdev/strop/issues/305) sign rows → [#306](https://github.com/stropdev/strop/issues/306) the second span rule → [#282](https://github.com/stropdev/strop/issues/282) steps 2–3 |
| Search (`strop-grammar`) | [#320](https://github.com/stropdev/strop/issues/320) |
| Frames (`crates/strop/src/render`, `bench`) | [#323](https://github.com/stropdev/strop/issues/323) scenario and allocation test → [#324](https://github.com/stropdev/strop/issues/324) tab line sweep → [#312](https://github.com/stropdev/strop/issues/312) painter (after #321) → [#325](https://github.com/stropdev/strop/issues/325) covered panes |
| Lists and git (`strop-picker`, `strop-git`, engine git worker) | [#314](https://github.com/stropdev/strop/issues/314) status in one scan → [#315](https://github.com/stropdev/strop/issues/315) gutter → [#313](https://github.com/stropdev/strop/issues/313) finder in chunks (after #322) |

Moved to v0.80.0, with the reason on each issue: the nine git-view fixes and the forge decision ([#294](https://github.com/stropdev/strop/issues/294), [#295](https://github.com/stropdev/strop/issues/295), [#297](https://github.com/stropdev/strop/issues/297)–[#302](https://github.com/stropdev/strop/issues/302), [#304](https://github.com/stropdev/strop/issues/304)), which rank third in the owner's order and share the engine's git files with #314 and #315; the replace-save profile ([#317](https://github.com/stropdev/strop/issues/317)); picker rows ([#326](https://github.com/stropdev/strop/issues/326)); finder survivors ([#327](https://github.com/stropdev/strop/issues/327)); the ratatui decision ([#72](https://github.com/stropdev/strop/issues/72)). If the cut still runs long, the first thing to drop is #315, then #314: both are independent of everything else here.

## Decision log and as-built notes

- 2026-11-28: drafted from measurements of v0.77.0. Awaiting the maintainer's decision; milestone assignment is not acceptance.
- 2026-11-28, D4 as built ([#310](https://github.com/stropdev/strop/issues/310)): the literal scanner copies the rope through a bounded 64 KiB window (overlapping by `needle.len() - 1`) instead of walking rope chunks, so a match across any number of chunks is whole in one window; `memmem` for exact needles, `memchr2` on the rarest byte for ASCII-folded ones, and a word check for the whole-word flag and `\<`/`\>`. KELVIN SIGN (U+212A, the only non-ASCII char whose lowercase is ASCII) near a candidate hands the rest of the walk to the engine. The scanner walks matches non-overlapping, as the engine and Vim do; the old KMP reported overlapping matches for case-sensitive literals (`/aa` counted 3 in `aaaa`, Neovim 2). Step 3 (a required-literal prefilter for the general engine) is not built. Host glibc: `nav_100k /needle` 6,046 ms (0.77.0 musl) → 4.5 ms; a last-line lower-case literal in 500 MB ≈ 50 ms in the grammar crate, 120 ms through `strop bench search_500mb` (paste + Enter: one preview and the jump).
- 2026-11-28, D7 as built ([#311](https://github.com/stropdev/strop/issues/311)): `LocalFs::read_rope` builds the rope through a 1 MiB `BufReader` wrapped in a reader that stops at cancellation; the notify reopen path uses it too. The budget lives in a new `strop bench open_100mb` (a real file through the open path; `buffer_100mb` opens from memory and cannot see the copy): open peak ≤ 1.3× the settled document. 468 MB log, `strop headless` open: peak RSS 1,062 MB (0.77.0 musl) → 576 MB; 1.31–1.42 s → 0.87–1.07 s on a quiet host (the new figure is a glibc build).
- 2026-11-28, amendment for v0.79.0: outside feedback on rendering, lists, themes and the test suite was measured against `cdbfadf`. Viewport-only painting and resolved theme tokens were already true; the per-glyph spans (D3), the per-key full scan in the finder (D5, steps reordered by measurement), panes painted under an owning picker (D8) and one 31 s width sweep (D9) are the gaps. Per the owner's rule that no optimisation ships without its proof, two kernel modules (`viewport`, `rank`) are specified with theorems and mutants and are ordered before the changes that call them; a painted-row cache and finder survivors are not adopted. D4 is amended by owner decision: Vim's pattern syntax on `regex-cursor`, the backtracking engine deleted, back-references in a pattern refused, three listed divergences; no Lean proof applies to it and its gate is a differential before deletion plus the Neovim corpus. Awaiting the maintainer's acceptance of the amendment; milestone assignment is not acceptance.
