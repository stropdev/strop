---
rfc: "2026-11-28-measured-performance"
title: "Close the measured gaps to Helix and Neovim: allocator, structural indent, in-buffer search, finder, status, large files"
status: "draft"
implementation: "not-started"
authors: ["Fable"]
created: "2026-11-28"
updated: "2026-11-28"
issue: "https://github.com/stropdev/strop/issues/307"
milestone: "v0.78.0"
supersedes: []
superseded_by: []
blocked_on: ["Adopt mimalloc as the shipped binary's allocator?", "Read status through libgit2 or through git status --porcelain=v2?"]
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

Nothing in this RFC changes a kernel decision, and no proof is weakened.

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

Alternative considered: copy the previous line's indent immediately and correct when the tree arrives. Rejected: the correction would move text the user is already typing after.

### D3. Paint rows straight into the cell buffer — [#312](https://github.com/stropdev/strop/issues/312)

Stack samples of the main thread while typing (400 samples, glibc build): `ratatui::widgets::Paragraph::render` and its grapheme iteration 33 %, `unicode-width` inside `Buffer::diff` 12 %, the diff itself 9 %, stdout 5 %. Every pane builds `Vec<Line>` of `Vec<Span>` with owned strings and hands it to `Paragraph` ([`render/buffer/rows/mod.rs`](../../crates/strop/src/render/buffer/rows/mod.rs)), which segments and measures text strop already laid out.

Decision: a row painter that writes cells from strop's own layout; no `Paragraph` for buffer rows; row scratch reused across frames. Then decide, with [#72](https://github.com/stropdev/strop/issues/72), whether the ratatui upgrade removes the per-cell width call in `diff` or strop diffs runs itself. Goldens must not change by a cell.

Budget: `input_frame` key+frame p50 ≤ 0.5 ms at 160×50 on the musl artifact.

### D4. In-buffer search: a fast path for the common pattern — [#310](https://github.com/stropdev/strop/issues/310)

The streaming literal path exists only for a case-sensitive literal ([`query/parse.rs`](../../crates/strop-grammar/src/query/parse.rs): `whole_word || fold` disables it). Under smartcase an all-lower-case pattern folds case, so the most common search goes to the general engine, which the numbers put at roughly 3–10 MB/s. The literal path itself is a byte-at-a-time KMP.

Decision, in order:

1. Replace the KMP by `memchr::memmem` over rope chunks (carry `needle.len() - 1` bytes across a chunk boundary). `memchr` is already in the lock file; it becomes a direct dependency of `strop-grammar`.
2. A case-folding literal path for ASCII needles: `memchr2` on the rarest needle byte in both cases, verify by folded compare. Non-ASCII folded needles keep the general engine.
3. For the general engine, a required-literal prefilter where the program has one.
4. The same resolver serves preview and execute, so the incremental preview gets the same speed-up; nothing is special-cased for either.

Budget: last-line match in 15 MB under 50 ms and in 500 MB under 1 s for a lower-case literal; `nav_100k /needle` under 20 ms.

Alternative: `regex-cursor` (regex-automata over rope chunks, what Helix uses). Not now: Vim's `\zs`, look-behind and back-references have no translation, so it would be a second engine beside the bounded one, and two engines are how preview and execute come apart. Revisit only if step 3 misses the budget for patterns people actually type.

### D5. File finder: score first, explain later; narrow, do not rescan — [#313](https://github.com/stropdev/strop/issues/313)

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
| `regex-cursor` | not now | D4, alternatives |
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

## Decision log and as-built notes

- 2026-11-28: drafted from measurements of v0.77.0. Awaiting the maintainer's decision; milestone assignment is not acceptance.
