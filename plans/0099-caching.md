# 0099 — Caching: one proved core, and only where the numbers ask for it

- Status: **in force** (2026-11-12). §9 is the implementation record.
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

Two things the measurements showed that were not expected:

- **Symbols are not slow against a healthy rust-analyzer.** On an ordinary file
  the picker is painted 4 ms after the key, also while the server is still
  indexing (`documentSymbol` is answered from the syntax tree). What the issue
  describes is real in three other places: a large file (row 5), a file with no
  server (row 7 — nothing at all), and any server slower than rust-analyzer.
  Every press pays the whole round trip again, because nothing is kept.
- **A found defect, not fixed here.** A `documentSymbol` reply with more than
  about 6,500 symbols exceeds the client's JSON structure bound
  (`frame_limits::MAX_STRUCTURE_TOKENS`), the connection is dropped and the
  server restarts: `Space s` on a 33,000-line generated file kills
  rust-analyzer every time. The bound is a memory guard for all replies;
  raising or streaming it is its own change (recorded in §8).

## 2. What is cached, and what is not

**Cached**

1. **Document symbols** (rows 5–7). The reply is kept per document, per server,
   per buffer revision, and is fetched ahead of the key. A file with no server
   gets an outline from the linked tree-sitter grammar, built in the background.
2. **Git log and file history** (rows 1–2). The rows are kept per repository
   and path, per `HEAD` commit. `HEAD` is the exact key: the commits a log shows
   are a function of it.

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
  core here: its entries are per path inside one index, not whole values, and
  its measured cost is the tree walk it shares with row 3.
- **A project-wide symbol index for files that are not open.** `Space s` only
  ever lists the current buffer, so nothing reads it.

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

