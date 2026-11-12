# 0099 — Caching: one proved core, and only where the numbers ask for it

- Status: **in force** (2026-11-12). The core and the symbols flow have landed;
  git history is chosen and designed but not landed (§4.2). §9 is the record.
- Source: the maintainer's issue ("find symbols in a file is super slow … explore
  pre-compute and caching … the caching model needs proper lean validation"),
  widened the same day to "caching in the critical/slow flows generally, without
  sacrificing correctness", with three guardrails: measure first, one cache core,
  and a build that never blocks anything.
- Follows [0086](0086-verified-foundation.md) (the ring design: the decision
  moves into `strop-kernel` and is proved in Lean over the shipped code).

## 1. What was measured

Release build of `0bfde59`, this machine (32 cores, warm page cache), the real
binary driven by `strop headless` with `--log`; times are differences between
`elapsed_us` stamps in the trace, key event to the last frame whose cells
changed. Nothing here used the network.

Corpora: this repository (1,102 files; `rust-analyzer 1.97.1`); a generated Rust
file of 15,400 lines / 4,900 symbols in its own crate; the local cargo registry
sources (75,958 files, 2.0 GB); helix (7,689 commits); a generated repository of
150,000 linear commits over 2,000 files.

| # | Flow | First time | Every time after | Where the time goes |
|---|---|---|---|---|
| 1 | `Space g h` file history, 150k commits | 1,120–1,170 ms | the same | `git log -- file` walks the whole history (git alone: 1,057 ms) |
| 2 | `Space g l` log, 150k commits | 460–500 ms | the same | `git log --graph -n 200`: `--graph` forces topological order, which walks every commit when the repository has no commit-graph file (git alone: 488 ms; without `--graph`: 2.9 ms) |
| 3 | `Space f` finder, 76k files | first rows 10 ms, list complete 355 ms (552 ms cold) | the same | the tree is walked again on every open (`rg --files`: 50 ms); per key 18–60 ms |
| 4 | `Space /` search re-run, 2.0 GB | first hit 50 ms, complete 140–150 ms | the same | reading the files; already 0093's parallel search |
| 5 | `Space s` symbols, 15,400-line file | 60–66 ms | the same | 37 ms in the server, 24 ms decoding 4,900 symbols and building rows |
| 6 | `Space s` symbols, 806-line file | 3.5–22 ms | the same | one `textDocument/documentSymbol` round trip (1.4–20 ms) |
| 7 | `Space s`, no language server for the file | nothing: a message | — | there is no second source |
| 8 | `Space g B` blame | 26 ms | 26 ms | — |
| 9 | `Space g g` status | 16–19 ms (51 ms on helix, cold) | 13–16 ms | — |
| 10 | commit → changed files | 2.6–14 ms | — | — |
| 11 | log and file history, 7.7k commits | 38–43 ms | the same | — |

Tree-sitter declaration extraction, for the outline of §4.1: 2.4 ms for the
806-line file, 51 ms for the 15,400-line one — too long for the event loop,
cheap for a background thread.

Two things the measurements showed that were not expected:

- **Symbols are not slow against a healthy rust-analyzer.** On an ordinary file
  the picker is painted 4 ms after the key, also while the server is still
  indexing (`documentSymbol` is answered from the syntax tree). What the issue
  describes is real in three other places: a large file (row 5), a file with no
  server (row 7 — nothing at all), and any server slower than rust-analyzer.
  Every press paid the whole round trip again, because nothing was kept.
- **A found defect, not fixed here.** A `documentSymbol` reply with more than
  about 6,500 symbols exceeds the client's JSON structure bound
  (`frame_limits::MAX_STRUCTURE_TOKENS`), the connection is dropped and the
  server restarts: `Space s` on a 33,000-line generated file kills
  rust-analyzer every time. The bound is a memory guard for all replies;
  raising or streaming it is its own change (§8). Until then a server is not
  asked *unasked* about a buffer of more than 5,000 lines (§4.1): a
  background request must never be what costs the user their server.
  *Fixed by [0101](0101-history-and-limits.md) §4: the bound is a 512 MiB
  decode budget, an over-budget reply fails only its request, and the
  5,000-line stopgap is gone.*

## 2. What is cached, and what is not

**Cached**

1. **Document symbols** (rows 5–7) — landed. The list is kept per document, per
   source, per buffer revision, and is fetched ahead of the key. A file with no
   server gets an outline from the linked tree-sitter grammar.

