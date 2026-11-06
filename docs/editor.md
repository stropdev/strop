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

## Themes

`:theme` opens the searchable palette selector; Enter applies a choice and
Escape leaves the current theme unchanged (once enters the picker's Normal
mode; twice closes it). `:theme catppuccin-mocha` switches directly. Rows show
human labels, palette swatches and a check on the applied theme; the selected
row's canonical name and full description appear in a bounded detail area.
Small terminals omit decoration before hiding identity or input.

| Name | Character |
| --- | --- |
| `dark` | Strop's original dark palette and amber accent; the default |
| `light` | Strop's warm paper-and-ink counterpart |
| `catppuccin-mocha` | Warm pastel dark, with violet and blue syntax |
| `catppuccin-latte` | Light Catppuccin, adapted for readable editor contrasts |
| `nord` | Restrained cool blue-grey, with brighter comments for legibility |
| `strop-ink` | Original low-chroma graphite, ivory and restrained brass |
| `paper` | Warm off-white paper, deep green ink and forest-green accents |
| `paper-dark` | Warm green-black paper, soft ivory ink and muted sage accents |

Choose at launch with `strop --theme paper file.rs` or
`strop --theme paper-dark file.rs`, or persist either choice in
`$XDG_CONFIG_HOME/strop/config.toml` (normally
`~/.config/strop/config.toml`):

```toml
theme = "paper"
# Or: theme = "paper-dark"
```

The same canonical names work for a session-only switch: `:theme paper` or
`:theme paper-dark`.

Precedence is the built-in default, then the user config, then `--theme`.
`:theme` changes the running session only; it does not rewrite the config.
Syntax, pickers/previews, diffs, diagnostics, help and statusline share the
palette. Existing terminal sessions update their default colors without
overwriting child-selected truecolor or OSC palette customizations.

Adaptations follow the [Catppuccin style guide] and [Nord color roles];
they are not promises of exact upstream editor-port fidelity.
`strop licenses` prints the bundled theme licenses. All palettes are
compiled into the binary: no downloads, plugins or setup.

[catppuccin style guide]: https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md
[nord color roles]: https://www.nordtheme.com/docs/colors-and-palettes

## Panes, previews and language services

Local pane titles use workspace-relative paths; duplicate directories,
outside-workspace paths and remote/container authority remain identifiable.
Pane numbers, focus, dirty and read-only markers survive narrow layouts.
File, search and symbol previews automatically reclaim unused result-list
width from short admitted labels, within bounded stable layout limits. Long
paths keep their list budget; narrow terminals retain stacked or omitted
preview fallbacks.

The modeline presents owned LSP starting, ready, refused and failed states as
compact badges. Long server prose does not displace mode and position. Use
`:explain` to inspect the complete server, workspace/endpoint and failure
reason in a real read-only buffer; recovered servers clear their old failure
without erasing another workspace's problem or an independent warning. If a
server exits during source navigation, the next language request names that
failure and starts one scoped reattach instead of claiming the installed
server is missing; repeat the request after it is ready.

Failed or unconfirmed saves, connection/admission failures, terminal decisions
and Dev Container outcomes use the same owner-scoped retained feedback. They
survive motions, search and unrelated success messages. **Space a** exposes
only currently valid recovery actions; **`:explain`** retains the complete
owner, detail and evidence. A compact textual severity marker remains in the
modeline even at ultra-narrow widths; there are no floating notification cards
to dismiss or fade. Dismissing a notice changes only presentation—it never
cancels work, clears a save barrier or marks an unconfirmed outcome as
successful.

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

Tree-sitter highlighting for 23 languages (including CMake, Markdown,
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
