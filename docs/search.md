# Search and filters

The same filters work in every search field: Files (`Space f`), Search
(`Space /`), Replace (`Space R`), document symbols (`Space s`), workspace
symbols (`Space S`) and the jump list (`Space j`). Learn them once.

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

## The filters

**Case is smart everywhere:** an all-lowercase query matches either case;
one capital letter makes case count. In Files, plain words are matched
fuzzily. In Search, plain words are searched for exactly as typed, unless
you press `Alt-R` or write `regex:`. Use `case:sensitive` or `case:ignore`
to decide outright:

```text
language:rust
language:cpp path:src/ request_id
glob:"src/my files/**/*.rs" -path:generated/ text:"request-id"
language:rust regex:"\brequest_(id|name)\b"
hidden:exclude ignored:include text:"-test.rs"
kind:function parse AND NOT wrap
```

- `language:NAME` — the file's language (the names `Space ?` lists under
  LSP servers, e.g. `rust`, `cpp`, `python`).
- `path:TEXT` — text that appears in the file's path inside the project
  (no leading `/`).
- `fpath:TEXT` — the same, matched fuzzily; use `fpath:modules/jweb/ parser`
  to stay in a directory while looking for `parser`.
- `glob:PATTERN` — a glob over the path inside the project.
- `hidden:include|exclude` / `ignored:include|exclude` — include or exclude
  hidden files and ignored files for this query.
- `case:sensitive|ignore|smart` — how case is matched, in file names, paths,
  symbols and text. The default is `smart`: a term is case-sensitive only
  when it has a capital in it. `case:insensitive` means `case:ignore`.
- `kind:` — in the symbol pickers, what the symbol is (`kind:function`,
  `kind:struct`, …).
- `text:"..."` — text to search for exactly, even if it looks like a
  filter. `regex:"..."` — a regular expression.
- A `-` in front excludes: `-path:generated/`, `-language:json`.
- `AND` / `OR` / `NOT` and parentheses combine terms. A filter next to
  text means both. In Search, plain words next to each other are one
  phrase: `request id` finds exactly that text.

`Ctrl-Space` in any search field suggests what can come next at the
cursor. `:help query language` covers quoting, errors and examples.

## Where the filters apply

- In **Files** the filters decide which files are listed.
- In **Search** and **Replace** they decide which files are searched.
- In **document symbols** and the **jump list** they filter the rows
  already in the list, as you type, and plain words rank the rows by fuzzy
  match. On the jump list, `language:cpp path:tests` keeps the jumps inside
  C++ files under `tests/`.
- A row a filter cannot judge is kept, not hidden. A buffer with no file
  has no path, so `path:` keeps it.

The jump list holds the places you jumped from, plus a row for where you
are now. It is a history of places, not of edits.

## Buffer search

`/` and `?` in a buffer are smart-case, like the pickers: `/edge` finds
`Edge` and `edge`, `/Edge` finds only `Edge`. Start the query with
`case:sensitive` or `case:ignore` to decide outright; Vim's `\C` and `\c`
do the same, including for `n`/`N` and the highlight. `*` and `#` match the
word under the cursor in either case. Patterns use Vim's default syntax,
and `\v`: `[...]`, groups, `\|`, repetition and backreferences. Anything
not supported gives an error; it is never searched for as plain text. The
match shown while you type is the one Enter goes to, with a count as well.

## Substitute: `:s`

`:[range]s/pattern/replacement/[flags]` takes the same patterns as buffer
search, with the same case rule. While you type the command, strop shows
the lines it would remove and the lines it would add, how many replacements
there are, and how many of them are off screen. A long line is cut to the
part that changes, the same part in the removed and the added row. Enter
makes exactly the change shown, and one `u` undoes it. A pattern that is
not finished or not valid changes nothing. On a big file the preview is
worked out in the background and typing stays quick. Esc cancels and
leaves the text, the registers and the undo history as they were.

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

## Search across the project

`Space /` opens Search, or brings it back as you left it. `Space R` does
the same with the With (replacement) field showing. Search is built in: you
do not need `rg` on your own machine. It follows your `.gitignore` rules
the way `rg` does.

- **Ctrl-T** shows or hides the With field. The search keeps running, both
  fields keep their text and cursor, and the hits you excluded stay
  excluded. Enter in **Find** opens the selected result; Enter in **With**
  opens the review. Tab switches fields when With is showing.
- **Ctrl-R {register}** inserts a register into Find or With, exactly as in
  insert mode and on the `:` / `/` lines: `Ctrl-R "` is the last yank or
  delete, `Ctrl-R 0` the last yank, `Ctrl-R a` a named register, `Ctrl-R +`
  the system clipboard.
- **Ctrl-X** excludes one hit, or includes it again; **Ctrl-D** does the
  same for its whole file. Excluded hits stay visible.
