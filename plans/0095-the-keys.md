# 0095 — The keys

- Status: **in force** (built: §3 has the corpus as it stands, §8 what is left)
- Date: 2026-11-10
- Adopts: [0091](0091-editor-critique-0.56.md) §2 and §7 "the keys";
  [0092](0092-registers-and-pinned-project.md) §3
- Amends: [0001](0001-vision-and-design.md) §5.10 (the differential corpus),
  [0016](0016-one-input-machine.md) (the walker gains a Visual table),
  [0003](0003-modes-leader-and-house-style.md) §5.7 (the Missing list)

The owner's direction: "if at the end we are Neovim reimplemented in Rust that
should be fine." Neovim with no configuration (`-u NONE`) is the
specification — the Neovim of the Docker `test` image, which is the gate:
0.12.2 today. (The critique measured 0.9.5, and so does a development
machine that has that version installed; §3 lists where the two differ.) Where Neovim's default differs from Vim's, Neovim's wins
(`Y` is `y$`, Visual `*` searches the selection). The target is a day of work
without a key doing something else.

0091 §2 measured 151 everyday sequences: 80 differed. The differential harness
that is supposed to find such things had 72 cases. Both are fixed here: the
keys, and the harness that keeps them fixed.

## 1. What is wrong, by cause

Reading 0091 §2 and 0092 §3 against the source gives five causes, not eighty.

1. **The vocabulary is small.** `strop-grammar` knows five operators, twenty
   motions and four text objects. `gU`, `ip`, `'a` are not wrong; they do not
   parse.
2. **The resolver lacks three of Vim's range rules.** `:help exclusive-linewise`
   (an exclusive motion ending in column 1 gives the line break back), the `w`
   rule (the last word moved over ends at its line), and "a motion that cannot
   move fails" (`dk` on line 1). Each is one place after motion resolution.
3. **Counts repeat the command instead of parameterising it.** `3J`, `3x`,
   `5~`, `2D` ran the single command N times: one join too many, N register
   writes, `$` never left the line.
4. **Visual mode is a second, smaller editor.** Its keys are a hand-written
   `match` in `visual.rs` (`d y c x > <`); everything else fell into the
   grammar and was refused. It needs the table Normal has.
5. **The `:` line is a `match` on whole strings.** No abbreviations, no
   ranges beyond numbers, a dozen commands.

## 2. Design

### 2.1 Grammar (`strop-grammar`)

The resolver stays pure and stays the one consumer-facing function
(doctrine 2). It grows by data.

- **Operators**: `gU` `gu` `g~` `g?` `!` join `d y c > <`.
  `Op::from_keys(&str)` replaces `from_key(char)`; a doubled operator
  (`gUU`, `gUgU`, `g??`, `!!`) is `Target::Linewise` as `dd` is. `=` `gq`
  `gw` `zf` are parsed too, so that they take their motion, but are not
  implemented: the engine answers that they are not supported (§2.7).
