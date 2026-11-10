# Search and the query language

One bounded qualifier grammar is shared by every picker surface: file find
(`Space f`), content search (`Space /`), replace (`Space R`), document
symbols, workspace symbols (`Space S`) and the jump list menu (`ctrl-o`
history, `Space j`). Qualifiers, highlighting, suggestions and errors behave
identically everywhere — what you learn in one field works in all of them.

## The grammar

Bare file queries are fuzzy and case-insensitive; content queries are literal and
case-insensitive unless you choose `regex:`. Use `case:sensitive` on any query
surface for exact-case matching, or `case:smart` for the former uppercase-sensitive
behavior:

```text
language:rust
language:cpp path:src/ request_id
glob:"src/my files/**/*.rs" -path:generated/ text:"request-id"
language:rust regex:"\brequest_(id|name)\b"
hidden:exclude ignored:include text:"-test.rs"
kind:function parse AND NOT wrap
```

- `language:NAME` — file language (the names `Space ?` lists under
  Languages, e.g. `rust`, `cpp`, `python`).
- `path:SUBSTRING` — substring of the workspace-relative path (no leading `/`);
  case follows the query's case mode.
- `fpath:SUBSEQUENCE` — fuzzy scope over the full workspace-relative path;
  use `fpath:modules/jweb/ parser` to scope the directory while ranking
  `parser`. `-fpath:` excludes matches.
- `glob:PATTERN` — glob over the workspace-relative path.
- `hidden:` / `ignored:` — visibility overrides for this query.
- `case:sensitive|ignore|insensitive|smart` — case mode for file, path, symbol
  and content matching. The default is insensitive; `insensitive` aliases
  `ignore`, and `smart` classifies each content atom as sensitive only when that
  atom contains uppercase.
- `kind:` — on symbol surfaces, the declaration's classification
  (`kind:function`, `kind:struct`, …).
- `AND` / `OR` / `NOT` and parentheses — Boolean expressions; juxtaposition
  is an implicit AND.

`Ctrl-Space` in any query field offers parser-driven suggestions from the
caret's position; `:help query` explains quoting, supported languages,
errors and examples. Old `-t`/`-g` UI flags are ordinary text now.

## What each qualifier narrows

- **File find** narrows the walk; **search/replace** narrows admission per
  line.
- **Document symbols and the jump list** are retained lists: the same
  qualifiers re-filter their rows locally as you type. The jump list contains
  origins left by jump commands, with a synthetic row for the current position;
  it is navigation history, not an undo list. On the jump list,
  `language:cpp path:tests` keeps only jumps inside C++ files under
  `tests/`; content atoms match the destination line's text. Evidence a
  row cannot decide (a scratch buffer with no path) admits — overfetch,
  never a silent drop.
- Operator-free content never enters admission; the local fuzzy ranker
  narrows it.

## Buffer search

Buffer `/` and `?` search is case-insensitive by default. Start the query
with `case:sensitive` for exact case; `case:ignore`/`case:insensitive`
select insensitive matching explicitly. Vim's `\C` and `\c` remain
supported, including in `n`/`N` and highlights. The dialect is bounded
Vim-magic (and `\v` very magic): collections, groups, alternation,
repetition and backreferences; unsupported constructs report an error
instead of becoming literals. Search previews and Enter use the same
counted resolver.

## Ex substitution

`:[range]s/pattern/replacement/g` uses the same bounded Vim-magic pattern
grammar as buffer search, and matches case exactly unless the pattern says
otherwise (`\c`, or a leading `case:insensitive`). While composing, Strop
shows the actual removed and inserted lines, substitution count, scope and
offscreen extent; a long line is cut to a window on its change, the same
window for the removed and the added row. Enter applies that displayed plan
as one undo unit. Invalid or incomplete input replaces the previous preview
rather than executing it, large plans resolve off the input path, and Esc
cancellation leaves bytes, registers and undo history untouched.

Replacement text is literal: `&`, `$1` and `\1` have no capture meaning. An
escaped delimiter inserts that delimiter and `\\` inserts one backslash; other
backslashes are preserved. (Project replace does expand captures; see With
below.)

## The Search workspace

`Space /` opens or resumes one large Search workspace; `Space R` resumes
the same investigation with the replacement field visible. Local
enumeration and content matching run entirely in Rust using the
`ignore`/`grep` crate family — `rg` is not required on your machine.

- **Ctrl-R** toggles the With field without restarting the search or losing
  either field's text, caret, mode, selection or workset. Enter in **Find**
  opens the selected source; Enter in **With** prepares the review. Tab
  switches fields when With is visible.
- **Ctrl-X** toggles one match; **Ctrl-D** masks/restores its file.
  Excluded hits stay visible.
