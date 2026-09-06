# 0025 — The 1.0 perf gate: measure before claiming

Status: landed (0.12.0). Follows 0020's roadmap line: "Perf benchmark
suite (1/10/100MB, 100k lines, 1k cursors, tail latency) — precedes any
perf claims; 1.0 gate item."

## 1. Why

Every perf statement about strop so far is a vibe. The doctrine says the
input→render path never blocks (0001 §3); without numbers that is
marketing, not engineering. The gate has two jobs:

1. **Baseline** — know what 1MB/10MB/100MB files, 100k-line navigation,
   1k cursors, and per-keystroke input-to-frame actually cost.
2. **Regression surface** — a suite that any future "optimization" must
   beat, and that turns "feels slow" bug reports into a reproducible
   scenario.

"No measured pain, no rewrite" (roadmap): the numbers decide what, if
anything, gets optimized. This plan ships the measurement, not a rewrite.

## 2. Shape

`strop --bench [scenario|all]` — a subcommand of the real binary, same
shape as `--headless` (0006). No criterion, no new dependencies, no
benchmark framework: deterministic iteration counts, `Instant` timing,
percentiles computed over recorded samples. Rationale:

- The crate is bin-only (0024 conformance made that choice); a bench
  module in-crate drives `Editor::feed` + the real render path
  (`render::render` onto ratatui's `TestBackend`) — the production path,
  not a harness-only shortcut.
- Deterministic data: synthetic source-shaped text generated in memory
  (seeded, no fs, no network — hermetic per AGENTS.md).
- Runs in docker like every other gate: `docker compose run bench`
  builds `--release` and runs the suite. Not in the `test` stage —
  wall-clock gates flake across machines; the bench is a measurement
  tool, not a CI assert.

## 3. Scenarios

| scenario | what it measures |
|---|---|
| `buffer_1mb` / `buffer_10mb` / `buffer_100mb` | open (Editor::new), a 100-keystroke insert session mid-file, 10× `dd`, 10× `u` |
| `nav_100k` | `G`, `gg`, `500j`, `/needle` (needle only on the last line — worst-case search) on 100k lines |
| `cursors_1k` | 999× ` c` (stack a cursor per line) then one cascaded `i…Esc` edit across 1000 cursors |
| `input_frame` | 500 insert-mode keystrokes, each followed by a full frame render at 120×40; p50/p95/p99/max |

Report: one aligned table, `scenario op n p50 p95 p99 max` (ms,
2 decimals). Greppable; the baseline below is a paste of one run.

## 4. Baseline (this machine, `--release`, docker bench service)

`docker compose run bench`, 2026-09-06 (Alpine container, shared CPU —
absolute numbers vary; the shape is the finding). Scratch buffers: the
text engine + render path; tree-sitter highlighting runs on its worker
(0022) and is outside this measurement.

```
buffer_1mb (12500 lines, 0.8 MB)
  open           p50=0.80   max=1.34 ms
  insert_100     p50=0.13   max=0.21 ms
  dd             p50=0.02   p99=0.06 ms
  u              p50=0.02   p99=0.08 ms
buffer_10mb (125000 lines, 8.0 MB)
  open           p50=6.13   max=6.80 ms
  insert_100     p50=0.13 ms
  dd / u         p99<=0.10 ms
buffer_100mb (1250000 lines, 80.2 MB)
  open           p50=68.9   max=111.0 ms
  insert_100     p50=0.13 ms
  dd / u         p99<=0.10 ms
nav_100k (100001 lines, 6.4 MB)
  G / gg / 500j  p50<=0.05 ms
  /needle        p50=3.35   max=3.96 ms   (worst case: hit on last line)
cursors_1k (1200 lines)
  add_cursor     p50=0.07   p99=0.12 ms   (no superlinearity over 999 adds)
  cascade_iZ     0.76 ms                  (one edit across 1000 cursors)
input_frame (10k lines, 120x40)
  key+frame      p50=1.18   p95=1.29   p99=1.48   max=1.67 ms
```

Read: no measured pain. Input-to-frame p99 is 1.5ms against a 16.7ms
frame budget; edits stay flat from 1MB to 80MB of buffer (the rope does
its job); cursor-add shows no O(n²) over 1k cursors. The only
three-figure number is the 80MB initial rope build (~69ms, one-time per
open). **No rewrite justified; the gate now exists for regression
diffs.**


## 5. Non-goals

- No optimization work in this plan beyond what the numbers make
  obviously broken (pathological p99 gets fixed; "could be 20% faster"
  does not).
- No perf claims on the site until these numbers exist.
- No CI assert on timings (flake machine-to-machine); the compose
  service is the gate's entry point.

## 6. Acceptance

- `strop --bench all` runs every scenario and prints the table.
- `docker compose run bench` green (release build + suite).
- Baseline table pasted into §4.
- Any pathological finding either fixed here or filed in the roadmap
  with its number attached.