- **Motions**: `-` `+` `Enter` `(` `)` `[(` `[{` `])` `]}` `g_` `g0` `gj`
  `gk` `N%`, counted `$` (down N−1 lines), and three that need editor state:
  marks (`'a` `` `a `` and the special marks `'< '> '' '. '^`), the
  window lines (`H` `M` `L`), the repeats (`;` `,` `n` `N`), and the column
  `gj` / `gk` want.
- **Text objects**: `ip` `ap`, `it` `at`, `ib` `iB` (aliases of `i(` `i{`),
  `gn` `gN`; counts on all of them (`d2aw`, `2i(` for the second enclosing
  pair). In Visual, an object extends a selection that is already more than
  one character (`viwiw`, `vipip`, `vi(i(`).
- **Forcing**: `v` `V` between operator and motion (`Command::force`), as
  `:help o_v` defines. `Ctrl-V` parses and resolves to nothing: a forced
  block needs a column model the resolver does not have.
- **Editor-bound targets.** A mark, a window line or "the last search" is not
  in the buffer, so the resolver cannot see it. The parser emits the unbound
  form (`Motion::Mark { name, exact }`); the engine binds it to a position
  immediately before resolution (`Editor::bind_command`), and dot-repeat keeps
  the unbound command so `.` reads the mark again. A command that reaches the
  resolver unbound resolves to nothing. No callback, no second resolver.
- **Vim's cursor model.** Vim's word, sentence and object code is written
  against a cursor that can stand on a line's end and steps from there to
  the next line. `resolve/walk.rs` is that cursor, and `words.rs`,
  `blocks.rs` and `objects.rs` are ports of `fwd_word`, `bck_word`,
  `end_word`, `bckend_word`, `current_word`, `findpar`, `findsent`,
  `current_par` and `current_block` over it. Porting, not re-deriving, is
  what makes the edge cases agree: an empty line is a word, the last word
  of a line ends there, `di{` takes whole lines.
- **Range rules**, applied once in `resolve::operated`, in Vim's order:
  forcing, exclusive-linewise (`:help exclusive-linewise`), whole lines for
  the operators that act on lines, then the `d` exception (`:help d`).
  Every target is first a `Span` — two positions and how the far one counts
  — so motions and text objects pass through the same rules.
- **A motion that cannot move fails**: `j` on the last line, `k` on the
  first, `2dd` on the last line, a search with no match resolve to nothing;
  the operator is cancelled and the register is not touched. A motion that
  can only not move (`h` in column 1) gives an empty region instead: `c`
  still opens Insert there, the others do nothing.
- **Where the cursor ends** is part of the answer (`Resolved::origin`,
  `end_adjusted`): Neovim's `nostartofline` keeps the column after a
  linewise delete or shift, a yank leaves the cursor at the start of what
  it yanked.

### 2.2 One operator engine

`normal/execute.rs` applies a resolved plan; `visual.rs` had its own copy for
selections. They become one function, `normal/operators.rs`
`apply_operator`, with Normal passing the plan's ranges and Visual passing
the selection (one range, or one per row for a block). The new operators are
arms of it, so every operator works in Normal, in all three Visual modes,
with a register and under dot-repeat without a second implementation. A
block's `d y c` and an occurrence selection's keep their own code: a block
has its own register shape, occurrences keep their cursors.

The new files are functions over `&mut Editor`, not new `impl Editor`
blocks, and Insert's new state is one owned struct (`insert::keys::
InsertKeys`): the ratchet of 0087 A4 (`editor/tests/ratchet_tests.rs`) holds.

### 2.3 Visual has a table

The binding table gets a second compiled index: rows whose sections include
`visual` are Visual's keys, looked up before the grammar. `d x y c s > < U u
~ g? ! J gJ r p P I A o O * # D X Y C R Ctrl-A Ctrl-X Ctrl-C S` are rows
whose handler is typed data, `Handler::Visual(VisualVerb)`, and receive the
count and the register as Normal's do. Those rows are left out of Normal's
index, so one key can mean two things in two modes (`u` lowercases in
Visual) without the first row in the table winning.

Dot-repeat after a Visual operator replays it over the same extent from the
cursor, as Vim does: N lines for a linewise selection, N characters on one
line, otherwise the selection's line and column deltas.

### 2.4 Normal and Insert

- `x` `X` are `dl` `dh` and `D C Y` are `d$ c$ y$`: counts, registers and dot
  come from the grammar instead of a loop.
- `R` is an Insert session that overtypes; Backspace restores what it
  replaced. `Ctrl-A` / `Ctrl-X` find the number at or after the cursor
  (decimal, `0x`, `0b`; a leading `-`) as Neovim's default `nrformats` does.
- `J` `gJ` take a count of lines; `~` a count of characters; `&` `g&` `@:`
  repeat the last substitute or command line; `gp` `gP` `]p` `[p`; `g*` `g#`;
  `gi` `gv` unchanged.
- Insert: `Ctrl-W` (word back), `Ctrl-U` (to the start of the inserted text
  or the line), `Ctrl-O` (one Normal command), `Ctrl-T` / `Ctrl-D` (indent),
  `Ctrl-H`.
- Marks move with the text, and a mark whose line is deleted is gone, as in
  Vim (it used to slide to the line that took its place).
- `j` `k` `G` `gg` keep the column the cursor wants through shorter lines
  (Vim's curswant); only block Visual did before.
- A Ctrl chord with no meaning is dropped at the frontend instead of
  arriving as its letter (`Ctrl-Y` was `y`, `Ctrl-K` typed `k`).

### 2.5 Registers (0092 §3)

Visual `p` replaces the selection and leaves the replaced text in `""`; `P`
leaves `""` alone. `"#` is the alternate file. Fields (pickers, prompts)
write registers when they yank or delete. Appending to a blockwise register
re-pads its rows.

### 2.6 The `:` line

`editor/ex/` replaces `normal/ex.rs`: a range parser (`N . $ % 'a '< /pat/
?pat? +N -N`, `,` and `;`), a command table with Vim's abbreviations and
`!`, and one function per command. The table is the source for completion and
for the dispatch — one list, as `EX_COMMANDS` was meant to be.

- Ranges are honoured by every command that takes one; `:` from Visual is
  prefilled with `'<,'>` and returns to Normal.
- `:s`: `&` `\0`–`\9` `~` `\n` `\r` `\t` and `\u \l \U \L \E` in the
  replacement; flags `g i I n c &`; `:&` `:&&` `:~`. `c` asks at each match
  with the match highlighted (`y n a q l`).
- `:g` `:v` (and `:g!`), `:norm[al]`, `:m`, `:t`/`:co`, `:d [x]`, `:y [x]`,
  `:pu`, `:j`, `:>` `:<`, `:sort`, `:[range]!cmd`.
- `:b {n|name}` `:bn` `:bp` `:bd` `:ls`, `:reg`, `:marks`, `:wa` `:wqa`
  `:xa` `:x` `:up`, `:cd` `:pwd`.
- **`:norm` is a function**, `Editor::ex_normal(range, keys)`, so the change
  track's `:cdo norm …` composes with it. This plan builds no quickfix.
- **`:cd` is the one way the project root changes** (0092 §2.3 pins it).
  It re-anchors what is keyed by `Editor::cwd`: relative buffer names, the
  session and recovery files, picker catalogues, the LSP "outside the
  project" test.

**Case.** `:s` compiles its pattern with the compiler `/` uses and so has
`/`'s case policy, as in Vim, where both obey `ignorecase` and `smartcase`;
`\c` and `\C` override it per pattern. This plan does not change that
policy: the find track (0093) makes it smart case for `/`, the finder and
project search at once, and `:s` follows. The cases written for this plan
do not depend on case folding; eight inherited or generated ones do, and are
listed as one known divergence (§3).

### 2.7 What is still missing is listed

`keymap/missing.rs` holds two lists: **Missing** (Neovim binds it, strop does
not: folds, tabs, `=`, `gq`, `U`, `K`, `q:`, …) and **Reassigned** (Neovim
binds it, strop uses the key for something else: `Q`, `gd`, `gI`, `gr`,
`gs`, Visual `S`, …). `strop compat` and `docs/vim-compat.md` print both,
and a test holds the checked-in page to the generated one. Another feeds
every Missing row and asserts it does not dispatch: the text, the cursor's
line and the mode are unchanged, nothing is left pending, no prompt opens —
a row that starts working fails until it is moved into the table.

A missing *operator* is the dangerous kind: `gq` refused, then `ip` typed,
is Insert mode and a `p` in the buffer. So `=`, `gq`, `gw` and `zf` are
parsed as operators, take their motion, and then say they are not supported.

## 3. The corpus

`crates/strop/tests/differential/` replaces the one file:

| File | Cases | Source |
|---|---|---|
| `hand.rs` | 95 | behaviours somebody once got wrong (the old list) |
| `critique.rs` | 151 | 0091 §2, verbatim, on its ten-line buffer |
| `keys.rs` | 174 | what this plan added that the generator does not reach |
| `ex.rs` | 186 | ranges, `:s`, `:g`, `:norm`, registers, buffers |
| `generated.rs` | 3,467 | operator × target × count × mode |

4,073 in all; the suite takes about 40 seconds on the development machine.

**Which Neovim.** The gate's: the `test` image installs Alpine's `neovim`
(0.12.2). Nine cases answer differently under 0.9.5, and strop follows the
gate: `gcc` is the comment operator (0.10+), Visual `*` / `#` search the
selection without yanking it, `:s///c` is one undo step, and an inner
bracket object in Visual selects nothing when the block is empty (`()`).
A machine with an older Neovim sees those nine as divergences; the image
is the authority.

**The generator.** A case is `position + operator + target`, rendered three
ways: Normal (`{op}{count}{target}`), Visual (`v{count}{target}{op}`) and
dot-repeat (the Normal case, `j`, then `.`). `c` types a marker and Escape.
The 64 targets are every motion and text object of §2.1 that needs no
second buffer. Two fixtures (prose with sentences, paragraphs, an indented
block and trailing blanks; code with nesting, quotes, a tag pair and
numbers) and six start positions in each (line start, mid-word, inside
brackets and a string, an empty line, the last line). `d` runs from every
position, `c` and `y` from one per fixture, the other six operators from
one position in one fixture: they share the resolver, so more positions
would test `d` again. The names are derived
(`gen/code@4:14/normal/d2aw`), so a failure names its own reproduction.

The compared state is the final text, cursor, mode, `""`, `"0 "1 "2 "- "a
"b` and `modified`. Cases run one process pair each, on every core.

**Divergence policy.**

- A divergence is a bug until it has a reason. `divergences::KNOWN` is capped
  at 10 entries; an entry is one case or one named family (a prefix) and
  says what differs and why strop keeps it.
- An entry whose cases all agree again fails the run until it is delisted.
- A generated combination that Neovim itself refuses or that depends on
  state the harness does not set (window height, a second buffer, the
  clipboard) is not generated; it is not a divergence.
- The floor is in the test: fewer than `MINIMUM_CASES` (600) fails.

**The known divergences**, four reasons covering 18 cases:

1. *Missing* (7 critique cases): folds, `U`, `=`, `gq` / `gw`, search
   offsets. On the Missing list; nothing happens.
2. *Reassigned* (2): `gd` and `gI` are the language server's.
3. *Search folds case* (8): `/edge` finds `Edge`; after `gU`, `/in` still
   finds `IN`. The policy is 0093's.
4. *A refused `daw`* (1, inherited): Vim's failed `current_word` still moves
   the cursor a column.

Two Neovim behaviours are bugs strop reproduces rather than lists, because
they are cheap and exact: a shift that changes no line still sets
'modified', and a case operator over an empty motion from column 1 runs to
the line's end (one character on the first line).

## 4. Tests beyond the corpus

- Engine unit tests for what the harness cannot see: Replace mode's
  Backspace, `Ctrl-O`, register contents after Visual `p` / `P`, marks moving
  with edits, `:cd` re-anchoring, `:s///c` answers, `:ls` / `:reg` / `:marks`
  text.
- The keymap tests gain the Visual index and the Missing assertion.
- A cell-grid golden for the `:` line prefilled with `'<,'>` from Visual.

## 5. Order of work

1. Harness split and the critique cases (the baseline: 77 of 246 diverge).
2. Grammar vocabulary and range rules; the generator.
3. One operator engine; the Visual table.
4. Normal, Insert and register items.
5. The `:` line.
6. Missing/Reassigned lists and `strop compat`.
7. The tagline (§6).

## 6. The tagline

0091 §3.3: "see the cut before you make it" is true on four surfaces and
false on the grammar. Once the corpus is green it is replaced, everywhere the
product says it (welcome card, `--version`, `install.sh`, crate descriptions,
README), by **"Vim's keys. Nothing to configure."** The website is a separate
change.

## 7. Not in this plan

Folds, tabs, `:set` beyond what the editor already has options for, a
`[keys]` remap, quickfix and `:cdo` (0094), `/` smartcase (0093), the
`gr*` family of Neovim 0.11 (`grr` `grn` `gra` `gri`, 0094).

## 8. Left for later

Each is on the Missing list or stated here, so none is a silent gap.

- **`=`** needs the structural indenter to answer "what indent does this
  line want" for an arbitrary line; today it answers for the line being
  opened or closed. **`gq` / `gw`** need Vim's comment-leader rules to be
  worth having. **`U`**, folds, tabs.
- **Search offsets** (`/pat/e`, `/pat/+1`): `Motion::Search` carries a
  query and no offset.
- **Sentence objects** `is` / `as`: the motion `(` `)` is ported
  (`findsent`); `current_sent` is not.
- **`"=`**, the expression register: there is no expression language.
- **`Ctrl-V` as an operator force** (`d<C-v>j`): parsed, resolves to nothing.
- **Visual `p` of a block**, and of the clipboard read asynchronously: the
  put over a selection reads the register file synchronously.
- **`:cd`** does not restart running language servers or change the
  process's own working directory; collection buffers keep their old labels
  until rebuilt; it refuses while a write or a recovery read is in flight.
- **A count before a Visual `>`** shifts once.
