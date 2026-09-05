# 0016 — One input machine (0.7)

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
