# 0110 — The command line

- Status: **in force** (0.71).
- Source: the owner — "our command palette, like ':' it needs some polish
  honestly … see what kind of redesign it will propose." The proposal is
  appended as §A; every taste question (§A.5) takes its recommended answer.
- Amends 0003 §1 and §5.8 (Esc cancels; the line lives on the bottom row),
  0098 §3.4, 0088.

## Decisions
1. The command line is the bottom row (vim), mode chip kept; the top card is
   deleted.
2. `Esc`/`Ctrl-C` cancel at once; Normal-key editing of the line is the
   command-line window (`Ctrl-F`, `q:`, `q/`), a real buffer of history.
3. Vim's editing keys (`Ctrl-W/U/B/E/R/V`, `Home`/`End`/`Del`,
   `Left`/`Right`), each rendered in `?`.
4. History: `Up`/`Down` filter by the typed prefix; 200 lines per project.
5. The menu opens as you type, nothing selected; the top row is what Enter
   runs; prefix matches, then fuzzy; Tab/S-Tab cycle and wrap back.
6. One command registry is the single source for dispatch, completion,
   usage, descriptions and `?`; argument completion by declared kind.
7. `:s` previews in place; the row shows the off-screen count; errors use
   error ink.
8. No separate command palette.

---

## §A. Proposal


Status: proposal for the owner. Read-only investigation; nothing in the repo was
edited, built or run. Behaviour below marked **(read)** was derived by reading
the code, not by running the editor — the repro keys are what to type to confirm.
If adopted this becomes `plans/0106-the-command-line.md` and amends 0003 §1 and
§5.8, 0098 §3.4 and 0088 rule 9 (for this surface).

Paths are relative to `/home/tarek/workspace/strop/crates/`.

---

## 1. Current state

### 1.1 What `:` is today

- **One prompt slot** for `:`, `/`, `?` and `|`: `PendingInput` holding a
  `TextPrompt` (`strop-engine/src/editor/pending.rs:43,135`). Keys go through one
  reducer (`pending.rs:223-297`); effects are applied in
  `editor/normal/pending.rs:137-165`.
- **Drawn as a floating card at the top centre** (`strop/src/render/cmd_card.rs`):
  row 1, 60 % of the columns (min 32) (`cmd_card.rs:71-73,84`), titled
  `command` / `search` / `pipe` (`:89`). Inside: the sigil in a two-cell gutter,
  the typed text, a thin `▏` caret; then up to six completion rows; then, for
  `:s`, a summary row and up to eight `-`/`+` rows.
- **The mode chip** reads `COMMAND` (`render/statusline/chip.rs:26`); the rest of
  the modeline keeps showing path and position.
- **Messages** (errors, completion lists) go to the echo row at the *bottom*,
  one row above the modeline (`render/echo.rs`).
- **Commands**: one table, 94 rows (`editor/ex/table.rs:92`), used for dispatch
  (`ex/mod.rs:156-188`), for the completion list (`ex/prompt.rs:26-38`), and
  checked against `?` by a test (`keymap/tests.rs:287`). Ranges are complete and
  differential-tested (`ex/range.rs`, `strop/tests/differential/ex.rs`).
- **`:s` preview** is real: one parsed spec and one plan feed preview, apply and
  confirm (`ex/substitute/mod.rs:232-279`). This part of the doctrine holds.

### 1.2 Defects and rough edges

