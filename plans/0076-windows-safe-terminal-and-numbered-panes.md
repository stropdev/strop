# 0076 — Windows-safe terminal and numbered pane navigation

## Contract

Terminal input reserves **F2** as the direct, single-key transition to the pinned
Normal/Visual inspection buffer. `Ctrl-\ Ctrl-N` remains an alternative for
frontends that deliver it, but is no longer the only or primary hint. F2 does
not change document-mode LSP rename and does not forward an inspection request
to the child.

`Space w` owns pane commands in Normal mode:

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
