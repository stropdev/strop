# 0101 — History on huge repositories, the LSP reply bound, and collections that close what they opened

- Status: **proposed** (2026-11-12). Investigation and recommendation only:
  nothing here is implemented. §7 is the work order for whoever executes it.
- Source: the three follow-ups 0.63 left — [0099](0099-caching.md) §4.2 (git
  history) and §8 (the `documentSymbol` reply bound and its `UNASKED_LINES`
  stopgap), and [0097](0097-collect.md) §6 / [0094](0094-change.md) §5 (the
  files a collection loaded stay open).
- Kernel: **no change proposed.** None of the three needs a new decision in
  `strop-kernel`; §6 says why, per part. No theorem and no mutant is added.

## 1. Part A — measurements

Release build of `476f9d9` (0.63.0), this machine (32 cores, warm page cache),
git 2.43.0. Two generated repositories, each 2,000 files: **big**, 150,000
linear commits, and **bigm**, the same with a merge of a one-commit side
branch every 50 commits (152,999 commits). Neither has a commit-graph file:
`git fast-import` does not write one, and neither does a fresh `git clone`
until the first `gc`.

### 1.1 Git alone, no commit-graph

Median of 7 runs. "First" is the time to the first output line, "done" to exit.

| Command (`--format` as shipped, `-n 200`) | big: first / done | bigm: first / done |
|---|---|---|
| `log --graph` (shipped, `Space g l`) | 452 / 468 ms | 657 / 675 ms |
| `log` (no `--graph`) | 1.4 / 6.7 ms | 1.1 / 8.3 ms |
| `log --graph -- file` (shipped, `Space g h`) | 1,118 / 1,159 ms | 920 / 959 ms |
| `log -- file` (no `--graph`) | 12.8 / 923 ms | 12.5 / 874 ms |
| `rev-parse HEAD` | 1.1 ms | 1.0 ms |

0099 §1 holds: `--graph` implies topological order, and without generation
numbers git must walk every commit before it can print the first one. The
file history has a second cost on top: the file was touched 75 times in
150,000 commits, so the walk diffs trees all the way to the root to learn
there is no 76th.

### 1.2 What a commit-graph does

| Variant | `log --graph` done | `log --graph -- file` first / done | Write cost |
|---|---|---|---|
| none (shipped) | 468 ms | 1,118 / 1,159 ms | — |
| `commit-graph write --reachable`, in the repository | 6.7 ms | 15.9 / 624 ms | 0.49 s, 9.0 MB |
| … `--changed-paths` (Bloom filters), in the repository | 6.8 ms | 6.3 / 194 ms | 2.26 s, 10.2 MB |
| the same file in a **private directory outside the repository**, named by `GIT_ALTERNATE_OBJECT_DIRECTORIES` | 7.3 ms | 6.4 / 198 ms | 2.23 s; repository files byte-for-byte unchanged |
| private graph 5,000 commits behind `HEAD` | 16.8 ms | 23.8 / 219 ms | refresh 0.61 s (filters are reused) |

Three facts the design rests on, each checked on copies of the repositories:

- **Git reads a commit-graph from an alternate object directory.**
  `GIT_ALTERNATE_OBJECT_DIRECTORIES=<dir>/objects git commit-graph write
  --reachable --object-dir=<dir>/objects` writes there and nowhere else, and
  the same variable on `git log` makes git use it. Nothing in the user's
  `.git` is created, modified or locked.
- **A stale or wrong-for-this-history graph cannot change the answer.** The
  file is an index from commit id to parents, dates and a generation number;
  commit ids are content addresses. With a graph 5,000 commits behind, the
  output of `log --graph -n 6000` and of the file history is byte-identical
  to the output without it. With a graph that names 1,000 commits the
  repository has since reset away and pruned, plus two new commits on the
  cut, both outputs are again identical and git prints nothing on stderr.
  (Only `git fsck` complains about such a graph, and strop never runs
  anything but its own history reads with the variable set.)
- **The repository's own graph shadows the private one.** Git loads the
  first commit-graph it finds, its own object directory first. A repository
  with its own graph covering one commit and a complete private graph took
  394 ms: the private file was not read. So strop builds nothing for a
  repository that has a graph of its own; that file is git's to refresh
  (`gc` does it by default since git 2.24).

