# 0112 — The help page: key inks, tabs, a readable column

- Status: **implemented 2026-11-18** (accepted the same day); every taste
  question (§12) is taken at its recommendation. §14 records where the
  code moved the plan.
- Source: the owner — "revamp the help page though please, it is very clean
  atm, but the commands are all orangy, like we need syntax highlighting for
  the commands? like <space> or <leader> is highlighted differently from say
  c or h, ...etc. also feel free to organize it differently, do tabs in it,
  ...etc."
- Builds on 0003 §5.7 (every dispatchable binding renders in `?`), 0088 (the
  look: four greys, one accent), 0107 (where-am-I goes on the modeline, no
  sticky rows), 0110 (one command registry feeds `?`).
- Amends 0088 §2 rule 6 (help's key column is highlighted text, not one
  keycap) and 0088 §6 "Help stays one column" (still one column, now with a
  fixed measure). Adds no colour to any palette, no setting.
- Paths are relative to `/home/tarek/workspace/strop/`. `E/` =
  `crates/strop-engine/src/`, `R/` = `crates/strop/src/render/`.

One correction to the brief: the default palette is **hone** (amber
`#e8a848`), not kiln (0088 amendment 2026-11-16, `ThemeId::default()`). The
goldens are hone and vellum. The table in §4 gives both hone and kiln values.

## Decisions

1. **Key cells are lexed and inked per token.** Plain keys keep the accent;
   `space`/`esc`/`enter` and friends, `ctrl-`/`alt-` chords, placeholders and
   separators each get their own ink (§4). Five inks, all existing palette
   roles.
2. **The raised slab behind the key column is removed.** Tokens sit on the
   base plane. Colour, bold and the column do the work.
3. **Rows are typed where they are generated.** The engine keeps a `HelpPage`
   beside the buffer (one `LineKind` per line); the renderer stops
   re-parsing row shapes out of text. Help becomes its own
   `DocumentSource::Help`, replacing three `name == "help"` string checks.
4. **One pure lexer module**, `E/keymap/notation.rs`, owns the key notation:
   `keys`, `command`, `filter`. `lookup::NAMED` moves there. No allocation.
5. **Nine tabs, one buffer.** Line 1 is the tab row; every tab's content is
   in the same buffer, under a heavy heading `━ Title ━`. Nothing is ever
   hidden, so `/` still searches all of help.
