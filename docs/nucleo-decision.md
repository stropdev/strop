# Nucleo decision record (0022 §2)

Date: 2026-09-06. Decision: **adopt nucleo-matcher** for the picker's
row scoring.

## The measurement

`cargo run -p strop-picker --example scorebench --release` —
full refilter of a repo-shaped path corpus, six queries, wall time:

| items | custom scorer | nucleo-matcher |
| ----: | ------------: | -------------: |
| 10k  |  4.7ms |  1.3ms |
| 50k  | 23.8ms |  6.1ms |
| 100k | 42.0–49.6ms | 11.9–12.3ms |

Hit sets identical on every run (9075 / 45375 / 90750). ~3.5–4×
faster at every scale; at 100k items the per-keystroke refilter drops
from a janky ~50ms to ~12ms.

## The constraint that decided it

The picker's accent render needs matched-char columns; nucleo's
`Pattern::indices` supplies them (verified: correct 0-based char
positions, boundary-heavy optimal path). Scoring parity held on the
picker's pinned test set; one pin re-pointed from the old scorer's
earliest-match preference to nucleo's boundary-preferred columns —
strictly better for path highlighting.

## What stayed the same

`fuzzy_score`'s signature and the picker's refilter logic. The scorer
is a thread-local shared Matcher (UI is single-threaded); the custom
60-line implementation is deleted, not shimmed.