With a graph, `--graph -- file` also **streams**: the first row is out in 6 ms
and the rest follow as the walk finds them.

Cancelling a write with SIGTERM left no file behind. The editor cancels with
SIGKILL (`strop_core::process`), which can leave git's `commit-graph.lock`;
§2.1 builds in a scratch directory and renames for that reason.

### 1.3 The editor, key to last changed frame

The real binary under `strop headless --log`, six presses each, 0099 §1's
method. "Private graph" is the shipped binary with
`GIT_ALTERNATE_OBJECT_DIRECTORIES` set in its environment, which is exactly
what §2.1 would do for these two commands.

| Flow | Shipped | With a private graph |
|---|---|---|
| `Space g l`, big | 424–436 ms | 8.8–13.0 ms |
| `Space g l`, bigm | 448–495 ms | 8.8–11.4 ms |
| `Space g h`, big | 869–914 ms | 232–242 ms |
| `Space g h`, bigm | 880–895 ms | 224–238 ms |

(0099 measured 1,120–1,170 ms for the file history on its own generated
repository; this one is the same shape and a little faster. The log matches.)

## 2. Part A — decisions

### 2.1 Fix it at the source: a private commit-graph. The user's repository is not touched

Strop keeps a commit-graph for a repository **in its own state directory** and
points its history reads at it. Plainly: strop does **not** write into the
user's `.git`, not the graph, not a lock, not a config key.

- **Where.** `<state root>/strop/graphs/<key>/objects/info/commit-graph`, the
  state root being `session::state_root()` (`XDG_STATE_HOME`, else
  `~/.local/state`). `<key>` is a hash of the canonical path of the
  repository's common object directory (`git2::Repository::commondir()` +
  `objects`), so worktrees of one repository share a graph. No state root, no
  graph: everything behaves as it does today.
- **Use.** Only `Space g l` and `Space g h` (`memory::log_graph_range`) run
  git with `GIT_ALTERNATE_OBJECT_DIRECTORIES` naming that directory, and only
  when it exists, the repository is local, and the variable is not already
  set in strop's environment (a user who set it means something by it).
  Commit, switch, blame, status and every other git run are unchanged.
- **Build.** A background job, started from the after-event hook (where
  `symbols::warm` runs, `editor/trace/drive.rs`), never from a key's
  dispatch. It starts when all of these hold: a history job on a local
  repository just finished and its git run took longer than **100 ms**; the
  repository has no commit-graph of its own (`objects/info/commit-graph` or
  `objects/info/commit-graphs/`); no build is in flight anywhere in the
  editor; and no build was already made for this repository at this `HEAD`
  in this session. The trigger is the measurement itself, so a small
  repository never pays anything and a graph that has fallen far behind is
  refreshed the first time it shows.
- **Two steps, each installed when it finishes.** First `commit-graph write
  --reachable` (0.5 s here): the log is fixed. Then the same with
  `--changed-paths` (2.3 s here, reusing step one): the file history gets its
  filters. Each step writes into `<key>/build-<pid>/objects`, seeded with a
  copy of the installed file so filters are reused, and the result is
  renamed over the installed file. A reader sees the old file or the new
  one; a killed build leaves a scratch directory the next build removes; two
  editors on one repository cannot corrupt each other. Deadline 120 s per
  step. An old git that refuses an option fails the step, the failure is
  remembered for the session, and nothing else changes.
