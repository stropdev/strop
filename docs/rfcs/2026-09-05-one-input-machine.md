---
rfc: "2026-09-05-one-input-machine"
title: "One semantic input machine"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-05"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/112"
milestone: "v0.7.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0016"
---

# RFC: One semantic input machine

v0.7.0 derived prefixes and aliases from the command table, routed keys to typed actions instead of assembled command strings, and added macro recording/replay and a generated compatibility report ([3422ad1](https://github.com/stropdev/strop/commit/3422ad17632b67c589fbb4c41cc946f3c530d6c6)). The report is now strop compat; 0095 extends the same grammar rather than replacing it.

[Tracking issue #112](https://github.com/stropdev/strop/issues/112) · [Published v0.7.0](https://github.com/stropdev/strop/releases/tag/v0.7.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0016-one-input-machine.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: accepted (2026-09-05 review). The 0.6 walker types the parser
state but still returns assembled key strings, and not every key
traverses it. This plan finishes the job.

## Scope

- The walker emits `ParsedCommand { id: CommandId, count, register,
  arguments }` — never a `String`.
- Every normal-mode key event traverses the one machine: chars, arrows
  (0.6.1), Ctrl-keys, Enter/Tab, mouse-derived motions, macro replay,
  dot repeat, test scripts.
- Aliases map to semantic commands (`D` = `DeleteToLineEnd`), not
  textual replay (0.6.1's count-preserving replay is the bridge).
- Macros: `q<reg>` records ParsedCommands, `@<reg>` replays. Readonly
  surfaces keep their contextual layer (vim's special-buffer
  precedent); macros work everywhere normal mode exists.
- Dot repeat becomes a recorded `RepeatRecipe` committed only after a
  plan executes successfully.
- `?` help and the compatibility report generate from the same table.

## Non-goals

- A literal trie/DFA data structure is an implementation detail; a
  table-driven state machine is equally acceptable.
- Keymap user configuration (0005 successor) — separate plan.

## Tests

Model-based transition testing: generate sequences from the registry,
assert Esc returns to ground, completed commands clear structural
state, counts never overflow, invalid transitions never leak. The
0.6.1 battery (d0, arrows+counts, alias counts) is the seed corpus.
