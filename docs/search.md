# Search and the query language

One bounded qualifier grammar is shared by every picker surface: file find
(`Space f`), content search (`Space /`), replace (`Space R`), document
symbols, workspace symbols (`Space S`) and the jump list menu (`ctrl-o`
history, `Space j`). Qualifiers, highlighting, suggestions and errors behave
identically everywhere — what you learn in one field works in all of them.

## The grammar

Bare file queries are fuzzy; content queries are literal unless you choose
`regex:`:

```text
language:rust
language:cpp path:src/ request_id
glob:"src/my files/**/*.rs" -path:generated/ text:"request-id"
language:rust regex:"\brequest_(id|name)\b"
hidden:exclude ignored:include text:"-test.rs"
kind:function parse AND NOT wrap
```

- `language:NAME` — file language (`strop --licenses`-style names, e.g.
  `rust`, `cpp`, `python`).
- `path:SUBSTRING` — case-sensitive substring of the workspace-relative
  path (no leading `/`).
- `fpath:SUBSEQUENCE` — fuzzy scope over the full workspace-relative path;
  use `fpath:modules/jweb/ parser` to scope the directory while ranking
  `parser`. `-fpath:` excludes matches.
- `glob:PATTERN` — glob over the workspace-relative path.
- `hidden:` / `ignored:` — visibility overrides for this query.
- `case:sensitive|ignore|insensitive` — case mode for content.
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
  qualifiers re-filter their rows locally as you type. On the jump list,
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
  matches into an editable collection.
- **Ctrl-O** from source results opens an editable collection with
  surrounding context, source syntax, match evidence and line numbers.
  `+`/`-` adjusts context; `g<Space>` opens the source. Edits update source
  buffers and their other views while typing; `:w` saves sources and `:wq`
  closes the view only after its admitted saves confirm.
- Filter-only file finding lists eligible files; content search needs an
  expression. Unignored dotfiles are included by default; `.git` metadata
  is excluded. `:search-options` shows the separate visibility controls.
- Replacement text is literal, including `$1` and backslashes.

Opening a source hides Search; `Space /` returns to the retained
investigation. Re-entry refreshes through owned work in its captured
namespace/root even if focus or cwd changed. In a Directory buffer,
`:fs search` or **Space a → Search here** captures that directory without
changing cwd.

Review preparation is cancellable background work. `:apply-change` edits
buffers; `:save-change` separately saves the changed files and opens a
per-file receipt. Dirty open source text wins over disk, stale witnesses
refuse, and Cancel restores the query and draft. An all-excluded,
incomplete or failed dataset cannot silently become a whole-project
operation.

## SSH and container scopes

Search here uses the same query fields, inclusion decisions and source
previews as local Search, on explicitly scoped SSH directories. Starting a
remote search explicitly admits a verified worker on that endpoint;
restricted/SFTP-only hosts refuse search without a fallback. **With/Review
are unavailable in SSH scopes**; individual remote write permits do not
authorize project replacement. Container Search here remains unsupported.

Source enumeration is limited to 100,000 selected paths or 16 MiB of names;
rg records to 1 MiB and 4,096 submatches; decoded batches to 4 MiB; a
request publishes at most 100,000 rows or 64 MiB of owned row data. Limits
and malformed output produce a visible failure, never a silently complete
partial result.