6. **Switching: `]]` / `[[`** (Vim's section motions, count allowed) and
   **`enter` on the tab row**. `Tab`, `h`/`l`, digits and `gt`/`gT` keep
   their normal meaning.
7. **No sticky row.** The tab row scrolls with the text like any line; the
   modeline says `help › Git` (the 0107 slot).
8. **One fixed key column (24 cells) and a 76-cell measure.** Descriptions
   wrap onto continuation lines; a key cell wider than 24 puts its
   description on the next line. Nothing is cut at 80 columns.
9. **No line numbers in help.** The gutter is the two-cell list gutter.
10. **Notation is unchanged** (`space f`, `ctrl-w h`, `esc`): it is what
    which-key, the modeline hints, the welcome card and the docs print.
11. **Search is the buffer's `/`. No filter, no second search.**
12. Hand-written rows that mix a context word into the key cell are rewritten
    so the cell holds only what is typed (§8).

## 1. Today

`E/editor/help.rs` (584 lines) writes one string: a header line, then per
section a `─ Title ─` line and rows `   {keys:<width}   {desc}`. `R/help.rs`
(128 lines) re-derives each row's shape from the text and paints the whole
padded key column as one keycap: `accent` bold on `bg.raised`.

```
   1  strop <version> · help · / search · q close                                 ▮
   2
   3  ─ Moving ────────────────────────────────────────────────────────────────
   4     h j k l                       move (never off the line)
  14     g_ g0 g^ g$ gj gk             last non-blank; line start, first non-bl
  20     ctrl-d ctrl-u ctrl-f ctrl-b   half/full page scroll (count = lines)
```

(`R/golden/help-hone-80x24.golden`; rows 3–38 of the style summary are all
`29:#e8a848/#23272f+b`: a 29-cell amber slab.)

| # | Defect | Evidence |
|---|---|---|
| H1 | Every key cell is one ink. `space`, `ctrl-w`, `<c>`, `/` the separator and `/` the search key look the same. | `R/help.rs:74-81` |
| H2 | The key column is as wide as the section's widest cell. Panes has a 62-cell cell (`ctrl-w h ctrl-w j …`), so at 80 columns every Panes description is off screen. | `E/editor/help.rs:332-339`, `E/keymap/bindings/panes.rs` |
| H3 | Descriptions are cut at the pane edge; strop has no soft wrap. At 80 columns rows 14, 15 and 18 of the first screen are cut. | `R/golden/help-hone-80x24.golden` |
| H4 | About 25 sections in one scroll (12 keymap, 4 command families, Query language, 6 hand-written, LSP servers), no index, no way to jump but `:help {title}`. | `E/editor/help.rs:326-395` |
| H5 | The renderer guesses a row's kind from its text (`starts_with("strop ")`, first double space). | `R/help.rs:23-90` |
| H6 | Hand-written cells mix words with keys: `Find enter`, `enter in With`, `buffer / ?`, `input ctrl-r {reg}`, `visual ctrl-c`. A lexer cannot tell `Find` from a key. | `E/editor/help.rs:28-304` |
| H7 | `ctrl-o / ctrl-i (tab)` puts a parenthesis in a key cell; `expand` and `per_key` read `(tab)` as an alternative of five keys (read, not run). | `E/keymap/bindings/panes.rs`, `E/keymap/lookup.rs:9-55` |
| H8 | The line-number gutter takes 6 of 80 cells for numbers nobody uses in help. | golden above |
| H9 | Help is identified by `buf.name == Some("help")` in three places. | `E/editor/help.rs:313,407`, `R/buffer/rows/mod.rs:123` |

What works and stays: it is a real readonly buffer (`/`, motions, yank, `q`,
`Ctrl-O`), generated from the binding table, the command registry and the
filter schema; `:help {topic}` reaches a heading or a command's row.

## 2. How others do it

- **Vim / Neovim help.** Plain text hard-wrapped at 78 columns, a contents
  list at the top whose entries are followed with `Ctrl-]`; `<Esc>`,
  `{motion}`, `[count]` in a "special" ink, tags in another. Neovim's help
  and Markdown filetypes map `]]` / `[[` to the next / previous heading and
  `gO` to an outline. *Take:* one searchable text, a fixed measure, `]]`/`[[`,
  a contents line you follow from the cursor. *Leave:* tags and `Ctrl-]`
  (strop has no tag stack), the angle-bracket notation (§12 T5).
- **Helix.** No help page; which-key only.
- **rootle (0003 §5.7), lazygit, k9s.** A keybindings popup with a sidebar or
  tabs; each tab shows only its own rows; `Tab` or `h`/`l` switch. *Take:*
  the tab row as the index. *Leave:* hidden rows (a `/` that cannot see the
  other tabs) and stealing `h`/`l`/`Tab` from a buffer.
- **0107** already decided the general question: a row pinned over the text
  "covers real rows of the buffer with rows that are not at that position";
  where-am-I belongs on the modeline.

## 3. The page

### 3.1 Tabs

| # | Tab | Parts, in order | Source |
|---|---|---|---|
| 1 | Move | Moving, Search | keymap sections `moving`, `search` |
| 2 | Edit | Editing, Insert | `editing`, `insert` |
| 3 | Visual | (no sub-heading) | `visual` |
| 4 | Leader | (no sub-heading) | `leader` |
| 5 | Windows | (no sub-heading) | `panes` |
| 6 | Git | (no sub-heading) | `git` |
| 7 | Lists | Pickers, Folder, Search workspace, Collections, Review, Query language | `pickers`, `folder`, hand-written, schema |
| 8 | Commands | Command line, Command line: text, … buffers, … files and windows, … strop | `cmdline`, `ex::Family::ALL` |
| 9 | More | Terminal, Remote, Indentation, Documentation, LSP servers | `terminal`, hand-written, LSP registry |

A tab with one part prints no `─ Title ─` line under its heading.
`keymap::SECTIONS` keeps its order (it is which-key's grouping order); the
help's order is this table, in `E/editor/help/content.rs`. A test asserts
every `SECTIONS` id and every `Family` appears in exactly one tab.

### 3.2 Lines

| Line | Text | Kind |
|---|---|---|
| 1 | ` Move  Edit  Visual  Leader  Windows  Git  Lists  Commands  More` (one leading space, two between names; 64 cells) | `Tabs` |
| 2 | ` strop {version} · ]] [[ section · enter open · / search · q close` | `Hints` |
| 3 | empty | `Blank` |
| per tab | `━ {Title} ━`, empty | `TabHeading`, `Blank` |
| per part (tabs with two or more) | `─ {Title} ─` | `Heading` |
| per row | two spaces, key cell padded to 24, three spaces, description | `Entry` |
| wrapped description | 29 spaces, text | `Continuation` |
| between parts and tabs | empty | `Blank` |

Constants: `KEY_COLUMN = 24`, `MEASURE = 76`, description width
`76 − 2 − 24 − 3 = 47`. Descriptions wrap at word boundaries (a word longer
than 47 stays whole). A key cell wider than 24 is printed alone and its
description starts on the next line as a `Continuation`. A planned row keeps
its `  (soon)` suffix on the last line of its description.

The renderer extends `━ Title ━` and `─ Title ─` to the pane's text width
with the same glyph (paint only, as today).

### 3.3 Mock-ups (80×24, two-cell gutter, scrollbar column at the right)

As opened; the cursor is on the `M` of `Move`:

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890123456789
   Move  Edit  Visual  Leader  Windows  Git  Lists  Commands  More
   strop 0.71.0 · ]] [[ section · enter open · / search · q close

  ━ Move ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  ─ Moving ────────────────────────────────────────────────────────────────────
    h j k l                    move (never off the line)
    w b e W B E                word / WORD motions
    zz zt zb                   center/top/bottom the cursor line
    H M L                      top/middle/bottom visible line
    g;                         older change (changelist)
    g,                         newer change (changelist)
    ge gE { }                  word-end back / paragraph motions
    - +                        line up / down, on the first non-blank
    ( )                        sentence back / forward
    [( [{ ]) ]}                the unmatched bracket before / after
    g_ g0 g^ g$ gj gk          last non-blank; line start, first non-blank,
                               end; line down / up
    gn gN                      select the next / previous search match (cgn
                               then . changes each)
    0 $ G %                    line/file/pair jumps
    gg                         top of file
    enter                      line down, first non-blank (collection: open
  NORMAL    help › Move                                              0%    1:2
```

After `4]]` (or `:help windows`, or `enter` on `Windows`); the heading is the
top row:

```
  ━ Windows ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    :vs :sp                    split vertical / horizontal
    space w h / l / j / k / w / 1 / 2 / 3 / 4 / 5 / 6 / 7 / 8 / 9
                               pane move / cycle / focus 1-9
    ctrl-o / ctrl-i            jump back / forward (jumplist; tab is ctrl-i)
    space w v / -              pane split (vertical / horizontal)
    space w q                  close pane (last → buffer)
    ctrl-w h / j / k / l / w / W / p
                               window: left / down / up / right / next /
                               previous / last used (ctrl-w ctrl-{key} is the
                               same)
    ctrl-w s / v / n           window: split below / split right / new empty
                               buffer below
    ctrl-w c / q / o           window: close / quit (:q) / keep only this one
    ctrl-w =                   window: equal sizes (panes always are)
    ctrl-l                     redraw: full repaint when the terminal desyncs

  ━ Git ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    space g                    git…
    space g g                  status: changed files
    space g c                  commit (:wq commits, :q! aborts)
  NORMAL    help › Windows                                          31%  139:1
```

Command rows (Commands tab; the descriptions here are stand-ins, the
registry's own text is kept):

```
  ─ Command line: files and windows ───────────────────────────────────────────
    :w[!] [file]               write (! over a file that changed on disk)
    :e[dit][!] [file]          edit a file; ! drops unsaved changes
    :bd[elete][!] [number]     close a buffer
    :tab-size [N|auto]         source width, 1-16; no argument opens the
                               selector
    :remote [root|home|connect|disconnect|clear|list]
                               choose an SSH destination
```

At 140 columns the page is the same text; the right of column 78 is empty
and the rules run to the pane edge.

## 4. Inks

### 4.1 Token → palette role

All on `bg.base`. Every role already clears 4.5:1 on base and surface in
every palette (`strop-core/src/theme/tests.rs`,
`inks_clear_their_floors_on_base_and_surface`).

| Token | Examples | Role (`Tokens` field) | Weight | Hone | Kiln |
|---|---|---|---|---|---|
| `Key` — a key typed as written | `h` `gg` `]d` `dd` `"+y` `:` | `accent` | bold | `#e8a848` | `#dc7a59` |
| `Named` — a key with a name | `space` `esc` `enter` `tab` `backspace` `delete` `up` `down` `left` `right` `home` `end` | `syn.link` | bold | `#7fb2d4` | `#a8b2f5` |
| `Chord` — a modified key | `ctrl-w` `ctrl-^` `ctrl-space` `alt-c` `s-tab` | `syn.type` | bold | `#8ecdb8` | `#8cc7b6` |
| `Placeholder` — stands for what you type | `<c>` `<a>` `{reg}` `[file]` `{cmd}` `N` in `[N\|auto]`, the value in `kind:fn` | `syn.string` | regular | `#aec37f` | `#a6c98a` |
| `Separator` — not typed | ` / ` between alternatives; `[ ] { } \|` and `/` in a command's usage | `muted` | regular | `#858c9a` | `#928d83` |
| `Command` — a command name | `:w` `:bd` `:remote` `strop`, a filter key `language:` | `accent` | bold | `#e8a848` | `#dc7a59` |
| `Tail` — the optional rest of a name | `elete` in `:bd[elete]`, `!` in `[!]` | `accent` | regular | `#e8a848` | `#dc7a59` |
| `Space` | the blanks between tokens | none | — | — | — |

