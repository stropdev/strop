# 0097 — Collect: search, Ctrl-O, and you are on your hit

- Status: **in force**; implemented with this plan
- Date: 2026-11-12
- Builds on 0094 §3.1 (the collection is the editable list of hits). Changes
  how you get into it and how you move in it; the write-through is untouched.

## 1. Why

The maintainer's note: *"collect flow ctr+o this is where you do grep then
collect, this flow needs to be proper, it is not great atm"*.

0094 made the collection editable and gave it Vim's quickfix names. The parts
work. The flow between them was never walked end to end. Walked on the real
binary (`strop --headless`, a 2-file and a 300-file project), this is what
happens:

| # | What you do | What happened | Kind |
|---|---|---|---|
| C1 | Type a query, press `Ctrl-O` before the search has finished | `results are still updating — retry Ctrl-O when ready`. Enter waits for the search; `Ctrl-O` made you wait and press again. | friction |
| C2 | Move the bar to the third hit, `Ctrl-O` | The cursor is on the title row, line 1. The hit you were looking at is somewhere below. | lost place |
| C3 | Look at the title | `collection: search — 4 match(es) · 2 file(s)`. It does not say what was searched. | display |
| C4 | Search again, `Ctrl-O` again | A second buffer, also named `collection: search`. `Space b` lists both, indistinguishable. Every collect adds one; none is ever replaced. | pile-up |
| C5 | Want the next hit, in the collection | There is no key. `]e` steps *excerpts* (two hits that share context are one excerpt, so one is skipped) and lands on the first context line, not on the hit. | missing step |
| C6 | Press `]q` in the collection, cursor on the third hit | You leave the collection for the file of hit **1**. `]q` counts from the last hit `]q` visited, not from where you are. | surprise |
| C7 | Press `Enter` on a hit | The cursor moves down a line. Opening the file there is `g<Space>`, which nothing on screen mentions. `Enter` does open the file — only on a card's top border. | missing step |
| C8 | `]q` to hit 3 in its file, then `:copen` | The collection opens wherever its cursor was left, not on hit 3. | lost place |
| C9 | Read the messages | `collection: 3 excerpt(s)` under a title saying `4 match(es)`; then `quickfix: 4 hit(s)`. Three nouns (match, hit, excerpt) for one thing, two names (collection, quickfix) for one buffer, and `(s)` everywhere. | wording |
| C10 | Read a card's top border | `╭─ src/a.rs ── rust · 2 excerpt(s) · context 2 (+/-)`. The language is visible in the colours; `context 2 (+/-)` is a key hint written as a fact. The number you want — hits in this file — is absent. | display |
| C11 | `Ctrl-O` with every hit excluded | `collection: no included source matches`. | wording |

None of these is a crash. Together they are why the flow feels unfinished:
you collect, and then you have to find your place, guess the keys and tell
the buffers apart.

## 2. What the others do

- **Vim quickfix** (`:help quickfix`, `:help quickfix-window`). The list has a
  *current entry*. `:cnext` moves it; in the quickfix window `<CR>` jumps to
  the entry under the cursor and makes it current; the window shows which
  entry is current. `w:quickfix_title` holds the command that made the list.
  A new list does not overwrite the old one (`:colder`, ten deep).
- **Telescope** `send_to_qflist` + `open_qflist`: collects whatever the
  picker has when the key is pressed. It does not wait and does not refuse.
- **Helix**: the picker is the only list; there is nothing to collect into.
  `Space '` reopens the last picker. strop's `Space /` already does that.
- **Zed / VS Code search**: results are an editable multibuffer (Zed) or a
  tree (VS Code). Zed titles the tab with the query, a repeated search
  reuses the tab, and `F4`-style next/previous-match keys move inside it.
  Opening the file at an excerpt is one key.

What they share: one current hit, a next/previous pair that works from where
you are, one key to open the file, and a title that says what the list is.

## 3. Design

One idea: **a collection has a current hit, and your cursor in the collection
is it.** Everything below follows from that.

### 3.1 Getting in

- `Ctrl-O` while the search is still running **waits and then collects**, the
  way Enter waits (0093 §3.1). The message says so. Any other key takes the
  request back. (C1)
- The cursor lands **on the hit under the picker's bar**, at the matched
  text, and that hit is the current hit. (C2)
- The collection is named for what made it: `collection: search needle_`
  for a search (the query, cut at 48 characters), and the list's own title
  otherwise (`collection: locations`, `collection: diagnostics`). The title
  row reads `search needle_ — 4 hits in 2 files`. (C3)
- Collecting the same thing again **refreshes that collection** instead of
  adding a buffer: same name, same buffer, new hits, history started over.
  A different search is a different collection, and both stay. (C4)

### 3.2 Moving

| Key | In the collection | In any other buffer |
|---|---|---|
| `]q` / `[q` (`:cnext` / `:cprev`) | cursor to the next / previous hit, counted **from the cursor** | the file of the hit after / before the current one |
| `Enter` | open the file at this line; the hit on it (or the nearest one above it in the excerpt) becomes current | (unchanged) |
| `Ctrl-O` | — | back, as always: after `Enter` it returns to the collection |
| `:copen` | — | show the collection with the cursor **on the current hit** |
| `]f` / `[f`, `+` / `-` | unchanged | — |

(C5–C8.) `]q` in the collection stays in the collection because the
collection is where the hits are edited; leaving it on every step would make
it a table of contents. This differs from Vim, where `:cnext` typed in the
quickfix window jumps to the file — but Vim's window is a read-only list
beside the file, and this one is the text itself. `Enter` is Vim's own key
for "go to this entry".

