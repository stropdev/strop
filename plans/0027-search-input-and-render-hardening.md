# 0027 — Search and terminal correctness (#13, #14)

Status: implemented for 0.14.0; gate/release tracked separately.
This replaces the interrupted draft's incorrect conclusions about both issues.

## Findings and changes

### #13 — search edits must recompute the same command

Typing, Backspace and modal line edits now recompute live search from its fixed
origin using `strop_grammar::resolve` and `cursor_after`. Enter resolves from that
same origin: it cannot skip the match already shown. Abort restores the origin.
Backspace on the bare `/` or `?` closes the prompt (the previous draft incorrectly
claimed Vim forbids deleting the sigil). The shared `LineEdit` caret mechanics
handle deletion/insertion inside the prompt, including UTF-8 text.

`?` gets the same prompt card and match computation as `/`. Plain searches no
longer receive operator-preview coloring. Find-candidate coloring reads the
walker's actual `f/F/t/T` state, not the final letter of a search or shell command.
Synthetic clipboard-yank compositions remain supported and covered.

Operator-search previews are completed temporarily with Enter, then use the
same grammar plan as execution, including count/register and all cursors.
`Resolved::motion_target` separates the landing position from the sorted affected
range, so wrapping searches remain well-defined. Literal matching streams rope
chunks rather than copying the entire document on every search update.

### #14 — terminal control bytes, not stale ratatui buffers

The actual path is CRLF → `Buffer::line_end` retaining CR → content span containing
raw CR → ratatui counting one cell → Crossterm printing cursor movement. Adjacent
padding then writes at the wrong physical column. TestBackend cannot observe the
cursor movement; earlier tests that repeatedly created fresh terminals were not
proof of the reported bug being absent.

`line_end` excludes CRLF without rewriting file bytes. The layout and renderer
share `printable_grapheme`: control graphemes render as a one-cell replacement,
never protocol bytes. The completed frame also enforces the printable-cell
invariant for metadata, diagnostics and popup content. Raw ESC cannot become an
OSC/CSI command. Legacy LSP `eprintln!` paths are removed in the tracing cutover.

A kept regression sends real Crossterm diff bytes into a VT terminal emulator,
compares the physical screen with the model, then shortens/deletes CRLF lines on
the same terminal. Another covers escape-bearing text and metadata. Tiny popup
geometry uses bounded dimensions rather than invalid `clamp(min > max)` calls;
resize/restore is covered. Ctrl-L is a user-requested repaint recovery, **not**
the fix for #14 and not an automatic clear-every-frame workaround.

## Quality and adjacent safety

- Split LSP client into `client/{mod,spawn,api,wire,trace_io}.rs`.
- Split keymap query logic into `keymap/lookup.rs`; the remaining large file is
  the one command-table listing, not mixed command logic.
- Split buffer renderer tests from rendering; new CLI, terminal and trace
  responsibilities live in separate modules.
- Restore accidentally omitted smart-indent test registration and script tokens
  `<space>` / `<c-o>`. Literal angle-bracket replay uses explicit key records.
- `FindPending` and `BlockRect` replace ambiguous mixed-unit tuples. Full numeric
  domain conversion is **not** claimed complete; the remaining work is in 0028.
- Buffer saves use exclusive private same-directory staging, permissions before
  data, propagated errors and no-clobber persistence for save-as without force.
- Updater uses private unpredictable staging for downloads and installation.
- Delayed shell replacements reject invalid UTF-8 offsets before slicing and
  enter the transaction gateway, so anchors/history/syntax do not bypass it.
- Git diff snapshots clone the rope on the input path; string conversion occurs
  on the worker, matching the preexisting comment's intended contract.

## Evidence

Targeted search and physical-terminal suites: 16 passing tests after the final
search changes. Actual TUI smoke exercised `/needle`, two Backspaces, Enter,
insert/Esc and `:q!` on a CRLF fixture; exit 0 and trace recorded the expected
edited first row, clean filler rows, cursor and successful session end.

All broader deferred findings and priority are in 0028. Proposed prevention
architecture is 0030, planning only, not an executed redesign.