| Other parts | Role | Weight | Hone | Kiln |
|---|---|---|---|---|
| Description, continuation | `text` | regular | `#e6e1d6` | `#f2eee6` |
| Label cell (not typed: `scope`, `detection`, a language name) | `secondary` | regular | `#a8adb8` | `#c4bfb3` |
| Planned row (whole row, tokens not inked) | `muted` | italic | `#858c9a` | `#928d83` |
| Tab heading: `━ ` and the rule | `faint` | regular | `#4a5160` | `#5a564f` |
| Tab heading: title | `accent` | bold | `#e8a848` | `#dc7a59` |
| Part heading: `─ ` and the rule | `faint` | regular | as above | |
| Part heading: title | `text` | bold | `#e6e1d6` | `#f2eee6` |
| Tab row: active tab, name plus one cell each side | `on_accent` on `accent` | bold | `#15171c` on `#e8a848` | `#141312` on `#dc7a59` |
| Tab row: other tabs | `muted` | regular | `#858c9a` | `#928d83` |
| Hints: `strop {version}` | `text` | bold | | |
| Hints: key runs | lexed as a `Keys` cell | | | |
| Hints: words and `·` | `muted` | regular | | |

The active tab is the one that holds the cursor: on line 1, the tab whose
name is at or left of the cursor column (the first tab left of every name);
on lines 2–3, the first tab; elsewhere the tab whose lines contain the
cursor. So `w`/`b` along the tab row move the filled chip, and it shows what
`enter` will open.