**Chosen by the numbers, designed, not landed**

2. **Git log and file history** (rows 1–2): the slowest flows measured. §4.2
   says how they fit the core and why they are not in this change.

**Not cached, and why**

- **The finder's file list** (row 3). It streams: the first rows are on screen in
  10 ms and Enter opens the top row of what has loaded (0093 §3.1). A kept list
  is only correct while a filesystem watch is live and has dropped nothing, and
  the list the watch would validate is 76k paths the walk re-reads in 0.35 s.
  The honest fix is at the source — the walk is seven times slower than
  `rg --files` on the same tree — and belongs to 0093 §9 ("a persistent path
  index — only if a real tree misses §7"). No budget of 0093 §7 is missed.
- **Search results** (row 4). They depend on every byte under the root. There is
  no cheap exact key, and 140 ms for 2 GB is the cost of reading the files.
- **Status, blame, commit files** (rows 8–10): 3–26 ms. A cache would add an
  invalidation rule and save nothing a person can see.
- **Workspace symbols** (`Space S`): it already keeps a symbol index and a
  project catalog per watched root (`strop-picker/src/source/cache.rs`,
  validated by `(length, mtime)` on every reuse). It is not moved onto the new
  core: its entries are per path inside one index, not whole values, and its
  measured cost is the tree walk it shares with row 3.
- **A project-wide symbol index for files that are not open.** `Space s` only
  ever lists the current buffer, so nothing reads it. This is also why no key
  is "path plus mtime": every entry of this plan belongs to an open buffer,
  whose revision is exact where an mtime is a guess.

The two older bounded stores in the engine (picker previews, delimiter matches)
stay as they are: they are rings of the last few values, not keyed caches with
an invalidation rule, and nothing measured asks for them to change.

## 3. The core

One module, `strop_kernel::cache`, proved once. It knows nothing about symbols
or git: an entry is a **subject** (what it is about), a **producer** (who
computed it) and a **version** (which state of the subject it describes), plus
a cost and a last-use tick. All are plain integers.

```rust
pub struct Slot { live, subject: u64, producer: u64, version: u64, cost: usize, used: u64 }

find(&[Slot], subject, producer, version) -> Option<usize>      // exact match on all three
touch(&mut [Slot], index, now)                                  // a hit is a use
retire(&mut [Slot], subject, producer, by_subject, by_producer) // invalidate
place(&mut [Slot], subject, producer, version, cost, limit, now) -> Option<usize>
```

The table is a fixed slice the owner allocates once, so the number of entries
can never exceed its length; `place` refuses a value whose cost is over
`limit`, so the total cost can never exceed `length × limit`. `place` first
retires the pair's previous entry, then takes a dead slot, and only when none
is dead the least recently used one.

**Validity is equality, never a flag.** There is no "dirty" bit that an edit has
to remember to set. An edit moves the buffer to a new revision; a lookup asks
for the current revision; an entry for any other revision cannot match. Every
invalidation the issue lists reduces to that or to `retire`:

| Event | What happens |
|---|---|
| Edit, undo, redo, reload after an external change | the buffer's revision moves (it is monotonic: `Buffer::epoch`), so the old entry no longer matches; the next build replaces it |
| Save | nothing: the text and the revision are unchanged, the entry is still right |
| Language server restart or failure | the server's entries are retired; a restarted server has a new id, so they could not match anyway |
| Buffer closed | the document's entries are retired (a document id is never reused: its generation moves) |
| Table full | the least recently used entry is replaced |

### 3.1 Theorems (`proofs/Kernel/Cache.lean`, over the Aeneas translation)

- `find_spec` — a hit is a live slot whose subject, producer **and version**
  equal the ones asked for, and it is the first such slot; a miss means no live
  slot matches. *An entry for another revision is never served.*
- `retire_spec` — every covered slot is dead afterwards, every other slot is
  untouched. `find_after_retire` — after retiring a subject (or a producer), a
  lookup for it misses, whatever the version. *Invalidate, then lookup, misses.*
- `place_spec` — a value over the limit is refused and the table is unchanged;
  otherwise the table is `Placed`: the pair's old entry retired, the victim
  slot chosen, the new entry written there. `placed_entries` — the new entry is
  in the returned slot under its exact key, and every other live slot is
  unchanged and belongs to another pair. `placed_bounded` — every live entry
  still costs at most the limit; `total_cost_le` — such a table costs at most
  `length × limit`. *Size ≤ bound after every operation.*
- `victim_spec`, `place_evicts_oldest` — when `place` overwrites a live entry
  of another pair, no slot was dead and no live entry had been used less
  recently. *Eviction is least-recently-used, and only under pressure.*
- `find_after_place` — the entry just placed is found under its key, and a
  lookup for the same pair at any other version misses. *A refresh replaces;
  it never leaves the previous revision's entry beside the new one.*
- `touch_spec` — a use changes one slot's tick and nothing else.

All eleven are in `Audit.lean`'s claims; their axioms are Lean's standard
three. Eight mutants in `mutants.py`, each breaking the theorem named: a lookup
that ignores the version, a lookup that serves a dead slot (`holds_spec`); an
invalidation by subject-and-producer that covers either (`selects_spec`); a
retire that leaves the slot live (`retire_loop_spec`); a use that revives a
slot (`touch_spec`); a victim choice that takes the most recently used
(`victim_loop_spec`); a `place` without the cost check, and one that does not
retire the pair first (`place_spec`).

### 3.2 What Lean does not cover, and what covers it

- **The value store.** The kernel proves facts about slots; the engine keeps
  the values in a parallel vector (`editor/cache.rs`, `Cache<V>`: one file, the
  only caller of the kernel module). That the value at an index is the one
  placed with that slot's key, and that a dead slot's value is dropped rather
  than hidden, is checked by a model test: 64 seeded sequences of 400 random
  operations against the obvious cache, the same answers at every step.
- **The race** — a build that finishes after the text or the server moved
  (§5.1).

## 4. The flows

### 4.1 Symbols — `editor/symbols/` (landed)

State lives in `Editor::analysis.symbols`, so the edit, close and stop
lifecycle `AnalysisState` already has covers it; no field and no `impl` block
was added to `Editor`.

- **Key.** Subject: the document (slot and generation). Producer: the language
  server's id, or a reserved id for the syntax outline. Version: the buffer
  revision.
- **Source.** The server, when the document is open on one that is ready.
  Otherwise the outline, when the file's grammar has a symbols query (Rust,
  Python, C, C++, Lua). Otherwise nobody, and the press says why, as before.
