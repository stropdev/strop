# Editing: indentation, themes, completion, registers, windows

How strop indents, what the screen shows, how to change the theme, and how
completion, registers, windows and sessions behave. The keys are listed in
[vim-compat.md](vim-compat.md) and under `Space ?`. Searching is in
[search.md](search.md).

## Indentation

Enter between `{}`, `[]` or `()` opens an indented line in between and
leaves the closing bracket lined up with the opening line. Typing a closing
bracket on a line that holds only indentation lines it up with its opening
bracket, even when the line was indented wrongly. With several cursors,
each new line is indented for the place it is in. `.` and a count work out
the indentation again at the new place; they do not copy the spaces from
the first one.

In C and C++, typing `:` after a label moves the line where Vim would put
it. `private:`, `protected:`, `public:`, `case` and `default:` go one level
inside their enclosing brace; a named label goes to column zero. `.c` and
`.h` files get the same rules as C++. Typing `std::` does not leave the
line pulled out as a label: the second colon puts the indentation back. In
other languages, and after anything that is not a label, the colon is just
typed.

A colon at the end of a line indents the next line one level deeper, in the
languages where it opens a block:

- **Python**: a block colon (`if ready:`) indents the next line; a colon
  inside brackets (a slice or an annotation) does not.
- **C, C++, Java, C#, Go, JavaScript/TypeScript, PHP**: a label colon
  indents what follows it: access specifiers (`private:`, `public:`),
  `case` and `default:`, and named labels. In a C++ class, Enter after
  `private:` puts the members one level inside the section.
- **Rust, JSON, CSS and prose**: a colon at the end of a line never indents.

Bracket indentation works in Rust, Python, JavaScript, TypeScript, Go, C,
C++, JSON, Java, C# and CSS. Brackets inside strings and comments do not
count. In prose and in languages not listed here, a new line copies the
indentation of the line above. A new buffer with no name still indents
after an opening brace. strop does not reformat code, does not indent HTML
tags, and does not line up `end` or `else` with their opening keyword. In a
large file the indentation is worked out in the background, so typing never
waits for it.

### Indent width and tabs or spaces

strop detects each file's indentation when it opens it, and falls back to
your configured `tab_size` and `indent_style`. To change it for the current
file:

- `:tab-size 3` sets the width (1 to 16); `:tab-size auto` goes back to
  the detected or configured width.
- `:indent-style tabs`, `:indent-style spaces`, `:indent-style auto`.
- `:tab-size` or `:indent-style` alone opens a short menu of the same
  choices, with the current one marked.

Changing these settings does not change text already in the file. The
status line shows `Spaces:N` or `Tabs:N` when a file's indent differs from
your default; `:explain` says whether it was detected, configured or set by
hand.

With the cursor on a bracket, that bracket and its partner are highlighted
(the same pair `%` jumps between). The view does not scroll to show a
partner that is off screen.

## What is on screen

The accent colour marks where you are and what will happen next: the mode
chip, the current line number, a pending operator, the cursor in an input
field, and the characters a search matched. In a list, the selected row has
a `▌` at its left. `●` marks the theme in use and buffers with unsaved
changes. Pickers, the key hints, completion, hover and the `:` line are all
drawn the same way: a rounded box with its title at the top left and its
keys along the bottom edge.