| # | What | Evidence | Repro | Neovim |
|---|---|---|---|---|
| D1 | **No history at all.** Only the last line is kept, for `@:`. `Up`/`Down` do nothing. | `normal/pending.rs:179` (the only store); `pending.rs:282` swallows every other key | `:2d⏎` `:` `Up` | `Up`/`Down` recall, filtered by what is typed; `Ctrl-P`/`Ctrl-N` unfiltered; kept across sessions |
| D2 | **No vim editing keys.** `Ctrl-W`, `Ctrl-U`, `Ctrl-B`, `Ctrl-E`, `Ctrl-A`, `Ctrl-N`, `Ctrl-P`, `Shift-Tab`, `Ctrl-V`, `Ctrl-R Ctrl-W` are all dropped silently. | `pending.rs:282` `Key(_) => None` | `:w foo` `Ctrl-W` — nothing happens | all work |
| D3 | **`Home`, `End`, `Delete` do not exist as keys** anywhere in the editor. | `editor/input/frontend.rs:123` (`_ => return None`); `Key` enum has no such variants | `:w foo` `Home` | work |
| D4 | **`Ctrl-C` does not cancel** the line. | same arm, `pending.rs:282` | `:w` `Ctrl-C` — still open | cancels |
| D5 | **`Esc` does not cancel**; it puts the line in a "field NORMAL" mode, a second `Esc` cancels. The modeline then says `prompt pending · field NORMAL · Esc cancels`. | `pending.rs:249-252`; `render/statusline/spec.rs:19-31`; `cmd_card.rs:93-103` | `:w` `Esc` — line still open | `Esc` cancels at once; editing a line with Normal keys is the command-line window (`Ctrl-F`, `q:`) |
| D6 | The differential harness cannot see D5: it compares `mode`, which is `Normal` while strop's line is still open. | `tests/differential/ex.rs` case `"visual colon returns to normal"` passes with one `<esc>` | — | — |
| D7 | **`q:` / `q/` / `Ctrl-F` missing** (already listed). | `keymap/missing.rs:60` | `q:` | command-line window |
| D8 | **Tab does not cycle.** The candidate list is recomputed from the line, so after the first Tab only the completed name matches. | `ex/prompt.rs:43-59` | `:w` `Tab` → `:write`; `Tab` again — stays `write`; `wall`, `wq` unreachable **(read)** | cycles through every match and back to what was typed |
| D9 | **The list lies about Enter.** Candidates are in table order; `exit` (row 329) is before `edit` (row 360). The top row is drawn in the accent as "the one". | `ex/table.rs:329,360`; `cmd_card.rs:200-204`; `ex/prompt.rs:33-37` | `:e` — top row `exit`, `Tab` gives `:exit`; but `:e⏎` runs `edit` **(read)** | — |
| D10 | **A bare `:` shows six arbitrary rows**: `substitute & ~ global vglobal normal` — the first six of the table. No count, no "more", no scrolling; rows 7+ are invisible. | `ex/prompt.rs:30` (empty prefix matches all); `cmd_card.rs:183` `take(6)` | `:` **(read)** | empty line; `Tab` lists all |
| D11 | **No completion after a range or `!`.** The list needs the whole line to be letters. | `ex/prompt.rs:30` | `:%sor` `Tab`; `:'<,'>nor` `Tab` | completes |
| D12 | **Prefix only**, no fuzzy; nothing can be selected with arrows or `Ctrl-N`. | `ex/prompt.rs:35` | `:sym` finds `symbols`, `:tloc` finds nothing | `wildoptions=fuzzy` available |
| D13 | **Path completion keys on the typed spelling**, not the command: only `e e! view sp split vs vsplit`. | `editor/remote_completion/mod.rs:34-39,257` | `:edit sr` `Tab`, `:vsp sr` `Tab`, `:w sr` `Tab`, `:cd sr` `Tab`, `:r sr` `Tab` — nothing | all complete |
| D14 | **Path candidates are shown in the echo row** at the bottom while you type in a card at the top; the cursor must be at the end of the line. | `remote_completion/mod.rs:437,501-527`; `:267` | `:e src/` `Tab` | popup menu at the cursor |
| D15 | **No argument completion** for `:b` (buffers), `:theme`, `:help`, `:set`, `:remote`, `:recover`, `:tab-size`, `:indent-style`, `:cdo`/`:g` (a nested command). | no code | `:b sta` `Tab` | completes |
| D16 | **`:help {topic}` knows section titles only.** | `editor/help.rs:219-245` | `:help :s`, `:help write` → "no help topic" | `:help :s` |
| D17 | **The card covers the text `:s` is changing.** At 80×24 it hides rows 2–13, in a 44-cell column, so changed lines are cut on both sides. | golden `render/golden/substitute-hone-80x24.golden` | `:%s/modeline/statusline/g` | `inccommand=nosplit`: the buffer itself shows the result |
| D18 | **`:s` is parsed and its regex compiled three times per frame**, in the renderer. | `cmd_card.rs:38`, `statusline/spec.rs:32`, `buffer/rows/overlays.rs:21` → `normal/preview.rs:17` → `substitute/mod.rs:245` → `substitute/parse.rs:160` | — | — |
| D19 | **Candidates are computed in the renderer every frame** (a fresh `Vec`), plus `format!` per row. | `cmd_card.rs:65,183,213` | — | — |
| D20 | **Errors look like information.** `run_ex` writes a plain string; the echo row draws it with `›` in text ink. Handlers return nothing, so a failed command is indistinguishable from a quiet one. | `ex/mod.rs:191-195,211`; `render/echo.rs:33-40`; `table.rs:31` | `:wrte⏎` | red `E492: Not an editor command` |
| D21 | **Question at the top, answer at the bottom.** The line is typed at row 2; its error appears at row *h*−2; the echo row also moves when the cursor is on the last row. | `cmd_card.rs:84`; `echo.rs:139` | `:foo⏎` | same row |
| D22 | **`?` help for ex is a second hand-written list** (20 grouped rows) beside the table's 94 doc strings; aliases repeat their text (`bNext`/`bprevious`, `ls`/`buffers`/`files`, `qall`/`quitall`). | `keymap/bindings/ex.rs:16-138` vs `ex/table.rs` | — | — |
| D23 | Usage and description are one string, so the list shows `:w[!] [file]: write (! over a file…` cut at 35 cells. | `table.rs:308`; golden `modeline-command-hone-80x12.golden` | `:w` | — |
| D24 | The three surfaces disagree on words: card title `command`, chip `COMMAND`, docs "ex line", "ex command line", "the `:` line"; `· field NORMAL ·` is coined. | `cmd_card.rs:89,101`; `keymap/bindings/ex.rs:17` | — | "command line" |