- **Bounded.** At most 16 repositories' graphs are kept; a build that would
  make a seventeenth removes the least recently used directory first (the
  directory's mtime is touched when a history read uses it). About 70 bytes
  per commit: 10 MB for 150,000 commits.
- **The non-blocking contract (0099 §5), point by point.** No flow waits for
  a build: a press during one runs git as it does today and the next press
  after it is fast. One build in the whole editor, a child process on its own
  worker thread, so it holds no queue a user request uses. It is cancelled
  when the editor quits and when its repository is no longer any open
  buffer's. Its completion carries the repository and `HEAD` it was started
  for and is dropped when either no longer matches (the file is still
  installed: it is correct for any history, §1.2). Nothing is kept in memory
  but one small record per repository.

**Why this does not go through `strop_kernel::cache`.** That core decides
which kept *answer* may be served. Here strop keeps no answer and decides no
validity: git computes every row on every press and is the only reader of
the file, which is keyed by content address. The wrong decision about when to
build costs time, never a wrong row, and the differential test of §5.1 is
what checks it. (Open question 1 asks the maintainer to confirm this reading
of "any cache goes through the proved core".)

### 2.2 Stream the rows

After §2.1 the file history still takes 230 ms to *complete* on 150,000
commits and grows with the history, while git has the first row out in 6 ms.
So the history job streams: `GitExec` gains a streamed run over the existing
`strop_core::process::stream_with`, the worker splits records and posts rows
in pages (the first as soon as 50 rows or 30 ms have passed, then every
30 ms), and the surface appends each page. Rows are only ever appended, in
git's order, so nothing above the cursor moves and no row is replaced. The
same job serves the log; there it is one page.

Without a graph (the first slow press, or a repository strop may not build
for) streaming gains nothing with `--graph`: git prints nothing until the
walk is done. That is accepted, not worked around (§3, "drop `--graph`").

### 2.3 No row cache, so no age wording

Not cached, by 0099's own rule (§2 there: "a cache would add an invalidation
rule and save nothing a person can see"):

- the log is 9–13 ms after §2.1 — faster than status (13–19 ms), which 0099
  declined to cache;
- a kept file history would be hit only by a second press on the same file
  at the same `HEAD`; the first press, and every press after any commit,
  would cost the same as today. Streaming puts the first rows on screen in
  the first tens of milliseconds on every press;
- and it would bring the stale-age problem with it. Rows carry `%ar`; keeping
  them means asking for `%ct` and wording ages ourselves, in git's rules and
  in git's translations (strop does not set `LC_ALL`, so a German user sees
  `vor 3 Tagen` today). That is a real fidelity risk taken on to save
  nothing measurable.

So **the visible row format does not change** and nothing about ages changes.
If the maintainer wants the row cache anyway, 0099 §4.2 is still the design
and it needs no kernel change (subject: repository and scope; producer: a
constant; version: a counter moved when a job reports another `HEAD`); the
cost is the age wording and its differential test against `git`.

### 2.4 What is left slow, said plainly

- The **first** history press on a large repository with no commit-graph is
  as slow as today (430–900 ms here), once per repository per machine. The
  build it triggers takes seconds; presses during it are also slow.
- A repository with a commit-graph of its own that is far behind: strop
  cannot help (§1.2). `git gc` or `git commit-graph write --reachable` does.
- SSH repositories: unchanged. The graph would have to be written on the
  host.
- File history completes in 230 ms per 150,000 commits; only its first rows
  are fast.

## 3. Part A — rejected

- **Writing the commit-graph into the user's repository.** Same speed, and
  the user's own `git log --graph` would gain too. But it is a write into
  `.git` from a command that reads; it takes `commit-graph.lock`, so a
  concurrent `git gc` or `fetch` of the user's can fail on strop's account;
  it fails on read-only checkouts; and a SIGKILLed build would leave a lock
  in the user's repository. The private file has none of these and costs
  10 MB.
- **Dropping `--graph`** (2.9 ms without a graph file). It changes the row
  text and the order (date order, not topological), and for a history with
  merges it removes the one thing the graph column says.
- **Showing a plain log first and swapping the graph in.** Rows would be
  replaced and reordered under the cursor (0099 §4.1: "never swapped").
- **Building the graph when a repository is opened**, before any history
  press. It needs a guess at "large" (pack index size), and spends seconds
  of CPU for users who never open the log. See open question 2.
- **`git rev-list --count` or libgit2 revwalk to draw the graph ourselves.**
  A second implementation of git's graph renderer, to be kept equal to git's
  forever.
- **The row cache** (§2.3).

## 4. Part B — the LSP reply bound

### 4.1 What the bound protects against

`async-lsp` 0.2.4 reads a frame as: allocate `Content-Length` bytes, then
`serde_json::from_slice` into an untagged enum whose payload is a
`serde_json::Value`. An untagged enum is first buffered as serde's private
`Content` tree, then converted, so two trees are alive at the peak. The
workspace builds `serde_json` with `preserve_order`, which makes a `Value`
72 bytes. Measured with a counting allocator on that exact decode path:

| Body | Tokens (`{ [ , :`) | Peak heap while decoding | Per token | × body |
|---|---|---|---|---|
| `documentSymbol`, 1,000 symbols (296 KB) | 43,006 | 7.4 MB | 173 B | 25 |
| 6,500 symbols (1.95 MB) | 279,506 | 48.5 MB | 174 B | 25 |
| 20,000 symbols (6.05 MB) | 860,006 | 150 MB | 175 B | 25 |
| 100,000 symbols (30.6 MB) | 4,300,006 | 747 MB | 174 B | 24 |
| semantic tokens, 1,000,000 integers (3.0 MB) | 1,000,009 | 141 MB | 141 B | 47 |
| flood `[[0],[0],…]`, 262,144 tokens | 262,152 | 90 MB | **344 B** | 172 |
| a full 32 MiB frame of `[[0],[0],…]` | 16.8 M | **4.9 GB** | 292 B | 146 |

So the frame bound alone (32 MiB) admits a reply that costs 4.9 GB to decode;
`MAX_STRUCTURE_TOKENS` is what stops that, and it is needed. What is wrong
with it is twofold:

1. **It is a count, and the thing being bounded is memory.** 262,144 tokens
   was not derived from anything. It cuts an honest reply at 6,500 symbols
   (0099's observation: 6,500 symbols is 279,506 tokens) while the symbol
   store is built to keep 20,000 (`SYMBOL_LIMIT`).
2. **Going over it kills the connection.** The guard is a pass-through
   reader: by the time the count is exceeded the header and half the body
   have been forwarded, and the only thing left is an I/O error, which ends
   the main loop and restarts the server. One oversized *reply* should fail
   one *request*.

### 4.2 Decision

**A byte budget for decoding, derived from what must fit, and an over-budget
reply answered as an error instead of dropping the connection.**

- `frame_limits.rs` states the bound as memory:
  `MAX_DECODE_BYTES = 512 MiB` and `DECODE_BYTES_PER_TOKEN = 384`; a frame is
  over budget when `tokens × 384 > 512 MiB` (1,398,101 tokens).
  - 384 is the measured worst case, 344 B per token, with a tenth of margin.
    A test with a counting allocator holds it there (§5.2): no shape in its
    adversarial list may decode above it.
  - 512 MiB is the smallest power of two that admits the largest reply a
    strop consumer keeps: 20,000 symbols are 860,006 tokens, 315 MiB at the
    worst-case rate. A test ties the two constants together so one cannot
    move without the other. An honest reply at the bound really peaks at
    about 230 MiB (174 B per token) and is about 32,500 symbols — a file of
    some 100,000 lines at the density 0099 measured.
  - `MAX_STRUCTURE_TOKENS` is deleted, not raised.
- The guard **holds a frame until its last byte is counted**, then releases
  header and body together. This is one extra copy of the body (bounded by
  the 32 MiB frame bound) and nothing a person can measure on a completion
  reply. With the whole frame in hand an over-budget one is not forwarded:
  - a **response** (a top-level `id` and no top-level `method`) is replaced
    by `{"jsonrpc":"2.0","id":<the same id>,"error":{"code":-32603,
    "message":"reply too large to decode: N tokens, limit M (512 MiB)"}}`.
    The request fails through the path every server error already takes; the
    connection and the server live;
  - a **notification** is dropped and traced, and the editor says once per
    server that a notification over the bound was ignored;
  - a server-to-client **request** over the bound still ends the connection:
    there is no honest way to answer a request that could not be read.
  The scanner already walks the bytes; it gains a depth counter and captures
  the top-level `id` value (at most 128 bytes; longer is treated as a
  request).
- `UNASKED_LINES`, `unasked()` and their test are deleted. With an oversized
  reply now costing one failed background request, and `settled` already
  preventing a second ask at the same revision, the stopgap has nothing left
  to protect.

### 4.3 Rejected

- **Raising the number.** It would still be a count nobody can defend and
  still kill the server one step further out.
- **A bound per method.** The reader sees bytes, not methods: a reply names
  only its id, and the id may come after the result. Matching ids to methods
  in the byte guard duplicates the client's request table in the one place
  that must stay simple. And the decoder is the same for every method, so
  the memory a token costs does not depend on the method.
- **Stream-parsing into the typed reply.** This is the real fix for the 25×:
  100,000 typed symbols are 22 MB where their `Value` tree peaks at 747 MB.
  But `async-lsp`'s `MainLoop` owns both the framing and the decode; doing
  it means replacing the client's transport. Deferred (§8) with the number
  that argues for it.
- **Keeping the pass-through reader and only changing the bound.** Then
  deleting `UNASKED_LINES` would let a background request kill the server on
  a 100,000-line file, on every revision.

## 5. Tests (the contract)

### 5.1 Part A

`strop-git` (real `git` in a tempdir, as the crate's tests do now):

- **Differential, the load-bearing one**: for a generated history with
  merges, `log_graph_range` returns the same rows with no private graph,
  with a fresh one, with one built 200 commits ago, and with one that names
  commits since reset away and pruned (`gc.writeCommitGraph=false`).
  Whole log and one file.
- The build leaves every file under the repository's `.git` unchanged
  (names, sizes, mtimes), and installs by rename: a reader started during a
  build gets the old file or the new one.
- A repository with its own commit-graph is reported as "git's" and gets no
  build.
- The streamed run yields the same rows as the captured run, whatever the
  chunk boundaries (rows split mid-record, mid-UTF-8, at the NUL).

Engine (`git_memory/tests/history.rs`):

- A history job slower than the threshold starts one build after the event,
  not during the key; a fast one starts none; a second slow job during a
  build starts none; a remote repository starts none.
- `nothing_waits_for_a_graph_build`: with a build held on a gate, keys move,
  `Space g l` opens and fills, a save reaches the disk.
- A build's completion for a repository no buffer belongs to any more is
  dropped; quitting cancels it.
- Pages: the first page replaces `loading log…`, later pages append; the
  cursor and the rows above it do not move; a page for a closed or recycled
  surface is dropped (the existing `stale_log_results_are_dropped`, extended
  to a second page); the blame dive's focus row is found in whichever page
  carries it.
- The 17th repository evicts the least recently used directory.

Journey (`crates/strop/tests/history.rs`, the real binary, hermetic
`XDG_STATE_HOME`): `Space g l` twice on a generated repository; the frames
are identical and the state directory holds a graph after the first.

`strop --bench history` (generates 150,000 commits with `git fast-import`,
needs `git`; `release-local.sh qualify` runs it):

| Operation | Budget (p95) | Measured here |
|---|---|---|
| `Space g l`, graph present: key → rows | 50 ms | 9–13 ms |
| `Space g h`, graph present: key → first rows | 60 ms | to be measured (git's first row: 6 ms) |
| `Space g h` complete (reported) | — | 225–240 ms |
| first press, no graph (reported) | — | 425–495 / 870–915 ms |
| graph build, both steps (reported) | — | 0.5 s + 2.3 s |

### 5.2 Part B

`strop-lsp`:

- **A huge symbol reply** (`client/tests.rs`, the scripted server): a
  `documentSymbol` reply of `SYMBOL_LIMIT` (20,000) symbols is decoded and
  delivered, and the next request on the same connection is answered.
- **An over-budget reply fails one request**: 40,000 symbols → that request
  ends in an error naming the bound; the next request on the same connection
  succeeds; no `Failed` event, no restart.
- An over-budget notification is dropped and the next frame is read; an
  over-budget server request ends the connection.
- **Calibration** (`tests/decode_cost.rs`, its own test binary because it
  installs a counting global allocator): for each of the shapes of §4.1 and
  the other adversarial ones (`[[0],…]`, `[{"a":[0]},…]`, `[{"a":0},…]`,
  `[0,…]`, `{"k":0,…}`, at sizes on both sides of a capacity doubling), the
  peak heap of the real decode is at most `tokens × DECODE_BYTES_PER_TOKEN`
  plus the body. If a `serde_json` or `async-lsp` upgrade makes decoding
  dearer, this fails and the constant moves.
- `SYMBOL_LIMIT` symbols fit the budget at the worst-case rate (the
  constants cannot drift apart).
- Property test of the guard: random frames, split at random chunk
  boundaries; admitted frames come out byte-identical and in order; a
  replaced response carries the same id; the output is always well-framed.
  The three existing tests in `frame_limits.rs` stay (the flood one against
  the new bound).

Engine (`symbols/tests.rs`): a background build is started for a buffer of
more than 5,000 lines (replacing `a_server_is_not_asked_unasked_about_a_huge_buffer`);
an error reply to a background build changes no message and is not asked
again at that revision.

### 5.3 Part C

`collections/tests/loans.rs`, on the real in-process search over a tempdir:

- **Never visited, never edited**: collect hits in three unopened files,
  `:q` the collection → the three are closed (`Space b` does not list them,
  their LSP documents are closed), the cursor is back where `Ctrl-O` was
  pressed.
- **Visited**: `Enter` on a hit, `Ctrl-O` back, `:q` → that file stays, the
  other two close.
- **Edited**: change a line through the collection, `:w`, `:q` → the edited
  file stays (its undo history is the only way back), the others close.
- **Dirty**: change a line through the collection, `:q` without `:w` → the
  dirty file stays open and dirty, and the message says so:
  `1 file the collection opened has unsaved changes and stays open`.
- A file that was open before the collect is never closed by it.
- Two collections share a source: closing one keeps it, closing the second
  closes it.
- Collecting the same search again (a refresh) closes the sources the new
  list no longer uses.
- A build abandoned mid-load (a second `Ctrl-O`, or every hit changed)
  closes what it had loaded.
- The collection was the only buffer: `:q` lands on one of its files, which
  stays; the rest close; the editor does not quit.
- `:cdo` with a command that changes nothing leaves the files closable; one
  that edits keeps them.
- The truth table of `releasable` (§6.3), as a unit test.

## 6. Part C — collections close what they opened

### 6.1 The rule

A document a collection build loaded in the background carries a **loan**
until the user takes it. When nothing holds a loaned document any more, it is
closed — without moving any cursor.

- **Loaned**: admitted with `OpenIntent::CollectionSource`. A file that was
  already open is not loaned.
- **Taken** (the loan ends, for good) when the document is on screen after
  an event: one check in the after-event hook over the panes. This is what
  "visited" means — `Enter` on a hit, `]q` from a file, `:e`, the finder, a
  split — and it is one site instead of every place a pane is rebound.
  `:cdo` switches to each file and back inside one command, so it does not
  take them.
- **Held** while any collection has an excerpt in it, or a collection build
  is in flight.
- **Released** when not held and still loaned and the buffer's revision is
  the one it was loaded at. A moved revision means it was edited (through
  the collection, by `:cdo`, or reloaded from disk): it stays, because its
  undo history is then the only way back once the collection is gone. A
  dirty buffer has a moved revision by construction, so **a dirty buffer is
  never closed**; the release path `debug_assert`s it, and the close of the
  collection says how many stay open unsaved.

The sweep runs where a holder goes away: a collection closes (`:q`), a
collection is refreshed, a projection is invalidated, a build is abandoned.
Closing a collection lands the cursor first, exactly as today, and sweeps
after; a file the landing put on screen is thereby taken. So `:q` on a
collection never quits the editor because of this change.

### 6.2 The close that moves nothing

`Editor::close_buffer` today is the guards, the teardown of the current
document, and the landing, in one function. It splits into
`release_document(id)` — the teardown alone (terminal, excerpts of other
collections, remote write, analysis, directory filter, LSP, shell, git
requests, blame gutter, collection, review, filesystem view, `docs.remove`,
`mru`, `generation`) for a document no pane shows — and `close_buffer`,
which keeps the guards and the landing and calls it. One teardown, two
callers; no second copy.

### 6.3 Not in the kernel

`releasable(loaned, shown, held, revision_moved, dirty)` is a conjunction of
five facts about one document. A Lean proof of it would restate it. A
truth-table test and the `debug_assert` on `dirty` are the right ring.

### 6.4 Rejected

- **Closing an edited, saved, never-shown file.** It would fix more of
  0091 §4.3 ("36 buffers open"), but it throws away the only undo path for a
  change the user made a minute ago. See open question 3.
- **Per-collection ownership** (the collection that loaded a file closes
  it). A second collection over the same file would lose its excerpts.
- **Taking the loan in `switch_to`.** `:cdo` and other commands switch and
  switch back; and panes are rebound in other places.
- **Asking before closing a dirty file.** It is never closed, so there is
  nothing to ask.
- **The replace review's files** (0094 §5). Same loan, different holder, but
  every file a replace loads is edited by it, so under this rule all stay.
  It needs the answer to open question 3 first; left deferred.

## 7. Implementation steps

Three independent changes; each is one commit with its tests, changelog line
and docs. Order: C (smallest), B, A.

### 7.1 Part C

1. `editor/document/mod.rs`: `Document` gains `loan: Option<BufferRevision>`
   (the revision at load). Extract `Editor::release_document(id)` from
   `close_buffer` (§6.2); `debug_assert!` no pane shows `id`. The file is at
   628 lines: put `close_buffer`, `release_document` and
   `drop_stale_scratch` in a new `document/close.rs`.
2. `editor/io/admission.rs` (the `try_insert` at ~340): set the loan when
   the intent is `CollectionSource`.
3. New `editor/collections/loans.rs`: the pure `releasable`; `take_shown`
   (after-event: clear the loan of every document a pane shows — call it
   beside `symbols::warm` in `editor/trace/drive.rs`); `release_unheld`
   (collect ids as data, then `release_document` each, then the message when
   dirty ones stay).
4. Call `release_unheld` from: `close_buffer` after the landing when the
   closed document was a collection; `build_collection` after a refresh
   replaced an existing collection and on its "nothing collected" return;
   `invalidate_collection_projection`; `open_collection_from_picker` when it
   replaces a pending `collection_build`.
5. Tests of §5.3. Docs: `docs/` page on collections, one paragraph.
   Amend 0097 §6 and 0094 §5 (first bullet: collections done, replace
   still deferred).

### 7.2 Part B

1. `strop-lsp/src/client/frame_limits.rs`: replace the pass-through guard
   with a frame assembler — states `Header`, `Body { remaining, tokens,
   depth, string, top-level key and id capture }`, and an output queue
   `poll_read` drains. Constants as §4.2. The file is 196 lines and will
   roughly double; if it passes ~400 split into
   `frame_limits/{mod,scan,tests}.rs`.
2. `client/process.rs`: the dropped-notification count reaches the editor as
   an `LspEvent` (one message per server); check `trace_io::Observed` still
   sees the bytes the server actually sent.
3. `strop-lsp/tests/decode_cost.rs` with the counting allocator; it needs
   the two constants and a `decode` helper exported under the crate's
   `test-support` feature (or `#[doc(hidden)] pub`), mirroring
   `async-lsp`'s `RawMessage` decode.
4. `strop-engine/src/editor/symbols/mod.rs`: delete `UNASKED_LINES` and
   `unasked`; `warm` uses `source` directly. Replace the test.
5. Tests of §5.2. Amend 0099 §1, §4.1 and §8 (the stopgap is gone).

### 7.3 Part A

1. `strop-git/src/exec.rs`: `GitExec::stream_records(op, argv, envs, cancel,
   consume)` over `stream_with` (local; the remote arm keeps the captured
   run and calls `consume` once). `GitExec::run_within` gains no
   environment: only the history reads take one.
2. New `strop-git/src/graph.rs`: `side_dir(state_root, repo) -> PathBuf`
   (the key), `owns_graph(objects) -> bool`, `build(step, repo, side,
   cancel) -> Result<(), String>` (scratch directory, seed copy, git run
   with the variable set, rename, scratch removal), `evict(root, keep)`.
   No engine types.
3. `strop-git/src/memory.rs`: `log_graph_range` takes the optional side
   directory and a row sink; record splitting moves into a small
   incremental parser (`LogRecords::push(&[u8]) -> rows`), tested on its
   own. The `LogRow` text is unchanged.
4. `editor/git_memory/`: new `history.rs` holding what moves out of `mod.rs`
   (`open_log*`) and `jobs.rs` (`publish_log_rows`, `handle_log_completion`)
   — `jobs.rs` is at 714 lines, so the move is part of the change. New
   `GitJob::LogPage(Ticket<LogKey>, Vec<LogRow>)`; the request's entry in
   `log_requests` carries the surface revision expected for the next page
   (each page's `replace_system` moves it). The final `GitJob::Log`
   completion carries the last rows and the git run's duration.
5. New `editor/git_memory/graph.rs`: the build state (in flight: repository,
   `HEAD`, step, worker; per-repository memo of the last built `HEAD` and of
   a failed step), `consider(editor)` called from the after-event hook, the
   completion handler. State lives beside the other git state; no new
   `Editor` field beyond one struct, to respect the 0087 A4 ratchet.
   `GitJob::Graph(Completion<GraphKey, ()>)`. Builds are skipped when
   `state_dir()` is `None` (headless without a state root, replay).
6. Replay: `LogPage` deliveries go through the tape like every `GitJob`; a
   build has no result the editor state depends on, so under replay it is
   withheld (`tape_launch`) and its absence changes nothing.
7. `crates/strop/src/bench/history.rs`, wired in `bench.rs` and
   `release-local.sh qualify` beside `symbols`.
8. Tests of §5.1. Docs: the git page gains "strop keeps a commit-graph for
   large repositories in its state directory; your repository is not
   modified", and where the directory is. Amend 0099 §4.2 and §8.

Changelog, `## Unreleased`, in the existing style, one line per part:
the log and file history on a huge repository; a huge symbol list no longer
restarts the language server; closing a collection closes the files it
opened that you never looked at.

## 8. Deferred

- Decoding LSP replies without a `Value` tree (§4.3): 747 MB → 22 MB for
  100,000 symbols. It means owning the transport.
- A commit-graph for SSH repositories.
- Closing the files a replace review loaded (§6.4).
- The row cache (§2.3), if a measured flow ever asks for it.

## 9. Open questions for the maintainer

1. **Is a private commit-graph a "cache" in the sense of 0099's one-core
   rule?** §2.1 argues it is not (strop serves and validates nothing from
   it), and so adds nothing to the kernel. If the answer is that anything
   kept must have a proved owner, the only decision available to move is the
   build trigger, and proving it would not make a row more correct.
2. **Build on the first slow press, or when a large repository is opened?**
   Recommended: on the first slow press (no guess at "large", nothing spent
   for users who never open the log). The price is one slow press per
   repository per machine.
3. **Should a file the user edited only through the collection, and saved,
   close with it?** Recommended and specified: no — it stays, for its undo
   history. Saying yes also settles the replace review's 36 buffers.
4. **512 MiB.** It is derived (§4.2) but it is a transient half gigabyte in
   the worst case a hostile server can force, against about 90 MB today.
   256 MiB would be 16,000 symbols and would mean lowering `SYMBOL_LIMIT`
   to match.
5. **Streaming changes how the log fills** (rows arrive in pages; no text
   changes). It touches `publish_log_rows`, which plan 0100 may be
   restyling; the merge has to reconcile that one function.
6. Not measured: a real `rust-analyzer` on a 33,000-line file (the token
   counts here are from a synthetic reply in its shape, and they reproduce
   0099's 6,500-symbol threshold exactly); a repository on the scale of
   Linux (1.3 M commits), where the second build step may take minutes and
   wants its 120 s deadline checked; git older than 2.27.

## 10. How the numbers were taken

Scripts are not committed (they are throwaway). To reproduce:

- repositories: a fast-import stream of `commit refs/heads/main` records,
  one file changed per commit (`src/dNN/fNNNN.txt`, 2,000 files, round
  robin), committer dates 600 s apart; for bigm a one-commit `side` branch
  merged every 50th commit;
- §1.1–1.2: `git -C <repo> --no-pager log '--format=%h %an · %ar · %s%x00%H'
  -n 200 [--graph] [-- src/d00/f0007.txt]`, time to first line and to exit,
  median of 7; the private graph with
  `GIT_ALTERNATE_OBJECT_DIRECTORIES=<dir>/objects git -C <repo> commit-graph
  write --reachable [--changed-paths] --object-dir=<dir>/objects`;
- §1.3: `strop headless steps <file> --log=log.jsonl` with `keys
  <space>gl`, `settle`, `keys q` six times; per press, the `elapsed_us` of
  the last `render` event whose `cell_hash` changed minus that of the
  chord's last `input` event;
- §4.1: a binary with a counting global allocator that builds
  `{"jsonrpc":"2.0","id":7,"result":…}`, decodes it into a copy of
  `async-lsp`'s flattened untagged message with `serde_json`
  (`preserve_order`), then `from_value` into `DocumentSymbolResponse`, and
  reports the peak live bytes.
