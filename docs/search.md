# Search and the query language

One bounded qualifier grammar is shared by every picker surface: file find
(`Space f`), content search (`Space /`), replace (`Space R`), document
symbols, workspace symbols (`Space S`) and the jump list menu (`ctrl-o`
history, `Space j`). Qualifiers, highlighting, suggestions and errors behave
identically everywhere — what you learn in one field works in all of them.

## Finding files (`Space f`)

- **The top row is the best match, and Enter opens it.** The bar stays on
  the top row while files are still streaming in; once you move it, it
  follows the file you moved to.
- **A file name beats a directory name.** `sched` lists files named
  `sched…` before files that merely live under `sched/`; typing a whole
  name (`sched.h`) lists the files with exactly that name, shortest path
  first.
- **Spaces separate terms; every term must match.** `core.c sched` finds
  `kernel/sched/core.c`. A term containing `/` is matched against the path.
- **It remembers what you use.** With nothing typed, the list starts with
  your other open files (most recent first), then the current file, then
  the files you open most in this project. So `Space f` `Enter` goes back
  to the file you were just in. When you type, a file you use often wins
  among equally good matches.
- **`Space b`** lists buffers most recent first with the bar on the
  previous buffer, so `Space b` `Enter` switches back.

## Keys in the search field

| Key | Where | Does |
|---|---|---|
| `Ctrl-U` | every picker | clear the field back to its start |
| `Alt-C` | Search, Replace | match case |
| `Alt-W` | Search, Replace | whole word |
| `Alt-R` | Search, Replace | regular expression |
| `Alt-H` | Files, Search | hidden files (dotfiles) on/off |
| `Alt-I` | Files, Search | files your ignore rules skip on/off |

The toggles stay as you set them until you quit. Whenever one differs from
the default, a row under the field says so in words — `match case · whole
word · ignored files shown` — and the row disappears when everything is back
to default. A qualifier typed in the query (`case:`, `hidden:`, `ignored:`,
`regex:`) wins over a toggle.

## The grammar

**Case is smart everywhere:** an all-lowercase query matches either case;
one capital letter makes case count. Bare file queries are fuzzy; content
queries are literal unless you press `Alt-R` or write `regex:`. Use
`case:sensitive` or `case:ignore` to decide outright:

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
  and content matching. The default is `smart`: each content atom is
  sensitive only when it contains a capital. `insensitive` aliases `ignore`.
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

Buffer `/` and `?` search is smart-case, like the pickers: `/edge` finds
`Edge` and `edge`, `/Edge` finds only `Edge`. Start the query with
`case:sensitive` or `case:ignore` (alias `case:insensitive`) to decide
outright; Vim's `\C` and `\c` do the same, including in `n`/`N` and
highlights. `*` and `#` match the word under the cursor in either case. The dialect is bounded
Vim-magic (and `\v` very magic): collections, groups, alternation,
repetition and backreferences; unsupported constructs report an error
instead of becoming literals. Search previews and Enter use the same
counted resolver.

## Ex substitution

`:[range]s/pattern/replacement/[flags]` uses the same bounded Vim-magic
pattern grammar and case rule as buffer search. While composing, Strop shows the actual removed and
inserted lines, substitution count, scope and offscreen extent; a long line
is cut to a window on its change, the same window for the removed and the
added row. Enter applies that displayed plan as one undo unit. Invalid or incomplete input replaces the
previous preview rather than executing it, large plans resolve off the input
path, and Esc cancellation leaves bytes, registers and undo history untouched.

The replacement is Vim's: `&` (or `\0`) is the match and `\1`–`\9` its
groups, `~` the previous replacement, `\r` a line break, `\t` a tab, and
`\u` `\l` `\U` `\L` … `\E` change the case of what follows; `\&`, `\~`,
`\\` and an escaped delimiter are themselves. The preview shows the expanded
text. Search's With field has its own capture syntax (`$1`; see With below).

Flags: `g` every match on a line, `i` / `I` ignore / match case, `n` count
the matches without changing anything, `e` no complaint when nothing
matches, `&` the previous substitute's flags, and `c` to be asked at each
match — `y` replaces it, `n` skips it, `a` replaces all the rest, `l` this
one and stop, `q` or Esc stop. A count after the flags (`:s/a/b/g 3`) covers
that many lines. `:s` or `:&` alone repeats the last substitute on the
cursor line, `:&&` with its flags, `:~` with the last search pattern.

## The Search workspace

`Space /` opens or resumes one large Search workspace; `Space R` resumes
the same investigation with the replacement field visible. Local
enumeration and content matching run entirely in Rust using the
`ignore`/`grep` crate family — `rg` is not required on your machine.

- **Ctrl-T** toggles the With field without restarting the search or losing
  either field's text, caret, mode, selection or workset. Enter in **Find**
  opens the selected source; Enter in **With** prepares the review. Tab
  switches fields when With is visible.
- **Ctrl-R {register}** inserts a register into Find or With, exactly as in
  insert mode and on the `:` / `/` lines: `Ctrl-R "` is the last yank or
  delete, `Ctrl-R 0` the last yank, `Ctrl-R a` a named register, `Ctrl-R +`
  the system clipboard.
- **Ctrl-X** toggles one match; **Ctrl-D** masks/restores its file.
  Excluded hits stay visible.
- **Ctrl-O** collects exactly the included matches into a collection (below).
- Filter-only file finding lists eligible files; content search needs an
  expression. Unignored dotfiles are included by default; `.git` metadata
  is excluded. `Alt-H` and `Alt-I` in the field turn hidden and ignored
  files on and off.

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