What already works and must not regress: ranges, `'<,'>` from Visual, `:s`
preview from the shared plan, `Ctrl-R {register}`, paste, origin restore on
cancel, stale-reply rejection for path completion, the `: / ? |` unification.

---

## 2. Research

**Neovim** (0.11/0.12). The line is the bottom row. `wildmenu` is on and
`wildoptions=pum`: `Tab` opens a popup menu above the line, at the word, listing
names only; `wildmode=full` cycles and writes each match into the line;
`Ctrl-N`/`Ctrl-P` move, `Ctrl-Y` accepts, `Ctrl-E` cancels; `Ctrl-D` lists.
`wildoptions+=fuzzy` exists. 0.12 (March 2026) adds `wildtrigger()` so the menu
can open as you type (with `wildmode=noselect`), and an experimental `ui2` that
re-houses the line and messages. `inccommand=nosplit` is the default: `:s` shows
its result in the buffer for visible lines; `split` adds a window listing
off-screen changes. History: `Up`/`Down` filter by the typed prefix; saved in
shada. `Ctrl-F` / `q:` open the command-line window: history as a real buffer,
edited with every Normal key, `Enter` runs the line. Errors are red, at the line.

**Helix.** `:` prompt on the bottom row. A menu of commands appears above it
*as you type*, fuzzy-matched; `Tab`/`Shift-Tab` cycle; a small box shows the
selected command's description and aliases. Each command declares a completer
for its argument (paths, buffers, themes, settings). No ranges.

**Kakoune.** Prompt in the status line; candidates above it; once the command
name is complete an info box shows that command's usage and doc string and stays
while you type the arguments.

**Zed / VS Code.** A centred modal palette: fuzzy over human titles, key binding
at the right, recently used first. Zed's vim mode opens the same palette for `:`
with vim names aliased in; ranges and previews are second-class.

**What fits strop:**

- Neovim's *place* (bottom), *keys*, *history*, *command-line window* and
  *in-place `:s`* — all of it, unchanged.
- Helix/0.12's menu that opens as you type, nothing selected — it costs nothing
  and makes the commands discoverable.
- Helix's rule that **each command declares what its argument is**.
- Kakoune's usage text for the command whose argument you are typing.
- From the palettes: the key column only. Not the modal, not the titles.

---

## 3. Proposal

### 3.1 Decisions

1. **The command line is the bottom row** — the modeline's own row, while a line
   is open. Nothing floats, no buffer cell is covered, nothing reflows, and it
   is where a vim user's eyes already are. The chip stays (`COMMAND`, `SEARCH`,
   `PIPE`); path and position give way to the typed line. The top-centre card
   (`cmd_card.rs`) is deleted. Messages stay in the echo row, now directly above
   the line that caused them.