- **Precompute.** After every event the owner makes one check: is the current
  document, at its current revision, from its current source, already decided?
  If not, and the editor is not in Insert mode, and no build is in flight, it
  starts one — a `documentSymbol` request, or an extraction on the outline
  thread from a frozen rope. So symbols are fetched when a file is opened, when
  its server becomes ready, when the buffer is switched and when an edit is
  finished — not once per typed character. A buffer of any length is asked
  about (0101 §4.2: a reply too large to decode fails that one request; the
  5,000-line stopgap this plan shipped with is gone).
- **`Space s`.** A kept list opens the picker in the same turn of the event
  loop, with no request. A build already running for exactly this document,
  revision and source is adopted: the picker opens when it lands. Otherwise
  the press starts its own build, ahead of any background one.
- **Never stale, never swapped.** Rows from another revision are never shown:
  their lines would be wrong. One press shows one source; the list is not
  replaced under the bar by a second one arriving later. An outline says it is
  one ("symbols from the syntax tree: no language server serves this file").
- **Bounds.** 64 lists of at most 20,000 symbols each. A longer list is shown
  and not kept.

### 4.2 Git history (designed, not landed; superseded by 0101)

[0101](0101-history-and-limits.md) §2 fixed these flows at the source
instead — a private commit-graph and streamed rows — and keeps no rows, so
nothing below landed. It stays as the design should a row cache ever be
asked for.

- **Key.** Subject: repository + scope (the whole log, or one file). Version: a
  counter the owner moves whenever a job reports a different `HEAD` commit for
  that repository. Line-range history (`-L`) depends on the buffer too and is
  not cached.
- **Validated in the job.** `Space g l` starts its job as it does now. The job
  first reads `HEAD` (`git rev-parse`: 1.1 ms on the 150k-commit repository).
  When that is the commit the kept rows were computed at it returns
  "unchanged" and the rows come from the cache; otherwise it runs the log and
  the result replaces the entry. The key is checked at the moment of use, on
  the worker, so a commit made outside the editor a second ago is seen. The
  press never waits for more than it does today; nothing is precomputed.
