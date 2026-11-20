---
rfc: "2026-10-28-windows-safe-terminal-and-numbered-panes"
title: "Numbered pane navigation and terminal inspection key"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-10-28"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/171"
milestone: "v0.46.3"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0076"
---

# RFC: Numbered pane navigation and terminal inspection key

v0.46.3 shipped Space w split/focus bindings and matching pane numbers, together with F2 to inspect a terminal from Windows Terminal. 0090 removed F2 and pinned inspection; numbered Space w navigation remains alongside Vim’s Ctrl-W window commands (docs/editor.md).

[Tracking issue #171](https://github.com/stropdev/strop/issues/171) · [Published v0.46.3](https://github.com/stropdev/strop/releases/tag/v0.46.3) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0076-windows-safe-terminal-and-numbered-panes.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

## Contract

Terminal input reserves **F2** as the direct, single-key transition to the pinned
Normal/Visual inspection buffer. `Ctrl-\ Ctrl-N` remains an alternative for
frontends that deliver it, but is no longer the only or primary hint. F2 does
not change document-mode LSP rename and does not forward an inspection request
to the child.

`Space w` owns pane commands in Normal mode:

**0081 amendment:** F2 inspection and `i` input are the primary short cues at
48 columns and tiny fallbacks. Namespace/launch identity is retained beside
them; legacy Ctrl-4 is explanatory decoder history, not another mode concept.

- `-` horizontal split and `v` vertical split;
- `1` through `9` focus the numbered pane;
- `h/j/k/l/w` retain directional/cyclic movement and `q` closes the pane.

Multi-pane title rows display the same one-based numbers. Old Normal `Ctrl-W`
bindings are removed by changing their single keymap-table entries, not kept as
a parallel dispatch path. The terminal-owned `Ctrl-W` prefix is removed too:
Ctrl-W is child input, while F2 followed by `Space w` provides editor pane
control. Existing `:vs` and `:sp` Ex commands remain.

`_` implements Vim's first-nonblank motion, including `2_` moving to the next
line's first nonblank. This is separate from `0` (column start) and `^` (same
line, first nonblank).

## Verification and release

Real-input tests cover F2 inspection and Ctrl-W reaching the child. Pane tests
cover split direction, numbered focus, visible numbers, close, and retired
Ctrl-W dispatch. Grammar tests cover bare and counted `_`, including indented
and empty lines. Help and which-key derive from the changed keymap table. The
full Docker gate and real headless journeys run before tagging 0.46.3; the
release workflow publishes artifacts and the website changelog.
