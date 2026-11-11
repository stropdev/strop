# Editor surfaces

Themes, completion, indentation and the visual system — the parts of strop
you configure rather than drive. For the input grammar see the README's
card; for pickers see [search.md](search.md).

## Indentation

Enter between `{}`, `[]` or `()` opens an indented inner line and leaves
the closing delimiter aligned. Typing a closer on an indentation-only line
aligns it with its matching opener, even when the current indentation is
wrong. Each cursor uses its own source context, indentation unit and line
endings. Undo, counted insertion and dot repeat preserve the operation
rather than copying generated indentation from a different location.

In C and C++, typing `:` uses vim's electric-label indentation. Bare access
specifiers (`private:`, `protected:`, `public:`) and `case`/`default:` labels
snap to one unit inside their enclosing brace; named labels snap to column
zero. C-mapped buffers (`.h`, `.c`) behave like C++, since headers detect as
C. The second colon in a C++ scope operator (`std::`) restores statement
indentation after the first colon's possible-label interpretation. Other
languages and non-label punctuation insert the colon unchanged.

A trailing colon opens a one-unit deeper body where the language gives it
that meaning — one punctuation rule decided by the language registry, not
per-language editor code:

- **Python**: a block colon (`if ready:`) deepens the next line; a colon
  inside brackets (a slice or annotation) does not.
- **C, C++, Java, C#, Go, JavaScript/TypeScript, PHP**: the label colon
  deepens its body — access specifiers (`private:`, `public:`), `case` and
  `default:` arms, and named labels. In a C++ class, Enter after
  `private:` puts the members one level inside the section.
- **Rust, JSON, CSS and prose**: a trailing colon never deepens.

Structural delimiter support covers Rust, Python, JavaScript, TypeScript,
Go, C, C++, JSON, Java, C# and CSS. Strings and comments are excluded.
Named prose/unsupported files inherit whitespace; unnamed scratch buffers
retain generic brace behavior. This is not a formatter, HTML tag
indentation, or keyword-based `end`/`else` alignment. Large-file analysis
runs on the existing revision-checked worker, never the render path.

## The look

Every surface reads its colours from one token set: three background
planes (`base` < `surface` < `raised`), four greys for text, one accent,
mode colours, diagnostic and diff inks, syntax roles. The accent means one
thing — where you are and what will happen: the Normal chip, the current
line number, the pending-operator footer, the input caret, matched
characters. Cards (pickers, which-key, completion, hover, blame, command)
share one primitive: rounded, title top-left, hints as keycaps in the
bottom border. Selected rows are a raised band with `▌`; the applied theme
and dirty buffers are marked `●`. Chrome appears only when it carries
information: the scrollbar is a thumb that disappears when the file fits,
pane titles sit in the pane's top border only with two or more panes, the
`LSP!` badge appears only when a language server failed, `Spaces:N` only
when a buffer's indent differs from your configured default, and the
picker's scope row only when a scope differs from the default.

Messages have their own echo row above the statusline, present only while
something is live: the response to your last key first, then retained
feedback by severity (`✗` failure, `▲` warning, `●` info) with its `space a`
or `:explain` hint. The statusline itself is a fixed 8-cell mode chip,
branch, path, then — only when non-zero — `●1 ▲2` diagnostics, the service
badge, the indent, and the position.

## Themes

`:theme` opens the searchable palette selector; Enter applies a choice and
Escape leaves the current theme unchanged (once enters the picker's Normal
mode; twice closes it). `:theme vellum` switches directly. Rows show the
human label, six swatches (base, surface, text, accent, keyword, string)
and `●` on the applied theme; the selected row's canonical name and full
description appear in a bounded detail area. Small terminals omit
decoration before hiding identity or input.

Six original palettes ship, every one holding the same contrast contract:
text ≥ 10:1, secondary ≥ 7:1, comments and every syntax ink ≥ 4.5:1 on both
the base and the surface plane, a visible Visual selection.

| Name | Character |
| --- | --- |
| `hone` | Graphite dark with the amber accent; the default |
| `slate` | Low-chroma dark: syntax by luminance, one brass accent |
| `carbide` | High-contrast dark for projectors and bright rooms (every ink ≥ 7:1) |
| `vellum` | Warm paper, graphite ink, burnt-umber accent |
| `paper` | Warm off-white paper with deep green ink and forest accents |
| `paper-dark` | Warm green-black paper with soft ivory and sage accents |

Choose at launch with `strop --theme vellum file.rs`, or persist the choice
in `$XDG_CONFIG_HOME/strop/config.toml` (normally
`~/.config/strop/config.toml`):

```toml
theme = "vellum"
```

The same canonical names work for a session-only switch (`:theme slate`).
Precedence is the built-in default, then the user config, then `--theme`.
`:theme` changes the running session only; it does not rewrite the config.
A config naming an unknown theme (including the retired `dark`, `light`,
`strop-ink`, `catppuccin-*` and `nord`) starts on `hone` with a warning
that lists the six names. Syntax, pickers/previews, diffs, diagnostics,
help and statusline share the palette. Existing terminal sessions update
their default colors without overwriting child-selected truecolor or OSC
palette customizations. All palettes are original and compiled into the
binary: no downloads, plugins, licences or setup.