- **Why it is not in this change.** A log row says "3 days ago" (`%ar`), and
  that text goes stale with the clock while `HEAD` stands still. Keeping rows
  correctly means asking git for the commit time and wording the age when the
  rows are published, by git's own rules — including git's translations of
  those words, which strop shows today because git prints them. That is a
  fidelity change to the log surface with its own differential test against
  `git`, and it belongs with the git work that follows 0.62 (history on huge
  repositories), not behind a symbols change. Until then both flows cost what
  §1 says. Nothing was added for them.

## 5. The non-blocking contract

1. **Nothing waits for a cache.** A build is a job that posts its result onto
   the event loop. The input path, pickers, search, git, LSP requests and saves
   never wait for one, and no lock is held across one (there are no locks: the
   cache has one owner, on the event-loop thread; a build owns a frozen rope).
2. **A cold cache means the ordinary path.** A press that misses starts the
   request it always started, or adopts the build that is that very request.
3. **Builds cannot starve other work.** At most one symbols build is in flight
   in the whole editor. The outline runs on its own thread — never the
   display-analysis thread, so highlighting does not queue behind a parse — one
   job at a time. A build is started only after the event loop has handled an
   event and found the list missing, never from inside a keystroke's dispatch,
   and not in Insert mode. A press does not queue behind a background build:
   it adopts it when it is the same work, and otherwise takes its place.
4. **Superseded and stale work is dropped.** An edit cancels the outline being
   built (a flag the parse and the query loop poll). A result is accepted only
   when it is the build the owner is waiting for **and** its document still
   exists **and** its revision is the buffer's current revision **and** (for a
   server) that server is still the document's. It is stored under the
   revision it was computed for, never under the revision current at delivery.
5. **Bounded.** §3: entries × limit, fixed at construction.

What the contract does not promise: the outline thread runs one job at a time
and cancellation is cooperative, so a parse that ignored its flag would hold
the next outline — and only the next outline — behind it.

### 5.1 How the race is checked

The race is one of message order: results arrive on the event loop after
edits, restarts and closes that were handled before them. There is no shared
memory between a build and the cache, so Loom has nothing to interleave. A
TLA+ model would restate the acceptance rule of 5.4 in a second language and
then need its own argument that it matches the Rust (0086 §2.2), and under the
survival rule it would have to show a catch.

Instead the shipped acceptance code is explored exhaustively.
`editor/symbols/race_tests.rs` runs **every** sequence of six steps drawn from
{edit, start an outline build, start a server build, deliver the oldest
result, deliver the newest result, restart the server, close and reopen the
document, supersede the build} — 262,144 sequences — against the real
`SymbolStore`, and checks after every step that whatever a lookup serves is
exactly the list of the current text from the producer asked.

The explorer is calibrated the way the proofs are. Two wrong acceptance rules
written in the test must each produce a counterexample, or the test fails:
filing a result under the revision current when it arrives, and filing an
outline's result as the server's. (Two other wrong rules were tried during the
design and are *not* calibrations, because with exact keys they cannot be
observed: accepting a result without the revision check, and accepting one
from a retired server. Both store a correct list under a key nobody asks for
again. They waste a slot; they cannot serve a wrong row. The checks stay in
the rule for that reason — the slot — and `find_spec` is what makes them
harmless.)

## 6. Tests

- Kernel unit tests; the Lean theorems; the mutants.
- `Cache<V>` against the obvious cache; values are dropped with their slots.
- **A stale list is never served after an edit**
  (`a_stale_list_is_never_served_after_an_edit`): open symbols, close, insert a
  line above a symbol, press again before the new build lands — no picker, no
  old row; when it lands the row carries the new line. And the same with no
  event-loop turn between the edit and the press.
- A second press at the same revision owns no request; after an edit the old
  row is not shown.
- A reply for text that has changed is dropped and not kept. A restarted
  server's lists are gone, a reply in flight when it died is nobody's, and the
  outline answers in its place.
- No server: the outline is built ahead of the key and opens without a build.
  No server and no symbols query: the press says so, naming the verb.
- An empty reply names the document and is kept.
- **Non-blocking** (`nothing_waits_for_a_build_that_is_stuck`): with an outline
  build held on a gate, keys edit and move, the finder lists, a save reaches
  the disk — all while the build is still held; released, its list is for text
  that no longer exists and is not kept.
