# 0094 — change: one surface for changing many places, one undo

Status: implemented with this plan. Scope comes from 0091 §4.3, §4.4, §5.1,
§5.3, §5.4 (B2–B6) and §7 "change". The look follows 0088.

## 1. What was wrong

0091 measured it on the real binary. Replacing a word in 34 files took a
picker, a read-only proposal buffer, `:apply-change`, a receipt, `:save-change`
and a second receipt whose rows never left `saving` (B4). `:undo-change`
reverted buffers and left the disk changed with no `:wa` (B5). `Space a` on the
proposal offered twelve filesystem verbs and no Apply (B3). Collections showed
search hits with context and refused every edit, and there was no quickfix
list, so a non-uniform edit at N hits meant visiting each. `:s` drew its
preview from column 0 and clipped before the change, covered the statusline,
and panicked in an 8-row terminal (B2, B6).

## 2. What the others do (research notes)

- **Zed multibuffers** (zed.dev/docs/multibuffers). Excerpts from many files in
  one buffer; edits write through to the files; saving the multibuffer saves
  every touched file; excerpt headers and gaps are not text. Project search,
  find-references and diagnostics all open one. Undo in a multibuffer undoes
  its own transactions. This is the model for the collection.
- **Neovim quickfix** (`:help quickfix`, `:help :cdo`, `:help :cfdo`,
  `:help ]q`). A list of locations fed by `:grep`/`:make`/LSP; `:copen` shows
  it, `]q`/`[q` (default since 0.11, unimpaired before) step through it,
  `:cdo {cmd}` runs an ex command at every entry, `:cfdo {cmd}` once per file.
  `:cdo s/a/b/ | update` is the muscle memory for a project-wide change.
- **grug-far.nvim** (README) and **VS Code search** (code.visualstudio.com
  docs, "Search and replace"). Live `-`/`+` rows while typing the replacement,
  per-hit and per-file exclusion, one action replaces everywhere and writes,
  capture groups (`$1`) in the replacement when the search is a regex. VS Code
  undoes a replace-all across files with one undo.
- **Neovim 0.11 LSP defaults** (`:help grn`, `:help lsp-defaults`): `grn`
  renames, prompting with the name under the cursor.
- **inccommand** (`:help 'inccommand'`): the `:s` preview shows the changed
  text in place; nvim scrolls the preview window, it never truncates before the
  match.

Not copied: Telescope's `<C-q>` as a second "send to quickfix" key. `Ctrl-O`
in a results list already collects; a second key for the same list would be an
alias (AGENTS.md).

## 3. Design

### 3.1 The collection is the editable multibuffer and the quickfix list

One concept, three vim names for it. `Ctrl-O` in search results, references or
diagnostics collects the included hits. The collection buffer is writable:

- An edit inside an excerpt body is applied to the view and written through
  to the source buffer as one batch per source, in the same key dispatch. The
  source's own history records it; the source becomes dirty; every other view
  of that source is updated by the existing projection.
- Card borders, gap rows and the title are not text. An edit that would touch
  them, or join the last body line with the border, is refused before any byte
  changes, with one message.
- `:w` in a collection writes every dirty source it shows. `:wq` does that and
  closes the view.
- `u` and `Ctrl-R` walk the view's own history, and every source that step
  changed takes the same step in its own history. Translating the view's
  inverse edit a second time was tried first and was wrong at an
  unterminated last line (the convergence test found it): the source's own
  history holds the exact inverse, so it is the one that moves. This is
  sound because a view's step and its sources' steps are recorded together
  (one undo group in each, closed together), and because any edit of a
  shown source that the view did not make starts the view's history over.
  The source's own `u` still works.
- A refused edit that was the first thing its command tried voids the rest
  of that command: `J` on an excerpt's last line must not keep the space it
  adds after the join it could not make.
- The newest collection is the quickfix list. `:copen` focuses it. `]q`/`[q`,
  `:cnext`/`:cprev` (`:cn`/`:cp`) and `:cc N` jump to a hit in its source.
  `:cdo {cmd}` runs an ex command with the cursor on each hit line;
  `:cfdo {cmd}` once per file. Sources are already loaded (collecting loads
  them as a background job), so `:cdo` crosses no `await`.
- `:cdo s/…/…/` plans inline whatever the buffer size: it is a batch the user
  asked for, bounded by the list, and a deferred plan would race the next
  entry.
- `:cdo` runs whatever the `:` line's command table runs (0095 §2.6), so
  `:cdo norm A;`, `:cdo d` and `:cdo g/…/…` work. Each entry's command is a
  compound command in 0095's sense (`ex::run_ex_in_batch`): it resolves
  inline and is one undo step in its source.

Decision moved into the kernel: `strop_kernel::excerpt::source_edit` decides
whether a view edit is admitted and where it lands in the source. Theorems
(`proofs/Kernel/Excerpt.lean`): the admission is exact; an admitted edit lands
inside the excerpt's source span at the same offset; an anchor followed
through the view edit and translated equals the translated anchor followed
through the source edit (`map_position` on both sides, so
`map_through_edits` composes it over a journal). Two mutants.