Search matches, Visual selection and the caret compose over these spans as
they do today: the spans' concatenated text is the buffer line's exact bytes.

Known limit: in Paper and Paper Dark `syn.link` and `syn.type` are the same
colour, so `space` and `ctrl-w` share an ink there; both still differ from a
plain key. A palette test asserts `accent` differs from `syn.link`,
`syn.type` and `syn.string` in every theme.

### 4.2 The lexers (`E/keymap/notation.rs`)

```rust
pub enum TokenKind { Key, Named, Chord, Placeholder, Separator, Command, Tail, Space }
pub struct Token<'a> { pub kind: TokenKind, pub text: &'a str }

/// The names of keys that are not written as themselves.
pub const NAMED: &[&str] = &["space", "esc", "enter", "tab", "backspace",
    "delete", "up", "down", "left", "right", "home", "end"];

pub fn keys(cell: &str) -> impl Iterator<Item = Token<'_>>;     // a binding row's key cell
pub fn command(cell: &str) -> impl Iterator<Item = Token<'_>>;  // a command's usage
pub fn filter(cell: &str) -> impl Iterator<Item = Token<'_>>;   // language:rust
```

Pure, no allocation, lossless: the tokens' texts concatenated are the input.
`lookup.rs` drops its own `NAMED` and asks `notation` whether a word is
`Named` or `Chord` (same set as today plus `delete`, `home`, `end`).

**`keys`** — words are split at single spaces (each run of spaces is one
`Space` token):

1. a word `/` that is not the first word → `Separator` (the same rule as
   `lookup::expand`; a leading `/` is the search key);
2. a word in `NAMED` → `Named`;
3. a word that starts with `ctrl-`, `alt-` or `s-` and has more after it →
   `Chord`;
4. a word that starts with `:` and is longer than one character → hand the
   text from here to the next word that starts with `:` (or the end) to
   `command`;
5. otherwise, left to right inside the word: `<` + ASCII letters + `>` →
   `Placeholder`; `{` + ASCII letters + `}` → `Placeholder`; every other run
   → `Key`. (`<` and `{` alone, `i{`, `[{` stay keys.)

**`command`** — one character-level pass with a bracket depth:

1. the name: from the start to the first space, `[`, `{` or `/` → `Command`;
2. `[`, `]`, `{`, `}`, `|` → `Separator`; `/` outside brackets → `Separator`;
3. a `[…]` group whose `[` directly follows an ASCII letter of the name, or
   the `]` of such a group, is the name's tail: its content → `Tail`;
4. everything else, inside or outside brackets → `Placeholder`;
5. a word that starts with `:` after a space starts a new command (rule 1).

So `:bd[elete][!] [number]` is `Command(:bd)` `Sep([)` `Tail(elete)` `Sep(])`
`Sep([)` `Tail(!)` `Sep(])` `Space` `Sep([)` `Placeholder(number)` `Sep(])`;
`:s/pat/new/[flags]` is `Command(:s)` `Sep(/)` `Placeholder(pat)` `Sep(/)`
`Placeholder(new)` `Sep(/)` `Sep([)` `Placeholder(flags)` `Sep(])`;
`:[range]!cmd` is `Command(:)` then a placeholder group and
`Placeholder(!cmd)`; `strop ssh://host/path` is `Command(strop)` and
placeholders.