Things appear only when they have something to say. The scrollbar is
hidden when the whole file fits. Pane titles appear when there are two or
more panes. `LSP!` appears when a language server failed and `LSP?` when
one was refused (see [`:trust`](#trust)). `Spaces:N` appears when a buffer's indent
differs from your default. A line under a search field appears when a
toggle is on, or when the search is not in the project directory.

Messages appear on a row above the status line, and only while there is
one to show: first the answer to your last key, then any message still
waiting for you (`✗` failure, `▲` warning, `●` info) with the key that
deals with it, `Space a` or `:explain`. The status line itself shows the
mode, the branch, the path, then, only when not zero, the diagnostics count
(`●1 ▲2`), the language-server badge, the indent and the position.

A failed save, a failed connection or a terminal that needs a decision
leaves a message that stays until you deal with it. Moving, searching or a
later success message does not clear it. `Space a` lists what you can do
about it (retry, or dismiss the message) and `:explain` shows the full
detail. Dismissing the message only hides it. It does not cancel anything
and does not mark a save as done. `:messages` lists the last 50 messages.

## Themes

`:theme` opens a list of the themes; Enter applies the selected one and
Escape leaves the current theme as it is (once goes to the picker's Normal
mode; twice closes it). `:theme vellum` switches directly. Each row shows
the theme's label, six swatches (background, panel, text, accent, keyword,
string) and `●` on the theme in use. Below the list you see the selected
theme's name, as you would type it, and its description. In a small
terminal the swatches and the description go first; the names and the
search field stay.

There are six themes. All six meet the same contrast minimums: main text at
least 10:1, secondary text 7:1, comments and syntax colours 4.5:1.

| Name | Character |
| --- | --- |
| `hone` | Graphite dark with an amber accent; the default |
| `slate` | Muted dark: syntax told apart by brightness, one brass accent |
| `carbide` | High-contrast dark for projectors and bright rooms (every colour at least 7:1) |
| `vellum` | Warm paper, graphite text, burnt-umber accent |
| `paper` | Warm off-white paper with deep green text and forest accents |
| `paper-dark` | Warm green-black with soft ivory text and sage accents |

Choose one at launch with `strop --theme vellum file.rs`, or keep the
choice in `$XDG_CONFIG_HOME/strop/config.toml` (normally
`~/.config/strop/config.toml`):

```toml
theme = "vellum"
```

`--theme` wins over the config file, and the config file over the built-in
default. `:theme` changes the running editor only; it does not rewrite the
config. `:theme` with a name strop does not have keeps the current theme
and lists the six names. A config file that names an unknown theme keeps
the default theme and shows a warning with the six names; every other
setting in the file still applies. The same goes for any key strop does
not know or any value it cannot use: that one key is skipped and named.

Syntax, pickers, previews, diffs, diagnostics, help and the status line all
follow the theme. Open terminals switch to the new theme's default colours;
colours a program inside the terminal chose for itself are left alone. The
themes are built into the binary: nothing to download or install.

## Pane titles and previews

A pane title is the pane's number and the file's path inside the project.
Two files with the same name are told apart by their directories. A file
outside the project shows its full path, and a file on an SSH host shows
the host (`ssh:devbox /etc/app.conf`). `●` marks unsaved changes and `[RO]`
a read-only buffer. In a narrow pane the directories are shortened first;
the number, the file name and the markers stay.

In Files, Search and the symbol pickers, the preview takes the spare width
when the result rows are short. When paths are long, the list keeps its
width. In a narrow terminal the preview moves below the list, or is
hidden.

## Language servers

strop starts the language server for a file's language when that server is
installed; `Space ?` lists which server each language uses. The status line
shows `LSP!` when a server failed and `LSP?` when one could not start: it
is not installed (the message names it and says how to install it, e.g.
`lsp: rust-analyzer not found on PATH — rustup component add
rust-analyzer`), or the project has not been trusted yet. A long
error from the server never pushes the mode or the position off the line.
`:explain` opens the full story in a read-only buffer: which server, which
project, and why it failed. When a server recovers, its failure message
goes away. Messages about other projects or other problems stay.

`Space s` lists the current file's symbols. The list is fetched when the
file is opened and after each edit, so it opens with the key and without
asking the server again; it is never a list from before an edit. In Rust,
Python, C, C++ and Lua files that no server serves, the list comes from
the syntax tree (functions, methods, types, modules, constants) and the
message line says so.

A server that crashes after it was ready restarts by itself, up to three
times per project: a message says `restarting (1/3)`, the `LSP!` badge
stays until the new server is ready, and every file the old server had
open, library files included, is opened on the new one. After the third
restart, or if the server never started, the next thing you ask of it (go
to definition, hover, and so on) shows the reason and tries once more to
start it. Repeat the command when the server is ready.

### `:trust`

A project can name its own language-server command in
`.strop/languages.toml`. strop does not run a command that comes from a
project until you allow that project: the message reads
`lsp: NAME needs :trust for this project`, and `:explain` shows the command.
`:trust` allows the current project, and the server starts. The answer is
remembered on this machine (under `$XDG_STATE_HOME/strop`, normally
`~/.local/state/strop`). Servers from strop's built-in list and from your
own config need no `:trust`.

## The project root

The project is the directory strop was launched in, and it stays there
until you move it: `:pwd` shows it, `:cd DIR` changes it (`:cd -` goes
back, `:cd` alone goes home). After `:cd`, the saved session and the
recovery drafts are the new project's, open buffers keep pointing at the
same files, and changes on disk are noticed under the new directory. Files
(`Space f`), Search (`Space /`) and new terminals work in the project root
whichever file has focus. Following go-to-definition into the Rust standard
library, a cargo registry crate or a system header does not change the
project: that file opens read-only, and the project's language server keeps
serving it (no second server is started in the library's directory).
Opening a folder on purpose (`:e DIR`) makes Files and Search work in that
folder while it has focus.

When a file changes on disk, a buffer with no unsaved edits reloads and
says so. A buffer with unsaved edits keeps them and tells you the file
changed; `:w!` overwrites the file, `:e!` takes the version on disk.

## Vim's keys

Neovim with no configuration is the specification: where Neovim's default
differs from Vim's, strop follows Neovim (`Y` is `y$`, `gc` comments, Visual
`*` searches the selection, `&` keeps the substitute's flags). `Space ?` lists every key;
[vim-compat.md](vim-compat.md) is the same list, and ends with what Neovim
binds that strop does not (**Missing**) or uses differently (**Reassigned**).

- **Operators** take any motion or text object, a count and a register, and
  repeat with `.`: `d` `y` `c` `>` `<`, the case operators `gU` `gu` `g~`,
  `g?` (rot13), `gc` (toggle line comments), and `!`, which opens the `:` line with the lines as its range
  so the command you type filters them. Doubled, an operator takes whole
  lines (`dd`, `gUU`, `g??`). `v`, `V` or `Ctrl-V` between an operator and
  its motion forces characterwise, linewise or blockwise (`dvj`, `dVw`).
- **Motions**: `h j k l`, `w b e ge` and their WORD forms, `0 ^ $ g_ |`,
  `gg G`, `N%`, `- +`, `{ }`, `( )`, `%`, `[( [{ ]) ]}`, `f F t T ; ,`,
  `/ ? n N * # g* g#`, `H M L`, and marks: `'a` (the line) and `` `a `` (the
  exact place), including `'<` `'>` (the last selection), `''` (where the
  last jump left from), `'.` and `'^`. Marks move with the text; a mark on a
  deleted line is gone. `j` and `k` keep the column through shorter lines.
- **Text objects**: `iw aw iW aW`, `ip ap`, `i( a(` (also `ib`), `i[ a[`,
  `i{ a{` (also `iB`), `i< a<`, the quotes, `it at` (an XML/HTML element),
  and `gn` (the next search match — `cgn` then `.` changes them one at a
  time). A count takes more of them (`d2aw`, `2i(`). In Visual an object
  extends the selection (`viwiw`, `vipip`, `vi(i(`).
- **Normal**: `x X D C Y s S`, `J` and `gJ` (a count joins that many lines),
  `~`, `r`, `R` (type over; Backspace gives the old text back), `Ctrl-A` and
  `Ctrl-X` (add to the number at or after the cursor: decimal, `0x`, `0b`),
  `p P gp gP ]p [p`, `&` and `g&` (the last `:s` again), `@:` (the last `:`
  line again), `q` and `@` macros, `Ctrl-E` / `Ctrl-Y` (scroll one line),
  `ZZ` `ZQ`.
- **Visual** (`v`, `V`, `Ctrl-V`; the same key leaves, another switches):
  every operator, `U u ~` for case, `J gJ`, `r`, `p` and `P` (see
  Registers), `I` and `A`, `o`, `*` and `#`, `D X Y C R` on whole lines,
  `Ctrl-A` / `Ctrl-X`, and `:` with the selection as its range.
- **Insert**: `Ctrl-W` (delete the word before the cursor), `Ctrl-U`
  (delete what was typed on the line), `Ctrl-T` / `Ctrl-D` (indent, dedent),
  `Ctrl-O` (one Normal command), `Ctrl-R {register}`, `Ctrl-H`.
- **Not yet**: folds, tab pages, the `=`, `gq` and `gw` operators, `U`,
  search offsets (`/pat/e`). `=`, `gq`, `gw` and `zf` still take their
  motion, so typing `gqip` never runs `ip` as commands; they then say they
  are not supported.

## The command line

`:` takes Vim's ranges and Vim's names. A range is line numbers, `.`, `$`,
`%`, marks (`'a`, and `'<,'>` — which `:` typed from Visual mode fills in),
`/pattern/` and `?pattern?`, each with `+N` / `-N` offsets, joined by `,` or
`;`. A reversed range or one past the last line is refused with a message.
Commands take their Vim abbreviations (`:s`, `:g`, `:norm`, `:d`, `:m`,
`:t`, `:bn`, `:wqa` …) and `!` where Vim does.

- **Lines**: `:d [x] [count]`, `:y [x] [count]`, `:m {address}`,
  `:t {address}` (`:co`), `:j[!]`, `:pu[!] [x]`, `:>` / `:<` (`:>>` twice),
  `:sort[!] [u][n][i]`, and `:{range}!cmd` to filter lines through a shell
  command.
- **Many lines at once**: `:g/pattern/cmd` runs a command on every matching
  line (`:g!` and `:v` on the others); `:[range]norm keys` types Normal-mode
  keys on each line. Each is one undo step.
- **Substitute**: see [search.md](search.md#substitute-s).
- **Buffers**: `:b N` or `:b name` (a fragment that matches one buffer),
  `:bn`, `:bp`, `:bd [N]`; `:ls`, `:reg [names]` and `:marks` open their
  listing as a read-only buffer.
- **Files**: `:w`, `:up` (only if modified), `:wa` (every modified file; it
  says how many, and names the count of modified buffers that have no file
  name), `:wq`, `:x`, `:wqa` / `:xa`, `:e`, `:enew`, `:new`, `:vnew`,
  `:close`, `:only`.
- `:@:` repeats the last command line.

Not there: `|` between commands, `:g` without a command (print), the `c`
flag of `:s` inside `:g` or `:norm`, `:set` beyond `ro`, `:r file`, and
`:sort` by pattern or by hex / octal / float.

## Registers

The same registers work everywhere you type: buffers, the `:` and `/` lines,
and every picker field. `y` fills `"0`; a delete or change
of a line or more shifts `"1`–`"9`, a smaller one fills `"-`; `"a`–`"z` are
named (`"A` appends), `"_` discards, `". ": "/ "% "#` are the last insert, last
`:` line, last search, file name and alternate file name. `p`/`P` read them in
Normal mode, and
`Ctrl-R {register}` inserts one in Insert mode, on the `:` and `/` lines, and
in every picker field — Files, Search's Find and With, symbols, buffers.
`Ctrl-R "` is the last yank or delete. `y`, `d` and `c` in a field's Normal
mode write the registers as they do in a document.

In Visual mode `p` puts the register over the selection and the text it
replaced becomes the unnamed register, so the two are swapped; `P` does the
same put and leaves every register as it was, so it can be repeated on the
next selection.

A plain yank is also a copy: `y` (with no register) puts the text on the
system clipboard as well, so the terminal's own paste (Ctrl-Shift-V, or Ctrl-V
in Windows Terminal) and other applications get what you yanked. Deletes and
changes (`dd`, `x`, `cw`) and yanks into a named register do not touch the
clipboard, so they never overwrite something you copied elsewhere. `"+` (and
`"*`) is the clipboard by name: `"+y` / `Space y` copy to it, `"+p` /
`Space p` / `Ctrl-R +` read it. `p` on its own reads the unnamed register, not
the clipboard. A yank over 1 MB stays in strop and says so; `"+y` sends it
anyway.

## Windows

`Ctrl-W` is Vim's window prefix: `h j k l` move, `w` / `W` next and previous,
`p` the last used window, `s` / `v` split below and right, `n` a new empty
buffer, `c` close, `q` quit the window, `o` keep only this one, `=` equal
sizes. Holding Ctrl for the second key works too (`Ctrl-W Ctrl-V`). A key
`Ctrl-W` does not bind is reported — `ctrl-w x is not bound` — and does
nothing else. `Space w` remains, with `1`–`9` to focus a pane by number.

## Code completion

In Insert mode strop suggests completions from the language server and
words from the current buffer. Words work without a language server. Each
row shows the kind (function, struct, …), where it came from (`lsp` or
`buf`) and a short detail. No icon font is needed. A selected
language-server item can show its documentation without changing the
buffer.

- `Ctrl-Space`: ask for both kinds; `Ctrl-X Ctrl-O`: the language server
  only.
- `Ctrl-N` / `Ctrl-P`: choose a candidate; `Ctrl-Y`: accept it.
- `Tab` / `Shift-Tab` step through the candidates and show each one in the
  text as you go. With the menu closed they indent and dedent.
- Enter accepts only after you have chosen a candidate; otherwise it starts
  a new line as usual.
- `Ctrl-E`: close the menu without changing the text. Escape closes it and
  leaves Insert mode.

When a candidate also adds an import, both edits are made together and one
`u` undoes both. If the text changed so that the server's edit no longer
fits, nothing is inserted. A candidate that needs a snippet format or a
server command strop does not support is marked as unsupported in the menu,
and accepting it inserts nothing. Completion does not open in a read-only
buffer, or in a `strop -` buffer that is still reading its input.

```toml
[completion]
enabled = true
auto_popup = true
```

With `auto_popup = false` the menu opens only when you ask (`Ctrl-Space`),
and then updates as you type. `enabled = false` turns completion off, the
keys included.

## Syntax and display

Syntax highlighting covers 26 languages (including CMake, Markdown, TOML,
YAML, HTML and CSS), all built into the binary. Code blocks inside Markdown
are highlighted in their own language, for any of the 26. The same goes for
code examples in documentation comments: Rust doc comments, JS/TS/TSX,
C/C++ and Java doc blocks, Go block comments above a declaration, and
Python docstrings. A code block you forgot to close ends with its comment.
In Rust doc comments a block with no language is treated as Rust. HTML
highlights the JavaScript and CSS inside it.

Highlighting, indent guides and the search match count are worked out in
the background. They are never shown for an older version of the text.
Very long lines stay fast to scroll sideways and to type in. Tabs and wide
characters line up the same way for the cursor, block selection and
sideways scrolling. A file keeps its line endings (LF or CRLF) when you
edit it. In a narrow terminal the status line still shows the file name,
the messages and the position.

## Reading stdin

`strop -` edits standard input: `cmd | strop -` opens the output in a buffer
named `stdin`. The buffer streams — `tail -f app.log | strop -` keeps
appending while you read, the cursor on the last line follows new output,
and it stays read-only until the pipe closes. Then it is an ordinary scratch
buffer: edit it, `:w path` saves it. Keys come from the terminal, not stdin.

## Sessions

strop remembers each project. Start it again in the same directory with no
file argument (`strop`) and the files you had open are back, the current
one at its cursor and scroll position, with undo history and the jump
list. Undo history comes back only for a file that has not changed on disk
since. Splits, marks and registers are not kept. `strop FILE` opens just
that file. The session file is readable only by you, and file names that
are not valid UTF-8 survive.

`:qa` saves the session for the whole project before it closes anything.

Each pane has its own jump list of 100 entries; a split starts with a copy.
`Ctrl-O` and `Ctrl-I` go back and forward through it, and put the cursor,
the scroll position and any extra cursors back as they were. Jumping
somewhere new from the middle of the list does not throw away the newer
entries. `Space u` opens the undo history as a tree: Enter goes to that
state of the text, `q` closes.

Files on SSH hosts, terminals and buffers with no file are not reopened
from a saved session. Connect again and open them.

## Recovering unsaved work

While you edit, strop keeps a copy of the unsaved text of each modified
file (under `$XDG_STATE_HOME/strop`, normally `~/.local/state/strop`), so
a crash or a closed terminal does not lose it. Saving the file removes the
copy. Unsaved text over 16 MiB is not kept, and strop says so.

When you open a file that has such a copy, a card offers:

| Key | Does |
| --- | --- |
| `r` | put the unsaved text in the buffer; `:w` saves it, `u` returns to the file on disk |
| `o` | open the file read-only and keep the copy |
| `d` | delete the copy |
| `e` or `Esc` | edit the file as it is on disk and keep the copy |

`:recover` lists every copy strop holds, numbered, including those of
buffers that never had a file name. `:recover restore N` opens copy N in a
new buffer without touching the file; `:recover discard N` deletes it.
Unsaved text of files on SSH hosts is kept only after
`:recover consent remote`, for the current run; `:recover consent off`
stops it.