find(&[Slot], subject, producer, version) -> Option<usize>   // exact match on all three
touch(&mut [Slot], index, now)                               // a hit is a use
retire(&mut [Slot], subject, producer, by_subject, by_producer) // invalidate
place(&mut [Slot], subject, producer, version, cost, limit, now) -> Option<usize>
```

The table is a fixed slice the owner allocates once, so the number of entries
can never exceed its length; `place` refuses a value whose cost is over
`limit`, so the total cost can never exceed `length × limit`. `place` first
retires the pair's previous entry, then takes a free slot, and only when none is
free the least recently used one.

**Validity is equality, never a flag.** There is no "dirty" bit that an edit has
to remember to set. An edit moves the buffer to a new revision; a lookup asks
for the current revision; an entry for any other revision cannot match. Every
invalidation the issue lists reduces to that or to `retire`:

| Event | What happens |
|---|---|
| Edit, undo, redo, reload after an external change | the buffer's revision moves (it is monotonic: `Buffer::epoch`), so the old entry no longer matches; the next refresh replaces it |
| Save | nothing: the text and the revision are unchanged, the entry is still right |
| Language server restart or failure | the server's entries are retired; a restarted server has a new id, so they could not match anyway |
| Buffer closed | the document's entries are retired (a document id is never reused: its generation moves) |
| `HEAD` moves (commit, switch, pull) | the repository's version moves; every history entry stops matching |
| Table full | the least recently used entry is replaced |

### 3.1 Theorems (`proofs/Kernel/Cache.lean`, over the Aeneas translation)

- `find_spec` — a hit is a live slot whose subject, producer **and version**
  equal the ones asked for, and it is the first such slot; a miss means no live
  slot matches. *An entry for another revision is never served.*
- `retire_spec` — every selected slot is dead afterwards, every other slot is
  untouched. `find_after_retire` — after retiring a subject (or a producer), a
  lookup for it misses, whatever the version. *Invalidate, then lookup, misses.*
- `place_spec` — the table keeps its length; a value over the limit is refused
  and nothing changes; an accepted value is in the returned slot with its exact
  key; every other live slot is unchanged and belongs to another pair; and
  `Bounded` (every live slot's cost ≤ limit) is preserved. `total_cost_le` —
  a `Bounded` table costs at most `length × limit`. *Size ≤ bound after every
  operation.*
- `place_evicts_oldest` — when `place` overwrites a live slot of another pair,
  no slot was free and no live slot was used less recently. *The eviction is
  least-recently-used, and only under pressure.*
- `find_after_place` — the value just placed is found under its key, and a
  lookup for the same pair at any other version misses. *A refresh replaces;
  it never leaves the previous revision's entry beside the new one.*
- `touch_spec` — a use changes one slot's tick and nothing else.

Each is in `Audit.lean`'s claims. Mutants in `mutants.py` (each must break its
named theorem): a lookup that ignores the version; a lookup that ignores
`live`; a retire that skips the first slot; a `place` without the cost check; a
`place` that does not retire the pair first; a victim choice that takes the
most recently used slot; a `touch` that also revives the slot.

### 3.2 What Lean does not cover, and what covers it

- **The value store.** The kernel proves facts about slots; the engine keeps
  the values in a parallel vector (`editor/cache.rs`, `Cache<V>`: 1 file, the
  only caller of the kernel module). That the value at an index is the one
  placed with that slot's key is checked by a model test: random operation
  sequences against a plain map, the same answers at every step.
- **The race** — a build that finishes after the text, the server or `HEAD`
  moved (§5).

## 4. The two adapters

Each flow has one owner module and adds no field to `Editor`.

### 4.1 Symbols — `editor/symbols/`

- **Key.** Subject: the document. Producer: the language server's id, or a
  reserved id for the syntax outline. Version: the buffer revision.
- **Precompute.** After every event the owner makes one O(1) check: is the
  current document, at its current revision, already cached or being built? If
  not, and the editor is not in Insert mode, it starts one build:
  - a ready server serves the file → a background `documentSymbol` request;
  - otherwise, if the file's grammar has a symbols query (Rust, Python, C,
    C++, Lua) → a tree-sitter extraction on the outline thread, from a frozen
    rope.
  So symbols are fetched when a file is opened, when the server becomes ready,
  when the buffer is switched, and when an edit is finished — not once per
  typed character.
- **`Space s`.** A valid entry opens the picker in the same turn of the event
  loop, with no request. A miss takes the path it took before this plan (ask
  the server; the picker opens on the reply) and the reply is kept. A build
  already in flight for this revision is adopted rather than asked for twice.
  With no server, the outline is what opens.
- **Never stale, never swapped.** Rows from another revision are never shown:
  their lines would be wrong. The list is not replaced under the bar by a
  second source arriving later; one press shows one source.
- **Bounds.** 64 entries, at most 20,000 symbols each; a larger reply is shown
  and not kept.

### 4.2 Git history — `editor/git_memory/history.rs`

- **Key.** Subject: repository + scope (whole log, or one file). Version: a
  counter the owner moves whenever a job reports a different `HEAD` commit for
  that repository. Line-range history (`-L`) is not cached.
- **Validated in the job.** `Space g l` starts its job as before. The job first
  reads `HEAD` (`git rev-parse`, 1 ms). When that is the commit the kept rows
  were computed at, it returns "unchanged" and the rows are published from the
  cache; otherwise it runs the log and the result replaces the entry. The key
  is checked at the moment of use, on the worker, so a commit made outside the
  editor a second ago is seen.
- **Ages.** A log row says "3 days ago". Kept text would go stale with the
  clock, so the job asks git for the commit time (`%at`) and the age is worded
  when the rows are published, by the same rules as git's `%ar`
  (differential test against `git` itself).
- **Bounds.** 32 entries of at most 200 rows (the log's existing limit).

## 5. The non-blocking contract

1. **Nothing waits for a cache.** A build is a job that posts its result onto
   the event loop. The input path, pickers, search, git, LSP requests and saves
   never wait for one, and no lock is held across one (there are no locks: the
   cache has one owner, on the event-loop thread; a build owns a frozen rope).
2. **A cold cache means the old path.** A flow that misses does what it did
   before this plan. It never waits for a build that is in flight unless that
   build is the very request it would have sent.
3. **Builds cannot starve other work.** At most one symbols build is in flight
   in the whole editor, on its own thread (never the display-analysis thread,
   so highlighting does not queue behind it) or as one request on a server.
   It is started only when the event loop has handled an event and found the
   entry missing — never from inside a keystroke's dispatch — and not in Insert
   mode. A user request always goes out; it does not queue behind a build.
4. **Superseded and stale work is dropped.** An edit cancels the outline build
   in flight (a flag the extraction polls). A result is accepted only when it
   is the build the owner is waiting for **and** its document still exists
   **and** its revision is the buffer's current revision **and** (for a server)
   that server is still the document's server. It is stored under the revision
   it was computed for, never under the revision current at delivery.
5. **Bounded.** §3: entries × limit, fixed at construction.

### 5.1 How the race is checked

The race is one of message order: results arrive on the event loop after
edits, restarts and closes that were handled before them. There is no shared
memory between a build and the cache, so Loom has nothing to interleave, and a
TLA+ model would restate the acceptance rule of 5.4 in a second language and
then need its own proof of correspondence with the Rust (0086 §2.2; the
survival rule would also want a recorded catch from it).

Instead the shipped acceptance code is explored exhaustively:
`editor/symbols/race_tests.rs` enumerates **every** sequence, up to a fixed
length, of {edit, start build, deliver the oldest result, deliver the newest
result, restart the server, close and reopen the document, look up} against
the real `SymbolStore`, and checks after every step that whatever a lookup
serves is exactly the symbols of the current text from the current producer.
The explorer is calibrated the way the proofs are: two deliberately wrong
acceptance rules written in the test (store under the revision current at
delivery; accept a result from a retired server) must each produce a
counterexample, or the test fails. A checker that cannot catch the bug it is
there for is deleted, not kept.

## 6. Tests

- Kernel unit tests; the Lean theorems; the mutants.
- `Cache<V>` model test against a map (values follow their slots; eviction and
  retirement drop the value, not only the slot).
- **A stale cache is never served after an edit**: open symbols, close, edit a
  line above a symbol, open symbols again — the row carries the new line, and
  no row of the old revision is shown at any frame in between.
- A second `Space s` at the same revision sends no request (the fake server
  counts them); after an edit it sends exactly one.
- Server restart: the old server's entries are gone; `Space s` asks the new one.
- No server: the outline opens; an edit rebuilds it; a grammar without a
  symbols query still says so in words.
- **Non-blocking**: with an outline build held on a gate (stuck on purpose),
  keys still edit and move, the finder and search still open and list, a save
  completes, and `Space s` on another buffer with a server still answers;
  releasing the gate after an edit drops the result.
- The exhaustive race exploration and its two calibrations (§5.1).
- Git: a second `Space g l` publishes without running `git log` (the job says
  "unchanged"); a commit made by `git` outside the editor is in the next log;
  ages match `git log --format=%ar` for commits of fixed dates.
- `strop --bench symbols`: budgeted like 0093 §7.

## 7. Budgets

`strop --bench symbols` (a generated 15,400-line Rust file, 4,900 symbols, no
server — the outline tier, so the scenario is hermetic):

| Operation | Budget (p95) |
|---|---|
| `Space s` with a warm cache: key → rows painted | 10 ms |
| a keystroke in a buffer while a build is in flight | 5 ms |

## 8. Deferred

- The `documentSymbol` reply bound that restarts the server on huge files (§1).
- Warming the log when a repository is opened, so the first `Space g l` on a
  huge history is instant too. The first open still pays git's 0.5 s.
- A persistent path index for the finder (0093 §9), if a real tree misses its
  budget.
- Symbols for remote (SSH) documents use the same store; nothing was measured
  over a real link here.

## 9. Implementation record

(filled in as the work lands)