## Panes, previews and language services

Local pane titles use workspace-relative paths; duplicate directories,
outside-workspace paths and remote authority remain identifiable.
Pane numbers, focus, dirty and read-only markers survive narrow layouts.
File, search and symbol previews automatically reclaim unused result-list
width from short admitted labels, within bounded stable layout limits. Long
paths keep their list budget; narrow terminals retain stacked or omitted
preview fallbacks.

The modeline presents owned LSP starting, ready, refused and failed states as
compact badges. Long server prose does not displace mode and position. Use
`:explain` to inspect the complete server, workspace/endpoint and failure
reason in a real read-only buffer; recovered servers clear their old failure
without erasing another workspace's problem or an independent warning. A
server that crashes after it was ready restarts by itself, up to three times
per workspace: the status line says `restarting (1/3)`, the `LSP!` badge stays
until the replacement is ready, and every file the old server had open —
library files included — reopens on the new one. After the third restart, or
for a server that never started, the next language request names the failure
and starts one scoped reattach; repeat the request after it is ready.

## The project root

The project is the directory strop was launched in, and it stays there:
`:pwd` shows it. Files (`Space f`), Search (`Space /`) and new terminals are
scoped to it whichever file has focus. Following go-to-definition into the
Rust standard library, a cargo registry crate or a system header does not
re-root anything — that file opens read-only, and the project's language
server keeps serving it (a library file belongs to the project that depends
on it; no second server is started in the library's directory). Browsing a
directory deliberately (`:browse DIR`) scopes Files and Search to that
directory while it has focus.

## Registers

One register file serves every text input. `y` fills `"0`; a delete or change
of a line or more shifts `"1`–`"9`, a smaller one fills `"-`; `"a`–`"z` are
named (`"A` appends), `"_` discards, `". ": "/ "%` are the last insert, last
`:` line, last search and file name. `p`/`P` read them in Normal mode, and
`Ctrl-R {register}` inserts one in Insert mode, on the `:` and `/` lines, and
in every picker field — Files, Search's Find and With, symbols, buffers.
`Ctrl-R "` is the last yank or delete.

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

Insert mode offers language-server candidates and incremental words from the
current editable source. Word suggestions remain usable without a language
server. Read-only source collections do not open completion or gain edit
server. The menu shows kind, source and detail. Kind chips use the symbol
picker's quiet family colors, and the ASCII `lsp` / `buf` provenance tags stay
color-coded without requiring an icon font; selected language items can show
documentation without changing the buffer.

- `Ctrl-Space`: request both providers; `Ctrl-X Ctrl-O`: language completion.
- `Ctrl-N` / `Ctrl-P`: choose a candidate; `Ctrl-Y`: accept it.
- Enter and Tab accept only after deliberate selection; otherwise they keep
  their ordinary Insert behavior.
- `Ctrl-E`: dismiss without editing. Escape dismisses and leaves Insert mode.

Primary edits and represented imports are validated together and share the
Insert undo group. Stale, overlapping or invalid edits are refused as a
whole; unsupported snippets and commands are not approximated by a label
insertion. Read-only and partial sources do not gain write authority
through completion.

```toml
[completion]
enabled = true
auto_popup = true
```

Set `auto_popup = false` for manual-only completion; an opened manual
session still follows typing. `enabled = false` also disables manual
invocation and retires completion-only work.

## Syntax and display

Tree-sitter highlighting for 26 languages (including CMake, Markdown,
TOML, YAML, HTML and CSS); grammars and licensed queries ship in the
binary. Markdown fences select any bundled language — not just Rust — and
the same injection pipeline highlights fenced examples inside Rust
documentation comments, JS/TS/TSX, C/C++ and Java documentation blocks,
declaration-attached Go block comments, and Python's initial
module/class/function docstrings. Comment delimiters remain comments;
independent documentation groups cannot leak unclosed fences into each
other. Only rustdoc defaults unlabelled fences to Rust. HTML embeds
JavaScript and CSS.

Syntax, structural indentation guides and search counts are
revision-owned worker results. Long-line byte/cell checkpoints keep
horizontal navigation and typing from repeatedly walking the line prefix.
Horizontal views, block selection and carets share display-cell geometry;
new edits preserve the buffer's line endings. The modeline keeps filenames,
live status and position legible at narrow widths.

## Reading stdin

`strop -` edits standard input: `cmd | strop -` opens the output in a buffer
named `stdin`. The buffer streams — `tail -f app.log | strop -` keeps
appending while you read, the cursor on the last line follows new output,
and it stays read-only until the pipe closes. Then it is an ordinary scratch
buffer: edit it, `:w path` saves it. Keys come from the terminal, not stdin.

## Sessions

File/native work runs on owned jobs rather than blocking keystrokes.
Sessions use private atomic files and lossless native paths, including undo
history and the active window's jump history. Per-project sessions restore cursor,
viewport and jump positions across restarts; retained jump records also restore
their selections. Each pane keeps an independent 100-entry jumplist, copied on
split. Quit-all writes one complete project snapshot before closing any buffer.
`Ctrl-O` / `Ctrl-I` traverse jump origins without discarding newer entries
when a new jump branches from history. Undo history is available in the `Space u`
tree browser. Remote contents and write permits are never restored from persisted
sessions.
