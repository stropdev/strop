# 0032 — Modeline and Git surface polish

Status: implemented for 0.15.1, after the published 0.15.0 correctness release.
The user requested modeline polish inspired by oh-my-pi, plus commit-viewer and
related presentation improvements. No grammar, keybinding or buffer-model redesign.

## Observed problems

- The modeline gives mode, absolute path and branch competing emphasis. It measures
  characters instead of display cells, so long/wide paths can push status and
  position off-screen. Its percentage includes the trailing-newline phantom row.
- Commit subjects compete visually with author/age metadata. Changed-file and
  sidebar hierarchy is hard to scan; sidebar padding uses character counts.
- Sidebar trees and widths are rebuilt for every painted row. Display-string
  reverse lookups can confuse distinct native paths with the same lossy label.
- The real review fixture displayed a Japanese Git path as quoted octal escapes.
  `git --numstat` is parsed as human text rather than its native NUL-delimited form.
- Once native paths were preserved, control-bearing filenames also needed a
  printable one-row label and literal libgit2 path matching. Display labels cannot
  be used as the current-file identity.
- Successful Git dives left a stale loading message. Headless text dumps exposed
  covered cells beneath wide glyphs, even though the canonical frame was correct.

The visual reference is [oh-my-pi's actual footer screenshot](https://github.com/can1357/oh-my-pi/blob/main/assets/lspv.webp):
quiet separators, distinct information groups, restrained context color and clear
primary content. Borrow the hierarchy, not its font-dependent icons or agent UI.

## Decisions before implementation

1. Keep strop's dark/amber palette and one-row modeline. Use a quiet fixed background,
   a compact mode accent, a prominent filename and muted directory context. Preserve
   dirty/read-only/multicursor/diagnostic signals and the existing preview/message
   precedence. Adapt by display-cell width; essential live status and position must
   not disappear behind a long path. A last content line reports 100 percent.
2. Keep all Git surfaces real buffers with unchanged text, motions, search and yank.
   Emphasize commit subjects, quiet metadata, and make the current log/file row clear.
   Improve diff bands and file hierarchy without adding modal controls or fake rows.
3. Build sidebar structure once per pane render, not once per row. Keep native path
   identity separate from display labels; clip labels at grapheme/cell boundaries.
   Existing caret/inset geometry must consume the same width as sidebar emission.
4. Fix Git pathname transport at its source: NUL-delimited numstat, native path bytes,
   explicit rename handling and real failures. Do not cosmetically hide quoted names.
5. Share one small printable-cell clipping helper for chrome, reusing the core's
   printable-grapheme policy. No theme engine, Nerd Font requirement, dependency,
   filesystem work on input/render or speculative configuration surface.

## Ownership and interfaces

Main owns shared `render/text.rs`, the render-root integration, visual acceptance,
release notes/versioning and all final validation. Statusline work owns its new
component; Git presentation owns `render/diff*` and its buffer-render integration;
Git pathname parsing owns `strop-git`'s native metadata boundary.

Shared chrome helpers: `width(&str) -> usize`, `clip_end(&str, cells) -> Cow<str>`
and `clip_start(&str, cells) -> Cow<str>`. Widths are display cells, controls use
`strop_core::layout::printable_grapheme`, and truncation never splits a grapheme.
The statusline entrypoint is `render(editor, frame, area)` in its own module.
Public Git data shapes remain unchanged; this is a patch-level presentation/fix pass.

All concurrent owners skip formatting, builds, lint and tests. Main runs these once
after integration, then the Docker gate and hosted checks before landing the polish.

## Acceptance

- Actual terminal captures at ordinary and narrow widths: normal/insert/visual,
  long Unicode paths, modified/read-only files, transient activity and Git surfaces.
- Cell-grid regressions for meaningful layout boundaries, not message wording.
- Commit log, changed files and diff/sidebar remain cursor/text aligned, including
  Unicode/combining filenames and focused/unfocused sidebar states.
- Git path regressions use an isolated repository with quoted/Unicode/native-byte
  names and renames; no sleeps, real HOME, network or source-text assertions.
- Docker fmt/Clippy/tests, Neovim differential and TLC remain green. The performance
  surface remains responsive; no repeated per-row sidebar rebuild remains.
- Ship separately from immutable 0.15.0, with observed evidence and updated notes.

## Observed verification

- A real Git review fixture exercised a branch/merge log, changed files, diff
  headers, focused/unfocused sidebar, and Japanese filenames. Row text and native
  identity remain separate; control/newline/tab filenames and lossy-alias siblings
  have end-to-end navigation regressions.
- Modeline captures exercised 120, 80, 40 and 16 cells, long Unicode paths,
  modified/read-only state and historical commit identity. Rectangle-boundary tests
  ensure empty or offset areas cannot paint neighboring cells.
- Shared printable text and headless visible-cell projection keep Unicode labels
  whole, with no control bytes or hidden continuation-cell residue.
- A 240-file commit, 120×32 cells, 12 traced debug frames: median 106.925 ms before,
  5.793 ms after; maxima 107.912 and 5.978 ms. Same workstation and fixture; the
  comparison is not a portable latency guarantee.
- The actual 120×40 TUI exercised operator-preview priority, normal/insert/visual
  mode contrast, commit log → file list → delta, Tab-focused sidebar navigation,
  and a Japanese commit-file destination. It exited cleanly; the complete capture
  replayed with the static 0.15.1 binary inside Docker without the original host
  fixture paths.
- Docker fmt/Clippy/tests, Neovim differential and the TLC mutation gate passed.
  The static release benchmark reported input + frame p50 0.81 ms, p95 1.19 ms,
  p99 1.43 ms and maximum 1.95 ms over 500 frames at 120×40.