- The exhaustive race exploration and its two calibrations (§5.1).
- `strop --bench symbols`, budgeted (§7); `release-local.sh qualify` runs it.

## 7. Budgets

`strop --bench symbols`: a generated 15,400-line Rust file, 2,800
declarations, no server — the outline, so the scenario is hermetic. It drives
the recorded-action path, where the after-event work runs.

| Operation | Budget (p95) | Measured here |
|---|---|---|
| `Space s` with a kept list: key → rows painted | 10 ms | 2.0–2.7 ms |
| a key while a build is in flight | 5 ms | 0.3 ms |
| the build itself (reported, not budgeted) | — | 34 ms |
| `Space s` straight after an edit: adopts the build (reported) | — | 28 ms |

## 8. Deferred

- ~~Git history (§4.2)~~ — fixed without a cache by 0101 §2.
- ~~The `documentSymbol` reply bound that restarts the server on huge files
  (§1).~~ Done in 0101 §4: `UNASKED_LINES` is deleted.
- A persistent path index for the finder (0093 §9), if a real tree misses its
  budget.
- Symbols for remote (SSH) documents use the same store and the same rule;
  nothing was measured over a real link.

## 9. Implementation record

Before → after, same machine, same corpora, real `rust-analyzer`, forty presses
one second apart while the server indexes:

| `Space s` | Before | After |
|---|---|---|
| 806-line file, every press after the first | 3.5–22 ms, one request each | 2.0–2.6 ms, no request; the list was fetched when the server became ready |
| 15,400-line file, every press after the first | 60–66 ms, one request each | 3.7–3.9 ms, no request. The first press at a revision is the one request, as before (over 5,000 lines nothing is asked unasked) |
| Rust file with no server | a message | outline built in 34–37 ms at open; then 2 ms |
| the press straight after an edit | as above | waits for the one build the edit started (27–60 ms), then shows the new rows |

What landed where:

- `crates/strop-kernel/src/cache.rs` — the core (§3). `proofs/Kernel/Cache.lean`
  — eleven claims; `proofs/mutants.py` — eight mutants; `proofs/Extracted/`
  regenerated.
- `crates/strop-engine/src/editor/cache.rs` — `Cache<V>`.
- `crates/strop-engine/src/editor/symbols/{mod,store,outline}.rs` — the flow,
  the store with its acceptance rule, the outline thread.
- `editor/lsp/state.rs` — `lsp_request_with` split into `lsp_prepare` and
  `lsp_launch`, so a background request is admitted exactly like a user's.
  The `LspEvent::Symbols` arm that opened the picker from the navigation slot
  is gone: every symbols request is a build.
- `strop-syntax` — `DeclExtractor::supports` (a table lookup, no parser
  built) and `SymbolKind::lsp_name`.
- `crates/strop/src/bench/symbols.rs`.

Departures from the first draft of this plan: git history did not land
(§4.2); a press never shows the outline and then the server's list (one
source per press, §4.1), which removed the "show what we have, then refresh"
path the draft considered; and the two calibrations of §5.1 are the ones the
design showed to be observable.

Gates, as run on this machine for this change:

- Host: `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets
  -- -D warnings` and `cargo test --locked --workspace` pass, except
  `differential_vs_nvim`, which fails on the base commit too: the host's nvim
  is 0.9.5 and the corpus needs the gate's pinned one.
- `docker compose run --build --rm test`: passed. Two earlier runs failed in
  `differential_vs_nvim` only — sixteen consecutive cases of one generated
  group, in a `.txt` fixture this change does not touch, each with the file
  unwritten: the harness's `settle 5000` ran out while the machine was building
  for several agents at once (load average 28). The harness does not check
  the editor's exit status, so a timeout reads as a divergence. Worth fixing
  there; not done here.
- `docker compose --profile proofs run --build --rm proofs`: passed — the
  translation matches the committed `proofs/Extracted`, `lake build`, the
  axiom audit (73 theorems, 25 required claims, standard axioms only) and
  20 of 20 mutants rejected by their named theorems, in 158 s.
- `strop --bench symbols`: within both budgets (§7).
- `model`, `ssh-pythonfree` and the release lanes were not run: nothing they
  cover changed.

A binary-level journey (`crates/strop/tests/symbols.rs`) runs the shipped
`strop --headless` with an empty `PATH`: the outline lists, and after an edit
the rows carry the new lines.
