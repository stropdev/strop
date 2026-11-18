# 0028 — Roadmap

- Status: **live** — the one list of deferred work and over-ceiling files
  (AGENTS.md "File size discipline"). An item leaves this list in the change
  that lands it, or moves to a numbered plan when it needs a design.
- History: until 0.71 this file held the 0.14–0.18 editor review and its
  release sequence (P1/P2 findings, the GUI arc, the 0057 assurance
  sequence). All of it landed or was retired (see `STATUS.md`); the text is
  in git history before plan 0108.

## Deferred from 0.71 (plans 0108–0111)

Correctness and data:

- **In-place write for hard links and read-only parents** (0108 D1): a
  save goes through a staged file and a rename.
- **Macro replay speed** (0108 H5): replays run at about 1,600 keys/s on
  files over 4 KiB; `Ctrl-C` interrupts them, the speed is unchanged.
- **`operated_span`, viewport clamps, mark fold** (0108 D4). The remaining
  hand-rolled span/point arithmetic that does not yet go through
  `strop_kernel::editmap`.
- **`c2iw` / `c2aw` at the end of the buffer** diverge from Neovim.
- **Word motions over combining marks** diverge from Neovim.
- **`|` counts bytes, not display columns** on lines with multibyte text.

Highlighting (0111):

- **Kernel K3 and K4**: injection range algebra, then layering
  (`spans::flatten`), as theorems in `strop-kernel`.
- **Syntax point model (L2)**: points handed to tree-sitter use ropey's line
  model; settle it with the line-model decision.
- **Huge Markdown fences reparse slowly**: typing inside a 2,000-line fence
  is bounded by the `tree-sitter-md` block grammar's incremental reparse.

Filters and the command line (0109, 0110):

- **`severity:`** filter for diagnostics lists.
- **Command-line window height** (0110).
- **Multi-line `:s` preview** (0110).

The help page (0112 §13):

- **Which-key, the welcome card and footer hints take the help's key
  inks**; they use `card::keycap`, one ink. Do it once the help has been
  lived in.
- **`gO`** (Neovim's outline) for help.
- **`Ctrl-]`** to follow the word under the cursor; strop does not decode
  the key.
- **The `ranges` row of the Command line part** (`:N :N,M :. :$ …`) inks
  every range as a command name; a ranges lexer is not worth its lines yet.

Clutter left by the 0.71 sweep (0108 §C):

- Six `#[allow(clippy::too_many_arguments)]` sites want a params struct each
  (`render/{picker/rows/search, buffer/chrome, buffer/rows/collection,
  picker/preview}.rs`, `editor/{git_memory/jobs, feedback/mod}.rs`).
- Hand-rolled test PRNGs (xorshift and LCG) in about a dozen test modules
  across five crates. One shared helper needs a cross-crate test-support
  module and changes every seeded stream; not done.
- Comments citing retired plans (0055, 0057, 0058) and ticket tags (`VF11`,
  `WK04`, `R12`, …) should say what the code does.
- crossterm 0.29 + ratatui 0.30 together clear three duplicate crates.

## Over-ceiling files

None. The largest source files sit at 760–790 lines
(`editor/resolution/mod.rs`, `strop-trace/src/tests.rs`,
`strop-remote/src/pool.rs`); split them by concern in the next change that
grows them.