What stays in ring 1 (property-tested, not proved): finding the excerpt for a
byte, composing a batch's journal into disjoint replacements
(`collections/journal.rs`, already tested), and the view's added terminator
when a source's last line has no newline: lines added below such a line put
their newline in front, so the file stays unterminated as it was. An excerpt
whose lines were all deleted shows no rows (it used to show one empty row,
which no source byte backed).

### 3.2 Replace is two steps

`Space R`, find, `Tab`, with, `Enter` prepares the review (unchanged: a
cancellable job, exact witnesses, dirty buffers win over disk). In the review:

- `Enter` applies every target and writes it. The heading says what happened.
- `u` reverts each buffer whose history still sits where the apply left it and
  whose file was written, writes it again, and names the ones it could not
  revert. After a full revert `Enter` applies again.
- `q` closes; a pending review is cancelled by closing it, and Search resumes.
- Buffers the review loaded itself are closed with it once they are clean, so
  a 34-file replace does not leave 34 buffers behind.

`:apply-change`, `:save-change`, `:undo-change`, `:cancel-change`, the receipt
text and the save receipt are deleted. Filesystem reviews keep their own
operations log (`:fs operations`); they are applied with `Enter` in the review
or `:fs apply`, cancelled with `q` or `:fs cancel`.

Captures: when the replaced term is `regex:`, `$1`/`${name}`/`\1` in With
expand per hit. The picker rows and the review use one function
(`strop_picker::Replacer`), so the preview cannot disagree with the result.
A literal term keeps With literal.

### 3.3 Rename

`grn` opens the `:rename ` prompt filled with the name under the cursor. A
rename touching one buffer applies directly and flashes; one touching several
opens the same review, with the same `Enter`/`u`. `grn` needs `gr` to be a
prefix, so references move from `gr` to `grr`: Neovim 0.11's own pair
(`:help grr`, `:help grn`). The rest of that family maps onto actions that
exist: `gra` is `Space a`, `gri` is implementation (`gI` stays; it is on
0095's Reassigned list). Plain `gr{char}` is not bound and the Reassigned
list says so.

### 3.4 `Space a`

On a file: retained recoveries, terminal controls and code actions. No
filesystem rows. In a Directory buffer and under `:fs`: the filesystem rows.
The `operations`, `verify` and `undo` rows live under `:fs` only.

### 3.5 `:s` preview

Each removed/added pair is windowed on its first differing column, with `…`
where text is cut. The card never grows past the row above the statusline and
is skipped when fewer than three rows remain. `:s` has `/`'s case policy
(0095 §2.6), which 0093 §3.7 makes smart case: an all-lowercase pattern
matches either case, `\c` / `\C` and the `i` / `I` flags decide outright.
(This track first forced `:s` case-sensitive by appending `\C`; that was
dropped when the three tracks met, so that one compiler has one policy.)

## 4. Tests

- Kernel unit tests; Lean claims `source_edit_spec`,
  `source_edit_commutes`; mutants `excerpt-end-unbounded`,
  `excerpt-terminator-ignored`.
- Property test (`collections/tests/convergence.rs`): random edit streams
  applied through a collection and directly to twin sources converge, and the
  view always equals its re-rendering.
- Engine tests: admission refusals, write-through, `:w`, `u`, `:cdo`/`:cfdo`,
  `]q`/`[q`, replace apply/revert, captures, rename review, `Space a` rows.
- Journeys on the real binary in a tempdir (`crates/strop/tests`): collect →
  `cw` on three excerpts in two files → `:w` → files changed, buffers clean,
  `u` reverts one; replace across 34 files → `Enter` `Enter` → disk changed,
  no dirty buffers → `u` → byte-identical, no dirty buffers.
- Goldens: editable collection, the review before and after apply, `Space a`
  on a source with a diagnostic, the `:s` card at 90×24 and 90×8.

The convergence test compares edits that mean the same thing at a caret in
the view and in the source. `dw` and `cw` are among them since 0095 fixed
their end-of-line behaviour (they used to reach into the next line, which in
a view is the border and is refused).

Found on the way and fixed here because the test could not pass without it:
`cc` planted a second cursor stacked on the first, which survived the insert
session and `u` and then edited a second place.

## 5. Deferred

- Closing the buffers a replace loaded once its review closes (0091 §4.3's
  "36 buffers open"). The close that moves no cursor now exists
  (`Editor::release_document`, [0101](0101-history-and-limits.md) §6.2) and
  collections use it for the files they loaded; the replace review does
  not yet — every file a replace loads is edited by it, and 0101 §6.4 keeps
  edited files open for their undo history.
- The undo tree (`Space u`) of a collection: restoring an arbitrary revision
  there is written through as a plain edit, not as source history moves.

- A collection's history surviving foreign edits to its sources. It needs the
  history's offsets mapped through system edits and a way to tell whose step
  a source's newest one is.
- One replacement grammar. `:s` has Vim's (`&`, `\1`, `\u`…, 0095 §2.6) over
  the buffer-search engine's captures; With has `$1` / `${name}` / `\1` over
  the `regex` crate's (`strop_picker::Replacer`). They overlap only in `\1`,
  and run on different engines, so both stay.
- Remote (SSH) replace stays unavailable; collections over SSH are editable
  like any other since they write to buffers.