**`filter`** — per word: up to and including the first `:` → `Command`, the
rest → `Placeholder`; a word with no `:` → `Key`.

## 5. The model

```rust
// E/editor/help/page.rs
pub struct HelpPage { pub tabs: Vec<Tab>, pub lines: Vec<LineKind> }
pub struct Tab { pub title: &'static str, pub name: Range<usize> /* bytes on line 0 */, pub heading: usize /* line */ }
pub enum LineKind {
    Tabs,
    Hints { keys: Vec<Range<usize>> },   // byte ranges of the key runs
    TabHeading,
    Heading,
    Entry { cell: Cell, keys: Range<usize>, planned: bool },  // byte range of the key cell, unpadded
    Continuation { planned: bool },
    Blank,
}
pub enum Cell { Keys, Command, Filter, Label }
impl HelpPage {
    pub fn tab_at(&self, line: usize, byte_in_line: usize) -> usize;
    pub fn tab_named(&self, name: &str) -> Option<usize>;      // case-insensitive
}
```

- A `Builder` in `page.rs` pushes text and kinds together (`tabs`, `hints`,
  `tab_heading`, `heading`, `entry`, `blank`) and does the padding and the
  wrap; `debug_assert!(lines.len() == buffer line count)` when it finishes.
- `DocumentSource::Help { page: Box<HelpPage>, return_to: Option<JumpRecord> }`
  replaces `Output` for help. `open_temporary_output`'s return-point handling
  (`E/editor/document/mod.rs:385-410`) covers it the same way it covers
  `Output`; closing restores the origin view (the existing tests
  `closing_help_*` keep passing).
- `Editor::help_page(doc) -> Option<&HelpPage>` reads it; "is this help" is
  that accessor everywhere.
- `Editor::gutter` (`E/editor/prepare/geometry.rs:415`) returns
  `(Gutter::List, LIST_GUTTER)` for a help document.
- `R/buffer/rows/owner.rs`: `Pane.help: bool` becomes
  `help: Option<&HelpPage>`, checked before the gutter match;
  `Owner::Help(&HelpPage)` paints an empty two-cell gutter and
  `help::line_spans(t, page, line, text, text_cells, active_tab)`.
- `R/help.rs` is a `match` on `LineKind`; it calls `notation::{keys, command,
  filter}` on the cell's byte range and maps `TokenKind` to a style through
  one `fn ink(t: &Tokens, kind: TokenKind) -> Style`.
- The modeline: the breadcrumb slot (0107, `R/statusline/scope.rs`) shows the
  active tab's title when the focused document is help: `help › Git`.

Code layout (each file under the 400-line mark):

| File | Holds |
|---|---|
| `E/keymap/notation.rs` | `NAMED`, tokens, the three lexers, their tests |
| `E/editor/help/mod.rs` | `open_help`, `open_help_topic`, `help_key`; the one `impl Editor` block (the ratchet's count does not move) |
| `E/editor/help/page.rs` | `HelpPage`, `LineKind`, `Cell`, `Builder`, wrap |
| `E/editor/help/content.rs` | the tab table, the hand-written rows as typed data (§8), the query-language and LSP-server rows |
| `E/editor/help/tests.rs` | the existing four tests, ported, plus §9 |
| `R/help.rs` | `line_spans`, `ink`, tests |

## 6. Keys

| Key | Where | Does |
|---|---|---|
| `]]` | help | cursor to the next tab's heading (the first heading below the cursor line); the heading becomes the top row. `{count}]]` repeats. On the last tab: nothing. |
| `[[` | help | the last heading above the cursor line: the current tab's heading first, then the previous tab's. `{count}[[` repeats. Above the first heading: line 1. |
| `enter` | help, cursor on line 1 | open the active tab (same landing as `]]`). On any other line `enter` is the normal line-down motion. |
| everything else | help | unchanged: motions, counts, `/ ? n N *`, yank, Visual, `:`, `q`, `Ctrl-O`, `Tab` (= `Ctrl-I`) |

- Each jump records a jump first (`push_jump`), so `Ctrl-O` goes back.
- Wiring: `Editor::help_key(key) -> bool` beside `status_key`, `directory_key`
  and `review_key` in `feed_readonly` (`E/editor/git_memory/mod.rs:223`); the
  `]`/`[` prefix is read from the walker the way the status buffer's `]c`
  does (`:209-221`).
- Table rows, so they render in `?` (0003 §5.7), both `Handler::Contextual`,
  section `moving`: `]] [[` — "help: next / previous section"; and the
  existing `enter` row's description gains "help: the section under the
  cursor".
- `E/keymap/missing.rs:78`: `]]` and `[[` leave the list; the row becomes
  `][ [] ]m [m` — "section-end and method motions". Regenerate
  `docs/vim-compat.md` (`strop compat > docs/vim-compat.md`;
  `keymap/tests.rs:437` checks it).
- `:help {topic}` resolves in this order: a tab title; a keymap section id
  or title; any part heading's title; a command by any of its names (its
  `Entry` line, found in the model by the command's usage, not by text
  prefix). A tab or heading lands as the top row. The `:help` completion
  menu (`E/editor/ex/complete/arguments.rs:41`) lists the tab titles first.