- **Ctrl-O** collects exactly the included matches into a collection (below).
- Filter-only file finding lists eligible files; content search needs an
  expression. Unignored dotfiles are included by default; `.git` metadata
  is excluded. `:search-options` shows the separate visibility controls.

Opening a source hides Search; `Space /` returns to the retained
investigation. Re-entry refreshes through owned work in its captured
namespace/root even if focus or cwd changed. In a Directory buffer,
`:fs search` or **Space a → Search here** captures that directory without
changing cwd.

## Collections: edit every hit in one buffer

**Ctrl-O** in search results, references (`grr`) or diagnostics (`Space d`)
opens a collection: one card per file, each hit with two lines of context,
source syntax and source line numbers. It is an ordinary editing buffer
whose text is the files' text:

- Edit inside an excerpt with any command — `cw`, `dd`, `o`, `.`, a macro,
  visual mode, `:s`. The edit is made in the source buffer in the same
  keystroke; open the file and it is already there. Edits made in the file
  appear in the collection the same way.
- `:w` writes every changed file the collection shows. `:wq` does that and
  closes it.
- `u` and `Ctrl-R` undo and redo what you changed through the collection,
  in the files too. Editing one of its files somewhere else starts the
  collection's history over; that file's own `u` still works.
- Card borders, the `⋮ N source lines omitted` rows and the title are not
  text. An edit that would touch one, or join the last line of an excerpt
  with the border under it, is refused and changes nothing.
- `+` / `-` widen and narrow the excerpt under the caret; `]e` / `[e` step
  between excerpts and `]f` / `[f` between files; `g<Space>` (or Enter on a
  card's top border) opens the file at that line, and `Ctrl-O` comes back.
- `:%s/old/new/g` in a collection substitutes inside the excerpts only,
  with the usual live preview.

### The quickfix list

The newest collection is the quickfix list, under the names Vim gave it:

| Command | Does |
| --- | --- |
| `:copen` | show the list (the collection) |
| `]q` `[q`, `:cnext` `:cprev` (`:cn` `:cp`) | go to the next or previous hit in its file |
| `:cc N` | go to hit N |
| `:cdo {cmd}` | run an ex command with the cursor on every hit line |
| `:cfdo {cmd}` | run an ex command once in every file of the list |

```vim
:cdo s/open_picker/open_the_picker/
:cfdo w
```

`:cdo` and `:cfdo` return to where you were. Each command finishes before
the next starts, and the files stay open and undoable. `:cdo norm …` waits
for `:norm`.

## Replace across files

Replace is two steps.

1. `Space R`, type the term, `Tab`, type the replacement. Every hit shows a
   `-`/`+` pair that follows your typing; `Ctrl-X` and `Ctrl-D` exclude a hit
   or a file. `Enter` in **With** opens the review: the exact diff, nothing
   changed yet.
2. `Enter` in the review changes every file and writes it. The heading says
   how many files were changed and written.

In the review:

- `u` puts the buffers and the files back. A file you edited since the apply
  is left alone and named. After a full revert `Enter` applies again.
- `q` closes the review. One that was never applied is cancelled, and Search
  comes back with both fields as you left them. `Esc` cancels a review that
  is still being prepared.
- `Space a` lists the same three: apply, revert, cancel.

Unsaved text in an open buffer wins over the file on disk. A file that
changed between the search and the review, or between the review and
`Enter`, is refused by name and left untouched; so is a read-only one. A
file that could not be written stays changed in its buffer, unsaved, and is
named in the heading. An all-excluded, incomplete or failed search cannot
become a whole-project operation.

**With** is literal unless the term is a `regex:` with groups. Then `$1`,
`${name}` and `\1` are that hit's own groups, `$0` the whole hit, `$$` a
dollar sign:

```text
Find  regex:"get_(\w+)\(\)"
With  fetch_$1()
```

A query with several positive terms is a fine search and an ambiguous
replace; With asks for exactly one.

## Rename

`grn` opens `:rename` with the name under the cursor, ready to edit. A rename
that touches one buffer is applied at once (and `u` there undoes it). One
that touches several opens the same review as a project replace: `Enter`
applies and writes, `u` reverts.

## SSH scopes

Search here uses the same query fields, inclusion decisions and source
previews as local Search, on explicitly scoped SSH directories: `rg` runs
on the host over its connection, and a host without `rg` refuses search
without a fallback. Collections work there too. **Replace (With and its review) is unavailable in SSH scopes**.

Source enumeration is limited to 100,000 selected paths or 16 MiB of names;
rg records to 1 MiB and 4,096 submatches; decoded batches to 4 MiB; a
request publishes at most 100,000 rows or 64 MiB of owned row data. Limits
and malformed output produce a visible failure, never a silently complete
partial result.
