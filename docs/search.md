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
inserted lines, substitution count, scope and offscreen extent; Enter applies
that displayed plan as one undo unit. Invalid or incomplete input replaces the
previous preview rather than executing it, large plans resolve off the input
path, and Esc cancellation leaves bytes, registers and undo history untouched.

The replacement is Vim's: `&` (or `\0`) is the match and `\1`–`\9` its
groups, `~` the previous replacement, `\r` a line break, `\t` a tab, and
`\u` `\l` `\U` `\L` … `\E` change the case of what follows; `\&`, `\~`,
`\\` and an escaped delimiter are themselves. The preview shows the expanded
text. This is separate from Search's literal With field.

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
  opens the selected source; Enter in **With** prepares Review. Tab switches
  fields when With is visible.
- **Ctrl-R {register}** inserts a register into Find or With, exactly as in
  insert mode and on the `:` / `/` lines: `Ctrl-R "` is the last yank or
  delete, `Ctrl-R 0` the last yank, `Ctrl-R a` a named register, `Ctrl-R +`
  the system clipboard.
- **Ctrl-X** toggles one match; **Ctrl-D** masks/restores its file.
  Excluded hits stay visible. **Ctrl-O** collects exactly the included
  matches into a read-only source-backed collection.
- **Ctrl-O** from source results opens that collection with surrounding
  context, source syntax, match evidence and line numbers. `+`/`-` adjusts
  context; `g<Space>` opens the real source for editing. Collections retain
  navigation, search and yank, but never write through to source buffers or
  save them. Use **With → Review** for deliberate multi-file changes.
- Filter-only file finding lists eligible files; content search needs an
  expression. Unignored dotfiles are included by default; `.git` metadata
  is excluded. `Alt-H` and `Alt-I` in the field turn hidden and ignored
  files on and off.
- Replacement text is literal, including `$1` and backslashes.

Opening a source hides Search; `Space /` returns to the retained
investigation. Re-entry refreshes through owned work in its captured
namespace/root even if focus or cwd changed. In a Directory buffer,
`:fs search` or **Space a → Search here** captures that directory without
changing cwd.

Review preparation is cancellable background work. **Space a** on a review or
receipt exposes **Apply to buffers**, **Save files**, **Cancel** and **Inspect
result** as applicable; `:apply-change`, `:save-change` and `:cancel-change`
are the Ex shortcuts. Apply edits buffers; Save files is a separate action
with a per-file receipt. Dirty open source text wins over disk, stale
witnesses refuse, and Cancel restores the query and draft. Failed saves retain
their exact unsaved targets and verification/recovery action. An all-excluded,
incomplete or failed dataset cannot silently become a whole-project operation.
Ordinary `u` and `:undo-change` are not interchangeable guarantees: the latter
reverts the newest applied multi-file change only while its recorded target
revisions still own that change.

## SSH scopes

Search here uses the same query fields, inclusion decisions and source
previews as local Search, on explicitly scoped SSH directories: `rg` runs
on the host over its connection, and a host without `rg` refuses search
without a fallback. **With/Review are unavailable in SSH scopes**.

Source enumeration is limited to 100,000 selected paths or 16 MiB of names;
rg records to 1 MiB and 4,096 submatches; decoded batches to 4 MiB; a
request publishes at most 100,000 rows or 64 MiB of owned row data. Limits
and malformed output produce a visible failure, never a silently complete
partial result.