User-facing text calls a tab a **section** (`]] [[ section`): it is Vim's
word for what `]]` moves over, and "tab" will mean tab pages once 0113 lands.

## 7. Search and narrow terminals

- **Search.** `/`, `?`, `n`, `N`, `*` are the buffer's and cross tabs; the
  modeline names the tab a match is in. A wrapped description can split a
  phrase across two lines, as in Vim's help. No `:filter`, no fuzzy field: a
  narrowed help buffer hides rows from `/`, which is the reason help is a
  buffer.
- **80 columns and wider.** Every line is at most 76 cells; nothing is cut.
- **Under 79 columns.** Lines scroll sideways with the cursor like any long
  line. The tab row (64 cells) fits down to 67 columns. No second layout.
- **Under 30 columns** the description column starts off screen; `$`/`w`
  reach it. Not designed for.

## 8. Content edits

Typed cells for the hand-written rows (everything not listed keeps its text):

| Now | Cell kind | New key cell | Description change |
|---|---|---|---|
| every `BINDINGS` row | `Keys` | — | — |
| every command registry row (`ex::help_rows`) | `Command` | — | — |
| filter schema rows, `kind:{value}` rows | `Filter` | — | — |
| LSP servers: the language name | `Label` | — | — |
| `-language: -kind: -path:` | `Filter` | — | — |
| `AND OR NOT ( )`, `ctrl-space`, `$1 \1`, `tab`, `esc`, `u`, `q`, `enter`, `ctrl-o`, `ctrl-t`, `space /`, `space R` | `Keys` | — | — |
| `tab / s-tab`, `ctrl-x / ctrl-d`, `]q / [q`, `]f / [f`, `+ / -`, `space o / :remote` | `Keys` | — | — |
| `unknown filter`, `scope` (both), `SSH listings`, `filter-only Search`, `quoted text`, `old -t / -g flags`, `file expressions`, `quotes`, `With`, `:s replacement`, `re-entry`, `edit`, `detection` | `Label` | — | — |
| `:[range]s/a/b/[g]`, every `:…` row in Remote, Collections, Indentation; `strop ssh://host/path` | `Command` | — | — |
| `buffer / ?` | `Keys` | `/ ?` | prefix "Buffer search: " |
| `buffer case:ignore` | `Filter` | `case:ignore` | prefix "Buffer search: " |
| `input ctrl-r {reg}` | `Keys` | `ctrl-r {reg}` | unchanged (it already says "in any input") |
| `visual ctrl-c` | — | deleted | the Visual tab has the row |
| `Find enter` | `Keys` | `enter` | prefix "In Find: " |
| `With enter` | `Keys` | `enter` | prefix "In With: " |
| `enter in With` | `Keys` | `enter` | prefix "In With: " |
| `enter on hover` | `Keys` | `enter` | prefix "On a hover card: " |

Keymap rows (`E/keymap/bindings/panes.rs`), using the notation's own
alternatives so dispatch is unchanged:

| Now | New |
|---|---|
| `ctrl-o / ctrl-i (tab)` — "jump back / forward (jumplist)" | `ctrl-o / ctrl-i` — "jump back / forward (jumplist; tab is ctrl-i)" |
| `ctrl-w h ctrl-w j ctrl-w k ctrl-w l ctrl-w w ctrl-w W ctrl-w p` | `ctrl-w h / j / k / l / w / W / p` |
| `ctrl-w s ctrl-w v ctrl-w n` | `ctrl-w s / v / n` |
| `ctrl-w c ctrl-w q ctrl-w o` | `ctrl-w c / q / o` |

`expand` yields the same sequences for the three `ctrl-w` rows (read from
`lookup.rs:9-55`; the keymap tests confirm it). The first row loses its
`(tab)` alternative.

## 9. Tests

Unit, `E/keymap/notation.rs`:

- lossless: for every `BINDINGS` row, every registry usage and every filter
  example, the tokens' texts concatenated equal the input;