- **Ctrl-O** puts the included hits into a collection (below).
- With only filters typed (`language:rust path:src/`), Files lists the
  files they match; Search asks for the text to look for. Dotfiles are
  searched unless your ignore rules skip them. The `.git` directory never
  is. `Alt-H` and `Alt-I` in the field turn hidden and ignored files on
  and off.

Opening a result hides Search. `Space /` brings it back with your query
and results. It searches again in the directory (and on the host) it was
first opened for, even if you are now in a file somewhere else. In a
directory listing, `:fs search` or **Space a → Search here** searches only
that directory. The project root stays where it is.

A search stops with an error when it passes 100,000 files, or meets a line
longer than 1 MiB or with more than 4,096 hits in it. Past 100,000 result
rows it keeps what it has and says so. It never shows part of the results
as if they were all of them.

## Collections: edit every hit in one buffer

**Ctrl-O** in search results, references (`grr`) or diagnostics (`Space d`)
opens a collection: one card per file, each hit with two lines of context,
highlighted like the file and with the file's line numbers. The cursor is
on the hit that was selected in the list. If the search is still running,
Ctrl-O waits for it and then collects; any other key takes that back.

The first row says what was collected, for example
`search open_picker — 12 hits in 4 files`, and the buffer is named
`collection: search open_picker`. Collecting the same search again brings
that collection up to date; a different search gets its own.

### Moving

| Key | In the collection | In any other buffer |
| --- | --- | --- |
| `]q` `[q` | the next or previous hit, from the cursor | the next or previous hit, in its file |
| `Enter` | open the file at the cursor | |
| `Ctrl-O` | | go back; after `Enter`, back to the collection |
| `:copen` | | back to the collection, on the hit you were last at |
| `]f` `[f` | the next or previous file | |
| `+` `-` | show more or less context around the hits under the cursor | |

A count works: `3]q` is three hits on.

### Editing

A collection is an ordinary buffer. What you edit in it is edited in the
files:

- Edit inside a card with any command — `cw`, `dd`, `o`, `.`, a macro,
  visual mode, `:s`. The edit is made in the file's buffer in the same
  keystroke; open the file and it is already there. Edits made in the file
  appear in the collection the same way.
- `:w` writes every changed file the collection shows. `:wq` does that and
  closes it.
- `u` and `Ctrl-R` undo and redo what you changed through the collection,
  in the files too. Editing one of its files somewhere else starts the
  collection's history over; that file's own `u` still works.
- Card borders, the `⋮ N lines not shown` rows and the title are not text.
  An edit that would touch one, or join the last line of a card with the
  border under it, is refused and changes nothing.
- `:%s/old/new/g` in a collection substitutes inside the cards only, with
  the usual live preview.

### One command at every hit

Vim's quickfix commands work on the collection you are in, or the one you
were in last:

| Command | Does |
| --- | --- |
| `:cnext` `:cprev` (`:cn` `:cp`) | the same as `]q` `[q` |
| `:cc N` | go to hit N |
| `:cdo {cmd}` | run an ex command with the cursor on every hit line |
| `:cfdo {cmd}` | run an ex command once in every file of the collection |

```vim
:cdo s/open_picker/open_the_picker/
:cdo norm A;
:cfdo w
```

`:cdo` and `:cfdo` run whatever the `:` line runs — `:s`, `:norm`, `:d`,
`:g` — and return to where you were. Each command finishes before the next
starts and is one undo step in its file; the files stay open. The `c` flag
of `:s` cannot ask in the middle of a batch and is refused there.

## Replace across files

Replace is two steps.

1. `Space R`, type the term, `Tab`, type the replacement. (If Search already
   holds a term, `Space R` keeps it and starts in **With**.) Every hit shows a
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
named in the heading. A search with every hit excluded, or
one that failed or has not finished, cannot be replaced.

**With** is literal unless the term is a regular expression with groups
(`regex:`, or a bare term with `Alt-R` on). Then `$1`,
`${name}` and `\1` are that hit's own groups, `$0` the whole hit, `$$` a
dollar sign:

```text
Find  regex:"get_(\w+)\(\)"
With  fetch_$1()
```

Replace needs exactly one thing to look for. A query that joins several
terms with `AND` or `OR` can be searched but not replaced; With says so.
Plain words such as `request id` are one phrase, and replace fine.

## Rename

`grn` opens `:rename` with the name under the cursor, ready to edit (`grr`,
`grn`, `gra` and `gri` are Neovim's language-server keys: references, rename,
actions, implementation). A rename
that touches one buffer is applied at once (and `u` there undoes it). One
that touches several opens the same review as a project replace: `Enter`
applies and writes, `u` reverts.

## Searching on an SSH host

Search works the same in a directory on an SSH host: the same fields, the
same include and exclude keys, the same previews, the same limits. It runs
`rg` on the host. If the host has no `rg`, Search says so and does not run.
A search on a host that takes longer than two minutes is stopped.
Collections work there. Replace does not, yet: `Space R`, `Ctrl-T` and the
With field say so.
