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
grammar as buffer search. While composing, Strop shows the actual removed and
inserted lines, substitution count, scope and offscreen extent; Enter applies
that displayed plan as one undo unit. Invalid or incomplete input replaces the
previous preview rather than executing it, large plans resolve off the input
path, and Esc cancellation leaves bytes, registers and undo history untouched.

Replacement text is literal: `&`, `$1` and `\1` have no capture meaning. An
escaped delimiter inserts that delimiter and `\\` inserts one backslash; other
backslashes are preserved. This is separate from Search's literal With field.

## The Search workspace

`Space /` opens or resumes one large Search workspace; `Space R` resumes
the same investigation with the replacement field visible. Local
enumeration and content matching run entirely in Rust using the
`ignore`/`grep` crate family — `rg` is not required on your machine.

- **Ctrl-R** toggles the With field without restarting the search or losing
  either field's text, caret, mode, selection or workset. Enter in **Find**
  opens the selected source; Enter in **With** prepares Review. Tab switches
  fields when With is visible.
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
  is excluded. `:search-options` shows the separate visibility controls.
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

## SSH and container scopes

Search here uses the same query fields, inclusion decisions and source
previews as local Search, on explicitly scoped SSH directories. Starting a
remote search explicitly admits a verified worker on that endpoint;
restricted/SFTP-only hosts refuse search without a fallback. **With/Review
are unavailable in SSH scopes**; individual remote write permits do not
authorize project replacement.

Source enumeration is limited to 100,000 selected paths or 16 MiB of names;
rg records to 1 MiB and 4,096 submatches; decoded batches to 4 MiB; a
request publishes at most 100,000 rows or 64 MiB of owned row data. Limits
and malformed output produce a visible failure, never a silently complete
partial result. Container Search uses the same bounded protocol through the
attached incarnation's admitted worker and selected root; without that worker
it refuses, never falls back to the host. **With/Review remain unavailable in
both SSH and container scopes**; individual write permits do not authorize
project replacement.