- a table of cells → token kinds: `h j k l`; `space w v / -`; `/ ?`;
  `ctrl-\ ctrl-n`; `f<c> F<c>`; `'<a>`; `d y c > < gU`; `ge gE { }`;
  `ctrl-r {reg}`; `up down left right tab s-tab`; `:terminal [cmd] :term`;
  `space o / :remote`; `:bd[elete][!] [number]`; `:s/pat/new/[flags]`;
  `:sor[t][!] [u][n][i]`; `:[range]!cmd`; `:tab-size [N|auto]`;
  `language:rust`; `-language: -kind: -path:`;
- agreement with dispatch: for every `BINDINGS` row, the words `keys` marks
  `Separator` are exactly the `/` words `expand` treats as alternatives.

Unit, `E/editor/help/tests.rs`:

- the four existing tests, ported (headings now `━ Git ━` or `─ Moving ─`;
  the LSP row assertion reads the model, not column padding);
- `lines.len()` equals the buffer's line count; every `Entry`'s `keys` range
  is non-empty except the "no built-in server" row;
- every keymap section and every command family is in exactly one tab;
- no line is wider than 76 cells; no key cell wider than 24 shares a line
  with a description;
- `]]` walks all nine headings in order and stops; `3]]` from line 1 lands
  on Visual; `[[` from inside a tab lands on its own heading, again on the
  previous; `Ctrl-O` returns; `enter` on `Git` in the tab row lands on
  `━ Git ━` with it as the top row; `enter` on a row is line-down;
- `:help move`, `:help moving`, `:help query language`, `:help :w`;
- `/half/full page` from the top finds the Move tab's row; `n` crosses into
  another tab.

Unit, `R/help.rs`: per `LineKind`, the spans' text equals the line and each
token kind wears its role (hone); the active tab is `on_accent` on `accent`
and moves with the cursor column on line 1; a planned row has no accent.

Theme: `strop-core/src/theme/tests.rs` — `accent` differs from `syn.link`,
`syn.type`, `syn.string` in every palette. `R/theme_tests.rs:117` — help no
longer has a `raised` background; assert `syn.link` and `accent` foregrounds
instead.

Goldens (`R/golden_tests.rs`, hone and vellum, 140×40 and 80×24):

| Name | Scene | State |
|---|---|---|
| `help` | `Space ?` | 4 files rewritten |
| `help-windows` | `Space ?`, `:help windows⏎` | 4 new: overflow rows, chords, separators, `:vs :sp`, the modeline's `help › Windows` |
| `help-commands` | `Space ?`, `:help commands⏎` | 4 new: usage tails, placeholders, the command-line keys |

Also rewritten: any golden whose frame shows help behind another surface
(none known; `STROP_UPDATE_GOLDEN=1` and read the diff).

## 10. Implementation steps

Each step compiles and passes the gate on its own.

1. **Notation.** Add `E/keymap/notation.rs` with `NAMED`, the tokens and the
   three lexers and their tests. Point `lookup.rs` at it; delete
   `lookup::NAMED`.
2. **Keymap rows.** The four `panes.rs` rows of §8. Run the keymap tests.
3. **Module split, no behaviour change.** `E/editor/help.rs` →
   `help/{mod,page,content,tests}.rs`; the hand-written tables move to
   `content.rs` as `(Cell, keys, desc)` rows with the §8 edits; the builder
   still emits today's text. Tests pass unchanged except the edited rows.
4. **Typed document.** `DocumentSource::Help`, `Editor::help_page`, the three
   name checks replaced, `Owner::Help(&HelpPage)`. `R/help.rs` reads
   `LineKind`; still today's look.
5. **Layout.** The builder emits §3.2: tab row, hints, tab headings, fixed
   column, wrap, overflow. `open_help_topic` reads the model. List gutter for
   help. Port the engine tests; add the §9 layout tests.
6. **Inks.** `R/help.rs` per §4.1; delete the keycap slab; theme tests.
7. **Keys.** `help_key`, the two table rows, `missing.rs`, regenerate
   `docs/vim-compat.md`, `:help` completion lists tabs, the modeline slot.
8. **Goldens.** Rewrite `help-*`, add `help-windows-*` and `help-commands-*`;
   read every diff.
9. **Docs** (§11), `plans/STATUS.md` (0112 in force), the 0088 rule 6 and §6
   amendment lines, and the `plans/0028` items of §13.
10. **Gate.** `docker compose run --build --rm test`; a tmux smoke at 80×24
    and 140×40 in hone, kiln and vellum: open, `]]` through every tab, `/`
    across tabs, `yy` a row, `q`.

## 11. Docs

- `README.md:59-68` — the `Space ?` sentence: sections on the top line,
  `]]`/`[[` to move between them, `:help TOPIC`.
- `docs/editor.md` — the help paragraph (around line 140) and the `:help`
  completion line (282).