The list these keys act on is the collection you are in, else the one you
were last in or made last.

**What a hit is.** One row of the list that was collected: for a search, one
match (two matches on a line are two hits, as they are two rows in Search),
with the cursor landing on the matched text; for references and diagnostics,
one line. A match that has since been edited away leaves its line as the
hit, so stepping never loses a place you changed. `:cdo` keeps running once
per hit *line* (0094): `:cdo s/a/b/` on a line with two matches must not run
twice.

### 3.3 Deleted

- `g<Space>` and `:collection source`: `Enter` does it, on every row that
  shows a file's text. One key, one meaning (AGENTS.md: no aliases). 0091 §6
  already listed `g<space>` among the keys that should not be in the Moving
  table.
- `]e` / `[e`: `]q` / `[q` step hits, which is what was wanted. An excerpt is
  how hits are drawn, not a thing to navigate.
- `:collection expand|contract`: `+` / `-`. With `source` gone the command
  has no reason to exist.
- The keymap's `<space>`-in-the-middle token (`lookup.rs`), which only
  `g<Space>` used.
- `Collection::match_count`: the title and the cards count the hits the keys
  step, so the two cannot disagree.

### 3.4 Words

One noun for a place the search found: **hit**. One name for the buffer:
**collection**. "quickfix" stays only where a Vim user would look for it
(`:help`, the docs heading, the `?` row for `:copen`). Counts are spelled
(`1 hit in 1 file`, `4 hits in 2 files`), never `hit(s)`.

- After collecting: `4 hits in 2 files — ]q next, Enter opens the file, :w writes`.
- `Ctrl-O` ahead of the search: `collecting when the list is complete…`.
- Stepping: `hit 2 of 4: src/a.rs:11`; at the ends `last hit` / `first hit`.
- Nothing collected yet: `nothing collected yet — Ctrl-O in search results, references or diagnostics collects them`.
- Nothing to collect: `nothing to collect — every hit is excluded` / `nothing to collect — the list is empty`.
- A card's top border: `╭─ src/a.rs ── 3 hits` (then ` · modified` /
  ` · read-only` as before). (C10)
- A gap: `⋮ 4 lines not shown`.

### 3.5 What does not change

Edits, `:w`, `u`, `:cdo`, `:cfdo`, `:cc N`, the card look, the kernel's
`excerpt::source_edit` and its proofs. No kernel code moves.

## 4. Rejected

- **A second key for "send to quickfix"** (Telescope's `Ctrl-Q`). 0094
  rejected it and nothing here changes that.
- **A split: list below, file above** (Vim's `:copen` layout). The collection
  already shows each hit with its context and is editable; a second window
  showing the same lines would be a preview of what is already on screen.
- **Collecting partial results** when `Ctrl-O` is pressed mid-search
  (Telescope). `:cdo s/…/…/` over a list that silently missed files is the
  one outcome a project-wide change must not have (docs/search.md: "It never
  shows part of the results as if they were all of them").
- **One collection, replaced by every collect.** Simple, but references and a
  search are often wanted side by side, and a collect would silently drop the
  other's undo history. Refresh-on-same-name gets the tidiness without that.
- **A stack with `:colder` / `:cnewer`.** `Space b` and the names do the same
  job with nothing new to learn.
- **`dd` on a hit removes it from the list** (quickfix-reflector). In a
  collection `dd` deletes the line in the file; `Ctrl-X` / `Ctrl-D` in the
  picker are where hits are left out.
- **`]q` in the collection jumping to the file** (Vim-exact). See §3.2.

## 5. Tests

Engine (`collections/tests/flow.rs`, on the real in-process search over a
tempdir; `:cdo` and friends stay in `tests/quickfix.rs`; the early `Ctrl-O`
in `picker/search_tests.rs`):

- `Ctrl-O` during a running search collects when it finishes; another key
  first takes the request back.
- The cursor lands on the picked hit's matched text; that hit is current
  (`]q` from a file goes to the one after it).
- `]q` / `[q` in the collection step every hit from the cursor, including two
  hits in one excerpt; the ends say `last hit` / `first hit` and do not move.
- `Enter` on a hit line, on a context line and on a card top opens the file
  at the right place and sets the current hit; `Ctrl-O` returns.
- `:copen` puts the cursor on the current hit.
- Collecting the same search twice leaves one collection; a different search
  leaves two with different names.
- Title, card and message wording, singular and plural.
- `g<Space>`, `]e`, `:collection` are not commands.

Goldens: `collected-*` is the collection straight after `Ctrl-O` from a
search with the bar on the second hit — title, cards with hit counts, the
message, cursor on the picked hit — in both themes and both sizes.
`collection-*` (0094, after an edit) and `help-*` are regenerated for the
new words.

Journey (`crates/strop/tests/change.rs`, real binary): search → `Ctrl-O`
before `settle` → the collection is there with the cursor on the first hit;
`]q` twice stays in it; `Enter` lands in the right file, line and column.

`?` lists `]q` / `[q`, `Enter`, `]f` / `[f`; the keymap contract test
(`keymap/tests.rs`) and `docs/vim-compat.md` (generated) cover it.

## 6. Deferred

- ~~Closing the file buffers a collection loaded once it closes~~ — done in
  [0101](0101-history-and-limits.md) §6: loans, released by
  `release_document`, a close that moves no cursor.
- `:cdo` over several hundred hits re-reads the hit list every round. It was
  12 s in a debug build for 600 hits; measure in release before changing it.
- A collection of the picker's *symbols* lists. They are not places with a
  matched text today.
