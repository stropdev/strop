---
rfc: "2026-10-27-discoverable-search-remote-and-terminal"
title: "Search position, remote discovery and terminal mode cues"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-10-27"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/170"
milestone: "v0.46.2"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0075"
---

# RFC: Search position, remote discovery and terminal mode cues

v0.46.2 delivered search-hit position/total, normalized shifted letters after terminal input ownership, and exposed SSH/container destinations and terminal-mode transitions. 0090 later removed the container picker entry and worker-admission hints, and made Ctrl-\ Ctrl-N the sole escape from terminal input.

[Tracking issue #170](https://github.com/stropdev/strop/issues/170) · [Published v0.46.2](https://github.com/stropdev/strop/releases/tag/v0.46.2) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0075-discoverable-search-remote-and-terminal.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

## Input and search feedback

Frontends may report a shifted letter as its lowercase scalar plus `SHIFT` (the
kitty keyboard protocol does this unless alternate keys are requested). Editor
normalization applies Unicode one-scalar uppercasing after terminal-input
ownership has been decided. Prompt, picker and Normal-mode consumers therefore
receive the same `Key::Char` as legacy frontends that already report `G`.
Control chords retain their existing spelling, and PTY input remains lossless.

The existing asynchronous buffer-search summary owns both the total and the
one-based global ordinal of each retained visible hit. The modeline reads the
summary covering the caret and renders `current/total` beside `line:column`.
Neither navigation nor painting scans the buffer. While the summary is absent
or does not cover the caret, the position remains only `line:column`.

## Remote and container discovery

Research found capable SSH/container backends but disconnected entry points:
SSH had `Space o`, while containers required prior knowledge of `:containers`.
This follows the useful part of VS Code's
[Remote Explorer / Attach to Running Container](https://code.visualstudio.com/docs/devcontainers/attach-container)
journey—one remote entry surface leading to a running-container chooser—without
adding a wizard or another filesystem architecture.

`Space o` is the single "elsewhere" entry: its existing destination picker
offers SSH hosts, add-host, and a pinned Containers action. The Containers
action delegates to the existing asynchronous discovery job and container
picker. Successful read-only remote opens name `:remote edit` and `:remote
worker`; container attaches name `:container-worker`. These are contextual
next steps, not permission changes. Worker verification, explicit consent,
read-only defaults, saved sessions and typed refusals remain unchanged.

## Terminal modes

The integrated terminal stays. Its pinned snapshot is what makes Normal and
Visual motions, search and yank safe while child output continues. Removing
that surface, rebinding Escape, or refreshing beneath a selection would trade
away existing correctness for a superficially simpler chord.

The discoverability contract is:

- Terminal input always advertises `Ctrl-\ Ctrl-N` for Normal and `Ctrl-W` for
  pane commands. A pending prefix displays its complete follow-up grammar.
- Normal inspection accepts the familiar `i`, `I`, `a` and `A` entries into
  live input, matching [Neovim's terminal-mode entry keys](https://neovim.io/doc/user/terminal.html).
  The walker-idle guard remains: `i`/`a` inside another grammatical sequence
  do not leak into terminal ownership.
- Edit commands refused by the readonly snapshot explain how to enter input;
  they do not make the projection writable or relaunch an exited child.
- Status text distinguishes private input from recorded input plus output.
  Escape and all child keys keep their existing physical-event behavior.

No VT, PTY, worker protocol, paste-consent or asynchronous event-lane changes
are part of this UX patch.

## Verification and release

Modifier-aware real-input tests cover uppercase prompt/picker text and `G`.
Worker-backed modeline tests cover `1/4 → n → 2/4 → N → 1/4`. Remote tests
cover the unified picker delegation and first-open hints without changing
permission tests. Terminal tests cover all four input entries, pending-chord
hints, Visual refusal guidance and the existing pinned-selection journeys.
The complete Docker gate and affected real frontend journeys run before the
0.46.2 tag. The existing release workflow publishes the catalog and artifacts,
then dispatches the strop.dev changelog/installer update.