- `docs/vim-compat.md` — regenerated (step 7).
- `CHANGELOG.md`, next version, Changed: "**The help page is rebuilt**: keys
  are coloured by what they are (plain keys, `space` and other named keys,
  `ctrl-` chords, placeholders), a top line lists nine sections and `]]` /
  `[[` or `Enter` on a name jumps to one, descriptions wrap instead of
  running off an 80-column screen." Fixed: the Windows rows whose
  descriptions were off screen at 80 columns.
- **Site** (`stropdev/stropdev.github.io`, after the release exists): vendor
  `CHANGELOG.md`; the docs pages rebuild from `docs/*.md`; add a `help`
  scenario to `tools/frames.py` (`Space ?`, then `:help leader`) and capture
  it **from the release binary** into `img/` and `frames.json` — no
  hand-drawn frame (0004 §4). There is no help frame on the site today.

## 12. Taste questions

| # | Question | Recommendation |
|---|---|---|
| T1 | Plain keys: keep the accent (amber/terracotta), or make them cream bold and keep the accent for `space` only? | **Keep the accent.** A plain key then looks the same in help, which-key and every hint; the page stops being one colour because named keys, chords, placeholders and separators each take their own. |
| T2 | Keep the grey slab behind the key column? | **Remove it.** Five inks on a slab is busy, and `muted` separators fall under 4.5:1 on `raised` in three palettes. |
| T3 | Keys for switching: `]]`/`[[` + `enter`, or also `Tab`/`S-Tab`, or `gt`/`gT`? | **`]]`/`[[` + `enter` only.** `Tab` is `Ctrl-I` (jump forward) in a terminal; `gt`/`gT` belong to tab pages (0113). Saying yes to `Tab` costs `Ctrl-I` inside help and nothing else. |
| T4 | A tab row that stays on screen while you scroll? | **No.** It needs a per-pane header row in the pane geometry (proved layout code) and contradicts 0107. The modeline names the section. |
| T5 | Vim's notation (`<Space>f`, `<C-w>h`, `<CR>`, `{char}`) instead of `space f`, `ctrl-w h`, `enter`, `<c>`? | **Not in this plan.** It changes which-key, the modeline hints, the welcome card, the README, the docs and the site together. The lexer is the one place that knows the notation, so a later switch is contained. Worth its own decision. |
| T6 | Line numbers in help? | **Drop them.** Four more cells for text at 80 columns. If kept: every mock-up shifts right by four cells and the measure becomes 72. |
| T7 | Wrap at a fixed 76 cells (Vim's help), or use the whole width on wide terminals? | **Fixed 76.** Generating to the pane width means regenerating on every resize and split; a fixed measure is one text and reads better than 130-cell lines. |
| T8 | The nine tabs and their names (§3.1). | As listed. The one judgement call: Search sits under Move, Terminal under More. |
| T9 | The word on screen: "section" or "tab"? | **"section"** in the hint line and docs; 0113 will make "tab" mean a tab page. |
| T10 | A help frame on strop.dev? | **Yes**, one frame of the Leader section. |

## 13. Not in this plan (→ `plans/0028`)

- Which-key, the welcome card and footer hints taking the same key inks
  (they use `card::keycap`, one ink). Do it once the help has been lived in.
- `gO` (Neovim's outline) for help.
- `Ctrl-]` to follow the word under the cursor; strop does not decode the key.
- The `ranges` row of the Command line section
  (`:N :N,M :. :$ …`) inks every range as a command name; a ranges lexer is
  not worth its lines yet.

## 14. Amendments

Implementation notes, where the plan met the code:

- **2026-11-18 — `keys` rule 1.** A `/` word that is the last word of a
  cell is a key, not a separator: `space /` is the leader's search key, and
  `lookup::expand` reads it as part of the `space` sequence. Rule 1 reads
  "a word `/` that is neither the first nor the last word"; the agreement
  test (§9) holds for every row with it.
- **2026-11-18 — the tab row.** Line 1 ends with one space after `More`
  (65 cells), so the last name's chip has a cell on both sides.
- **2026-11-18 — the modeline.** The breadcrumb slot follows the name
  group with the modeline's group gap, so it reads `help [RO]  Git`, not
  `help › Git`; the slot is sized to the longest section name.
- **2026-11-18 — steps 3 and 4** landed as one commit: the typed `Cell`
  has no reader until the document carries it, and the gate refuses a dead
  field.
- **2026-11-18 — step 2** also touched `ex::table`'s
  `a_listed_key_does_what_its_command_does`, which looked a command's key
  up as text in a row's cell; it now reads the row through `expand`
  (`ctrl-w o` is in `ctrl-w c / q / o`).
- **2026-11-18 — §6 completion.** The sections come first, then the keymap
  groups (ids) as before, then commands; `:help gi⇥` still completes to the
  `git` id.