2. **`Esc` and `Ctrl-C` cancel.** The line has no Normal mode. Editing a line
   with the full grammar is the **command-line window** (`Ctrl-F`, `q:`, `q/`):
   a real buffer of history — doctrine 4 in its plainest form. `FieldMachine`
   stays for picker fields only (Telescope's convention, which nvim users know).
3. **`:` completion is the palette. There is no second one.** Arguments:
   - strop already has three ways to find a thing: `?` (every key and command,
     searchable with `/`), the key hints on a prefix, and `Space a` (what can be
     done *here*). A fourth list of "all commands" would restate `?`.
   - Everything a palette offers is here: type a few letters, see matching
     commands with a one-line description and the key that does the same, pick
     one. `:` then `Tab` on an empty line lists every command.
   - A palette is a second vocabulary (titles) for the same commands. One name
     per command is the plain option, and it is the vim one.
   - Keys with no command name (`Space f`, `gd`) stay keys; `?` finds them.
4. **One registry** (`ex/table`) is the only source for dispatch, completion,
   usage, description, the key column and the `?` rows. `keymap/bindings/ex.rs`'s
   hand-written rows are deleted.
5. **The menu is not a picker.** It takes no keys of its own, has no field, no
   preview pane and no query language: `:e path:foo` is a file name. It shares
   with the canonical picker only (a) the scorer and match columns
   (`strop_picker::fuzzy_score`), (b) the row and card primitives in
   `render/card.rs`, (c) the "matched letters are accent + bold" rule. Commands
   that open pickers (`:theme`, `:jumps`, `:symbols`) keep doing so. The picker
   redesign needs to keep one thing stable for this: a scorer callable on a
   plain `&str` without the filter parser.

### 3.2 Behaviour

**Opening.** `:` opens an empty line; from Visual it opens with `'<,'>`. No menu
on an empty line.

**Keys** (every row renders in `?` under a new "Command line" section):

| Key | Does |
|---|---|
| `Enter` `Ctrl-J` `Ctrl-M` | run the line |
| `Esc` `Ctrl-[` `Ctrl-C` | cancel; the view returns to where it was |
| `Backspace` `Ctrl-H` / `Delete` | delete before / under the cursor; `Backspace` on an empty line cancels |
| `Ctrl-W` | delete the word before the cursor |
| `Ctrl-U` | delete to the start of the line |
| `Left` `Right` | move one character |
| `Ctrl-B` `Home` / `Ctrl-E` `End` | start / end of the line |
| `Ctrl-R {reg}` | insert a register (`"`, `a`–`z`, `0`, `:`, `/`, `+`) |
| `Ctrl-R Ctrl-W` / `Ctrl-R Ctrl-A` | insert the word / WORD under the buffer cursor |
| `Up` `Down` | older / newer history line that starts with what is typed |
| `Ctrl-P` `Ctrl-N` | older / newer history line, unfiltered (menu closed) |
| `Tab` `Shift-Tab` | next / previous completion, written into the line |
| `Ctrl-N` `Ctrl-P` | same as `Tab` / `Shift-Tab` while the menu is open |
| `Up` `Down` | move in the menu once a row is selected |
| `Ctrl-Y` | keep the selected completion, close the menu |
| `Ctrl-E` | with a row selected: put back what was typed, close the menu |
| `Ctrl-D` | open the menu without selecting |
| `Ctrl-F` | open the command-line window on this line |
| `Ctrl-L` | repaint (kept) |

Not done, listed in `strop compat` Missing: `Ctrl-V` literal, `Ctrl-K` digraphs,
`Ctrl-A` insert all, `Ctrl-G`/`Ctrl-T` in `/`, `Ctrl-\ e`.

**History.** Two lists, `:` and `/ ?`, 200 lines each. Every line run goes in,
failed ones too; an equal older line is removed; empty lines, and lines run by
`@:`, `:g`, `:normal`, macros and dot, do not. `Up` filters by the text before
the cursor at the moment recall started; `Down` past the newest restores what
was typed. Saved in the project's session file.

**Completion.** The line is split once, by a pure function, into: range ·
command name · `!` · argument — the same split dispatch uses. The cursor's part
decides the list:

- *Command name.* Rows, in order: (1) the command `Enter` would run for the
  typed word (`:e` → `edit`), (2) other names that start with it, in table
  order, (3) fuzzy matches by score. Aliases fold into one row. Works after a
  range (`:%sor`) and after `:g/pat/`, `:cdo`, `:cfdo`.
- *Argument*, by the command's declared kind:

  | Kind | Commands | Rows |
  |---|---|---|
  | file | `e w r sp vs view new vnew` | names in the typed directory; `ssh://` hosts and paths from live connections (as today) |
  | directory | `cd` | directories only |
  | buffer | `b bd` | `number  name  ●` for open buffers |
  | words | `set remote recover tab-size indent-style sort` | the fixed words with one line each |
  | theme | `theme` | theme names with their descriptions |
  | help topic | `help` | section titles and command names |
  | command | `g v cdo cfdo` (after the pattern) | commands, as above |
  | none | patterns, registers, marks, shell text | no menu |

- The menu opens by itself after the first character of a command name, and on
  `Tab`/`Ctrl-D` anywhere. Nothing is selected until `Tab`. File lists are
  fetched only on `Tab`/`Ctrl-D` (a job, never on the input path); after that,
  typing refilters the fetched directory with no new job.
- `Tab` writes the row into the line; after the last row it returns to what was
  typed. A directory keeps its `/` so the next `Tab` descends.
- At most 10 rows (fewer on short terminals); the list scrolls; the top border
  shows the count.

**Right side of the row** (muted, right-aligned, first that applies):
an error in error ink (bad regex, unset mark) · the `:s` summary · the lines a
range names (`12–18 · 7 lines`) · the `/` match count · the command's usage
(`:w[!] [file]`).

**Ranges.** While a range is typed the lines it names get the accent in the
line-number column. The hint and the highlight come from `parse_ex_range`, the
function `Enter` uses.

**`:s` preview.** In the buffer, in place (`inccommand=nosplit`): each visible
changed line is drawn as it will be, the new text in `diff.add` ink. A line
break added or removed is drawn `↵`, on the same row. The summary on the right
counts the rest: `4 changes · 1 below`. Same plan as `Enter` — the renderer
reads `SubstitutionPlan.edits`, nothing else. No `-`/`+` list.

**Errors.** `Enter` on a bad line: the line closes and the echo row, one row
above, shows `✗ not an editor command: wrte` in error ink. `:` `Up` brings the
line back. Nothing turns red while a command name is being typed.

**Command-line window.** `q:`, `q/`, `q?`, or `Ctrl-F` from an open line: a
split at the bottom, 7 rows, holding the history one line per row and the
current line last. A normal buffer: every motion, operator, `/`, yank.
`Enter` runs the cursor's line and closes; `:q` or `Ctrl-C` closes.

### 3.3 Mock-ups

Cells are indicative; the goldens are the contract. `▏` is the caret.

**Idle — 80 columns**

```
  18  │   │   Self { chip, dirty: path.exists(), line }
  19  │   }
  20  }
~
~
 COMMAND    :▏
```

**Idle — 120 columns** (identical; the row is full width)

```
  20  }
~
 COMMAND    :▏
```

**Typing a command — 80 columns** (`:w`, nothing selected)

```
  14  impl Modeline {
  15  │   fn gather(path: &Path, line: usize) -> Self {
        ╭ commands ────────────────────────────────────────────────────── 4 ╮
        │   write  w    write the buffer (! over a file changed on disk)    │
        │   wall   wa   write every modified buffer                         │
        │   wq     wq   write and quit                                      │
        │   wqall  wqa  write every modified buffer and quit                │
        ╰  tab  next   s-tab  previous   ctrl-e  dismiss ───────────────────╯
 COMMAND    :w▏                                                  :w[!] [file]
```

Columns: name (matched letters accent + bold) · shortest abbreviation (muted) ·
description. The name column starts under the first typed character.

**Typing a command — 120 columns** (`:c`, `Tab` pressed twice; key column appears)

```
        ╭ commands ─────────────────────────────────────────────────────────────────────────────────── 2/9 ╮
        │   copy       co    copy lines below the address                                                 │
        │ ▌ close      clo   close the window (never the last one)                              ctrl-w c  │
        │   cd         cd    change the project root (- goes back)                                        │
        │   cnext      cn    next hit of the collection                                               ]q  │
        │   cprevious  cp    previous hit of the collection                                           [q  │
        │   cc         cc    hit N of the collection                                                      │
        │   copen      cope  show the collection, on its current hit                                      │
        │   cdo        cdo   run a command on every hit line of the collection                            │
        │   cfdo       cfdo  run a command once in every file of the collection                           │
        ╰  tab  next   s-tab  previous   ctrl-y  accept   ctrl-e  dismiss ─────────────────────────────────╯
 COMMAND    :close▏                                                                                  :clo[se]
```

**Argument: files — 80 columns** (`:e src/re` `Tab`)

```
            ╭ src/ ────────────────────────────────────── 1/3 ╮
            │ ▌ render/                                       │
            │   replay.rs                                     │
            │   remote.rs                                     │
            ╰  tab  next   s-tab  previous   ctrl-e  dismiss ─╯
 COMMAND    :e src/render/▏                               :e[!] [file]
```

**Argument: buffers — 120 columns** (`:b st`)

```
            ╭ buffers ──────────────────────────────────────────────────────── 2 ╮
            │   3  crates/strop/src/render/statusline.rs  ●                      │
            │   7  crates/strop/src/render/statusline/layout.rs                  │
            ╰  tab  next   s-tab  previous   ctrl-e  dismiss ────────────────────╯
 COMMAND    :b st▏                                                                               :b {number|name}
```

**Range — 80 columns** (Visual `Vjj:` then `d`; line numbers 8–10 in the accent)

```
   7  /// Gather the row.
   8  pub struct Modeline {
   9  │   chip: String,
  10  │   dirty: bool,
  11  │   line: usize,
 COMMAND    :'<,'>d▏                                           8–10 · 3 lines
```

**Range error — 120 columns**

```
 COMMAND    :'q,$d▏                                                                              mark q is not set
```

**`:s` live preview — 80 columns** (new text in `diff.add`, shown here between ‹ ›)

```
   1  const LABELS: &[&str] = &["alpha", "beta", …, "omicron", "‹statusline›", "
   2  //! The ‹statusline›: one quiet row at the bottom edge.
   3  //!
   8  pub struct ‹statusline› {
  14  impl ‹statusline› {
  15  │   fn gather(path: &Path, line: usize) -> Self {
 COMMAND    :%s/modeline/statusline/g▏                    4 changes · 0 below
```

**`:s` live preview — 120 columns** (bad pattern)

```
 COMMAND    :%s/mode\(line/x/▏                                                           invalid: unclosed group
```

**Error after Enter — 80 columns**

```
  20  }
~
 ✗ not an editor command: wrte
  NORMAL    src/render/statusline.rs                                 40%   8:1
```

**History — 80 columns** (`:%s` then `Up`: the newest line starting with `%s`)

```
 COMMAND    :%s/modeline/statusline/g▏                    4 changes · 0 below
```

**Command-line window — 120 columns** (`q:`)

```
  12  }
── command line ────────────────────────────────────────────────────────────────────────────────────────────────────────
   1  w
   2  %s/modeline/statusline/g
   3  8,10d
   4  e crates/strop/src/render/echo.rs
   5  ▏
~
  NORMAL    command line                                                              enter  run   :q  close      5:1
```

**Narrow — 48 columns**: the menu drops the description first, then the
abbreviation; the right-side text drops before the typed line is ever cut; a
long line scrolls under the caret (`render/field.rs` already does this).

---

## 4. Implementation plan

Each step is one change that passes the compose gate on its own and leaves no
old path behind. Line budgets are targets (≤ ~400, ceiling 800).

### Step 0 — the plan
`plans/0106-the-command-line.md` from this document; `plans/STATUS.md` row;
amend 0003 §1 ("field Normal mode on the `: / ? |` line" → the command-line
window) and §5.8, 0098 §3.4, 0088 rule 9.

### Step 1 — one registry
`editor/ex/table.rs` (538 lines) becomes a directory:

```
editor/ex/table/
  mod.rs      types, lookup, exact_name, the COMMANDS concatenation   ~200
  text.rs     :s :g :norm :d :y :m :t :j :pu :> :< :sort :! :@ :r …   ~250
  buffers.rs  :b :bn :bp :bd :ls                                       ~120
  files.rs    :w :q :e :sp :vs :cd :set :help :messages …              ~300
  own.rs      strop's own: :theme :remote :format :rename :c* …        ~250
```

```rust
pub(crate) struct ExCommand {
    /// Primary name first; every name with its shortest abbreviation.
    pub names: &'static [Name],          // Name { name, min }
    pub range: Addr,
    pub bang: bool,
    pub edits: bool,
    pub arg: Arg,                        // what follows the name
    pub usage: &'static str,             // ":w[!] [file]"
    pub doc: &'static str,               // "write the buffer (! over a file changed on disk)"
    pub family: Family,                  // Text | Buffers | Files | Own — the ? heading
    pub key: Option<&'static str>,       // the Normal key that does the same: "]q"
    pub run: fn(&mut Editor, &ExCall<'_>) -> ExResult,   // Result<(), String>, see step 6
}
pub(crate) enum Arg {
    None, File, Directory, Buffer, Theme, HelpTopic, Command,
    Words(&'static [(&'static str, &'static str)]),
    Register, Mark, Pattern, Shell, Text,
}
```

- `editor/help.rs` writes the command rows from `COMMANDS` (usage as the key
  column, `doc` as the text, one `─ Command line: files ─` rule per family).
  `:help write`, `:help :w` land on the row.
- Delete the 20 command rows in `keymap/bindings/ex.rs` (the `:` and ranges rows
  move to the new command-line section in step 2) and the test
  `every_ex_command_has_a_row` (true by construction).
- Tests: every `key` names a live binding whose handler is that command; names
  are unique; for every command and every prefix ≥ `min`, `lookup` returns it or
  an earlier row (precedence stated once, in a table test listing the overlaps:
  `s`, `e`, `d`, `c`, `m`, `t`, `b`, `w`, `q`).

### Step 2 — the line and its keys
```
editor/cmdline/
  mod.rs      Cmdline (was PendingInput), Line (was TextPrompt), Context, open/close   ~250
  keys.rs     KEYS: &[(Key, Action)] and the reducer                                     ~300
  accept.rs   accept, cancel, origin restore, incsearch hook (from normal/pending.rs)    ~300
  tests/{keys,accept}.rs
keymap/bindings/cmdline.rs   the "Command line" section rows, one per Action             ~120
```
- `editor/pending.rs`, `editor/normal/pending.rs`, `editor/ex/prompt.rs` are
  deleted; callers move to `editor.cmdline`. `PendingFieldState/Mode/Escape`,
  `PendingEvent::CompleteEx`, the field-NORMAL branch in
  `render/statusline/spec.rs:19-31` and `Editor::input_normal` for sigil lines go.
- `Key` gains `Home`, `End`, `Delete` (`strop-core/src/frontend_input.rs`,
  `editor/input/frontend.rs`, trace and macro tables); Insert mode and picker
  fields get them in the same change (a key that exists must work everywhere).
- `Esc`/`Ctrl-C` cancel. `Ctrl-R Ctrl-W`/`Ctrl-A` read the origin pane.
- The reducer is a lookup in `KEYS`; a test asserts every `Action` has a row in
  `bindings/cmdline.rs` and the reverse (0003 §5.7 by construction).
- Differential (`tests/differential/cmdline.rs`, new, ≥ 40 cases): `Ctrl-W`,
  `Ctrl-U`, `Ctrl-B`+typing, `Ctrl-E`, `Ctrl-R "`, `Ctrl-R a`, `Ctrl-R Ctrl-W`,
  `Esc` and `Ctrl-C` cancel, `Backspace` on empty. The harness dump gains
  "command line open" (strop) vs `mode()` starting with `c` (nvim) so D6 closes.

### Step 3 — history
`editor/cmdline/history.rs` (~200): `History { lines: VecDeque<Box<str>>,
recall: Option<Recall> }` for `Ex` and `Search`; `push`, `older(prefix)`,
`newer(prefix)`, `reset`. `registers.command_line` (`":`) reads the newest ex
line — one store, the separate field is deleted. Serialised in
`strop-engine/src/session.rs` (two bounded arrays).
- State tests: prefix recall, `Down` restores the typed text, dedup moves to
  newest, cap 200, failed lines kept, `@:`/`:g`/macro lines not kept, session
  round-trip.
- Differential: `:2d⏎:⟨Up⟩⏎`, `:1d⏎:2y⏎:1⟨Up⟩⏎`, `:d⏎:⟨C-p⟩⏎`, `/two⏎/⟨Up⟩⏎`.

### Step 4 — completion
```
editor/ex/complete/
  mod.rs        Completion { kind, title, rows: Vec<Row>, typed: String, selected: Option<usize>, top }   ~250
                split(line) -> Split { range_end, name: Span, bang, arg: Span, command }  (pure; run_ex uses it too)
  commands.rs   rank(word, &mut rows): Enter's command, prefixes, fuzzy                    ~150
  arguments.rs  buffers, words, themes, help topics                                         ~200
  paths/{mod,run,directory}.rs   ← editor/remote_completion/* (moved, renamed)             as today
  tests/{split,commands,arguments,cycle}.rs
```
- `split` replaces `split_command`'s callers and the string surgery in
  `remote_completion_tab` (`body.split_once(' ')`, the literal `"e" | "e!" | …`
  list): the command comes from the registry, the kind from `Arg`.
- Path results fill `Completion.rows`; `candidates_message` and the "cursor at
  the end of the line" refusal are deleted (the argument span ends at the cursor).
- `Editor::ex_candidates` is deleted; the renderer reads `editor.completion()`.
- Tests: `split` table (`w`, `%s`, `'<,'>norm`, `g/x/d`, `e! foo`, `b `, cursor
  mid-line, `m-2`, `select-next`); **the menu cannot lie**: for every command
  and every prefix, row 0 is `lookup(prefix)`; `Tab` visits every row once and
  returns to the typed text; `Shift-Tab` reverses; `Ctrl-E` restores; typing
  after `Tab` keeps the completed text; stale path replies are dropped (existing
  tests move). Hermetic temp directory for local paths; no network.

### Step 5 — the row and the menu
```
render/menu.rs            list-card primitive lifted out of render/completion.rs:
                          rows window, ▌ selection, count in the border, hints         ~200
render/completion.rs      keeps placement and the doc pane, draws through menu.rs       ~200
render/cmdline/mod.rs     the bottom row: chip, sigil, field, right-side text           ~200
render/cmdline/menu.rs    columns per Completion kind, anchored above the row           ~200
render/cmdline/tests.rs
```
- `render/cmd_card.rs` is deleted; `statusline::render` yields its row to
  `cmdline::render` while a line is open; `cmd_card_active` → `cmdline_open`.
- The `/` match count moves to the right side of the row.
- In this step the `:s` `-`/`+` rows are drawn in the menu's place, full width,
  with `changed_windows` (moved to `render/cmdline/`); step 7 replaces them.
- Goldens (hone and vellum; 80×24, 120×40, 48×12): `cmdline-idle`,
  `cmdline-menu`, `cmdline-menu-selected`, `cmdline-files`, `cmdline-buffers`,
  `cmdline-search`, `cmdline-long-line`; regenerate `substitute-*`,
  `ex-visual-*`, `modeline-command-*`, `modeline-search-*`.

### Step 6 — errors that look like errors
`run: fn(..) -> ExResult`; handlers return `Err(text)` instead of writing
`editor.message`. `run_ex` calls `editor.fail(text)`, which sets the message and
its kind; the echo row draws a failure with `✗` in error ink
(`render/echo.rs:33-40`). `:messages` records the kind. Golden `cmdline-error`.
State tests: each error path of `run_ex_line` yields a failure; a successful
command clears it.

### Step 7 — `:s` in place, range hint
- `Cmdline` caches, per line revision, the parsed range and the parsed `:s`
  spec (`Option<Result<…>>`); `substitute_preview`, `Editor::preview` and the
  renderer read the cache (D18).
- `render/buffer/rows/substitute.rs` (~200): for a visible line with edits in
  the prepared plan, the row is drawn from the line with its edits applied, the
  new spans in `diff.add`. The text for those rows is built when the plan or
  `view_top` changes, not per frame.
- Range lines get the accent in the gutter (`render/block/gutter.rs`).
- Delete the `-`/`+` rows and `changed_windows`.
- Goldens `cmdline-substitute`, `cmdline-range`, `cmdline-range-error`,
  `cmdline-substitute-invalid`. State test: for random buffers and `:s` lines
  the previewed text of every visible line equals that line after `Enter`
  (property test; this is the doctrine as an assertion).

### Step 8 — the command-line window
`editor/cmdline/window.rs` (~250): a scratch buffer named `command line` in a
bottom split of 7 rows (0104 window tree), filled from history; buffer-local
`Enter` (Normal and Insert) and `Ctrl-C`; `Ctrl-F` carries the open line in.
Remove `q:`/`q/` from `keymap/missing.rs:60`; binding rows in
`bindings/cmdline.rs`. Differential: `:1d⏎q:k⏎`, `q:ix⟨esc⟩:q⏎`,
`:2d⟨C-f⟩⏎`. Golden `cmdline-window`.

### Performance rules (all steps)
- Everything is computed when the line changes, in the reducer; the renderer
  only reads. No candidate list, parse or regex compile in `render/`.
- Per keystroke on the name: `split` is one pass over the line, no allocation;
  ranking is one pass over ≤ 100 static rows into `Completion.rows`, cleared and
  refilled (capacity kept); the scorer is the thread-local matcher. Match
  columns are computed for the ≤ 10 visible rows only, into one reused buffer.
- Rows borrow: `Row { text: Text::Static(&'static str) | Text::Owned(Box<str>), … }`;
  command, word and theme rows never allocate.
- Directory listings, remote or local, are worker jobs keyed by (line, cursor,
  revision); a late reply for an older key is dropped. Nothing awaits.
- History recall is a scan of ≤ 200 lines.
- A bench case in `crates/strop/src/bench`: 40 keys of `:%s/foo/bar/g` and 10 of
  `:wri` + three `Tab`s, on a 100k-line buffer; a counting-allocator test in
  `strop-engine` asserts zero allocations per key for command-name completion
  after the first key.

### What gets deleted
`render/cmd_card.rs` · `editor/pending.rs` · `editor/normal/pending.rs` ·
`editor/ex/prompt.rs` · `editor/ex/table.rs` (→ directory) ·
`editor/remote_completion/` (→ `ex/complete/paths/`) · the command rows of
`keymap/bindings/ex.rs` · `candidates_message` · `PendingField*` ·
`registers.command_line` · `keymap/missing.rs:60` · the "field NORMAL" strings.

---

## 5. Taste questions

| # | Question | Recommended |
|---|---|---|
| 1 | Where does the line live: bottom row (vim) or the top-centre card (0003 §5.8)? | **Bottom row.** It fixes D17 and D21 by construction and is the vim answer. |
| 2 | `Esc`: cancel at once, or keep Esc → "field NORMAL", Esc again cancels? | **Cancel at once**, plus `Ctrl-F` / `q:` for Normal-key editing. Picker fields keep theirs. |
| 3 | Does the menu open as you type, or only on `Tab` (Neovim's default)? | **As you type, nothing selected.** `Up`/`Down` stay history until a row is selected. |
| 4 | Fuzzy matches after the prefix matches? | **Yes**, always below them. |
| 5 | `:s` preview: in place only (`nosplit`), or also a list of off-screen changes (`split`)? | **In place only**, with the off-screen count on the row. |
| 6 | Keep the mode chip on the command row, or start the line at column 0 like vim? | **Keep the chip** (0098: the row's left edge never changes). |
| 7 | History: saved per project in the session file, global, or not saved? | **Per project, 200 lines.** |
| 8 | Show the Normal key that does the same thing in the menu? | **Yes**, right-aligned, only where a test proves it is the same action. |
| 9 | Menu look: the bordered card with key hints that Insert completion uses, or a bare vim-style list? | **The same card** — one menu look in the editor. |
| 10 | Highlight the lines a typed range names, and print them on the row? | **Yes.** |
| 11 | `?`: one generated row per command (about 80 rows) instead of 20 hand-grouped ones? | **Yes** — it is the only way the help cannot drift; `/` searches it. |
| 12 | A separate fuzzy "command palette" over keys and commands? | **No.** `:` + menu for commands, `?` for keys. |
| 13 | Word for the surface in UI and docs: "command line" everywhere (chip `COMMAND`)? | **Yes**; retire "ex line", "card", "field NORMAL". |

Adjacent, not in this proposal: the echo row moves above the cursor when the
cursor is on the last row (`render/echo.rs:139`), so messages have no fixed
place; and `|` (pipe) is a strop key where vim uses `!{motion}` → `:.,.+N!`.
