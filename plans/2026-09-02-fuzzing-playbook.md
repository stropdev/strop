# Gripsack fuzzing playbook — replicable hardening rounds

How both hardening rounds (0.17.12, 0.17.13) were fuzzed, so a future
round can be run the same way. Companion doc to the agent-session
fuzzers; evening task: land the harnesses in-repo under `fuzz/` so
they survive /tmp wipes (a real session lost harness + artifacts that
way — see ops notes).

## Principles

1. **Black-box CLI fuzzing.** The binary is the target; no coverage
   instrumentation, no cargo-fuzz. Cheap, realistic, and it exercises
   the full stack (eval → sema → exec → store) exactly as users do.
2. **Crash-only criteria.** Flag exclusively: exit 101, `panicked at`
   anywhere, negative exit (signal), timeout, or *verified data loss*
   (file gone/wrong after a command reporting success). Warnings and
   ugly output are not findings.
3. **Isolation per invocation.** `GRIPSACK_HOME` fresh per case (or one
   warm home for eval-heavy campaigns — see below), `HOME` a SIBLING
   of the env repo (never inside it — deploy refuses in-repo
   destinations and you'll chase phantom refusals), `GRIPSACK_TRUST_ALL=1`
   (skip the trust prompt), `NO_COLOR=1`, per-invocation timeout 30–60s.
4. **Warm vs fresh homes.** Deno provisions (~40MB download) on first
   eval into `GRIPSACK_HOME`. Eval/flow campaigns: reuse ONE warm home
   to avoid re-downloading. Store-corruption campaigns: fresh home per
   case so state is deterministic.
5. **Sequential or nproc-limited execution.** Concurrent fuzzers +
   timing-sensitive e2e on the same host = false e2e failures (this
   happened; two "flaky" tests were fuzz CPU contention on WSL2).

## Seeds

- **IR**: the `ir` object (NOT the whole envelope) of
  `e2e/fixtures/golden/kitchen-sink.ir.json`, fed to `grip plan --ir FILE`.
- **Env repo template** (minimal valid):
  - `env.toml`: `[env]\nname = "f"\n`
  - `modules/<name>.ts`: `import { fileFetch, module, symlink, merge, dep } from "@gripsack/core"; export default module("m", { fetch: fileFetch("payloads/p.tar.gz"), install: { "bin/x": symlink("~/.local/bin/x") }, config: { "README": merge("~/.bashrc") } });`
  - `hosts/testhost.ts`: `defineEnv((ctx) => ({ tags: ["t"], modules: [m] }))` — **the host must import every module**; deps on unimported modules are E101 unknown-deps.
  - `payloads/p.tar.gz`: local tarball (python `tarfile`), the offline fetch source.
- **Gotchas that cost time**: `depends` wants `dep("name")` objects, not strings (strings → E000 "expected struct Dependency"); subcommands accepting `--host` are only adopt/apply/check/plan/update — rollback/gc/generations/why-owns/store-verify reject it (rc=2 arg-parse no-op if appended blindly).

## Campaign menu (per round: pick a spread; 20–30 min binary time each)

| Campaign | Method | Iterations used | Round-1 yield | Round-2 yield |
|---|---|---|---|---|
| IR mutation | structure-aware (delete/retype/insert keys, traversal strings, huge ints, unicode, dup keys) 70% + byte-level (bit flips, truncation, chunk dup) 30% on the golden seed | 1500–2500 | 0 panics (validated 0.17.11 parse hardening) | 0 |
| argv garbage | random 0–6 tokens from a seeded pool (subcommands, flags, huge ints, unicode, `/dev/null`, `--`) | 1200–2700 | 0 | 0 |
| eval flow | mutate TS modules / env.toml / host: line ops (delete/dup/swap/insert garbage) + raw byte flips; run `check`/`plan` | 150–1050 | 0 | 0 |
| store corruption | apply a valid env 2×, randomly corrupt ONE victim (manifest bit-flip/truncate/JSON-surgery incl. short hashes, tampered `store_path`, deleted store dir, junk files, `current` symlink → file/deleted), then run generations / gc --dry-run / gc / store-verify / store-verify --repair / rollback / why-owns | 100 rounds × 7 cmds | found: tree256 slice panic, short-hash panic, repair rm -rf traversal | 60 scenarios: 0 (post-fix) |
| interleavings | apply ×3 → drift edits → rollback → re-apply; take-over + mid-run failure (second module verify `exit 3`); subset applies | targeted | found: merge-marker corruption (HIGH), take-over prior loss | — |
| special files | `mkfifo`, symlinks to `/dev/zero`, as fetch payloads / adopt targets | targeted | — | found: adopt FIFO hang, fileFetch FIFO/zero hang |
| hostile tarballs | traversal paths, symlink entries, nested markers in payloads | targeted | tar traversal already refused (verified extraction reached) | marker payload → corruption finding |
| corners | adopt on symlinks/dirs/unreadable/ANSI paths, init into nonempty dirs, trust.toml corruption, `--repo` garbage, doctor with tampered home | targeted | — | clean |

The round-2 incomplete campaign (template rendering / take-over
corners / step-graph cycles) was lost to a /tmp wipe before finishing
— **first campaign to re-run next round**.

## Triage protocol (per finding)

1. Reproduce from scratch (fresh dirs) — record cmd + rc + stderr.
2. Minimize: strip the input until the crash needs exactly that.
3. Confirm against the pre-fix binary when possible (proves
   regression-free causality).
4. Fix at the root (never symptom-suppress); add a unit or e2e
   regression that pins the fixed behavior.
5. Re-run the finding campaign against the fixed binary (0 crashes).
6. Append the bug class to KNOWN-FIXED below.

## KNOWN-FIXED (do not re-report unless you find a BYPASS)

From 0.17.12: E116/E117 name validation; corrupt-lockfile loud failure
(bad JSON + non-64-hex pins) on apply AND update; store-verify --repair
confined to `$GRIPSACK_HOME/store` + lifecycle lock; merge into
non-UTF-8 dest refused; take-over priors restored on run-level rollback;
NDJSON deadlines + 1 MiB line caps + writer-thread stdin; download cap
is an error not truncation; GH enterprise token binds to `GH_HOST` only;
steps entries obey E102/E111.

From 0.17.13: module-scoped merge markers + strict marker-line matching
(payload quoting `<<< gripsack <<<` no longer truncates blocks; legacy
blocks upgrade on next apply); adopt/fileFetch/canonical-hash refuse
fifos/devices (symlinked payloads still followed); evaluated host
travels with EvalOutcome (no `update`/`apply` lockfile divergence);
fetchless build/run staging persists to publish; trust flock + escaped
prompt values; plugin tag = single safe path segment; git rev charset
validated; IPv6-aware shared URL host parser; gc dir_size symlink-safe.

## Ops notes (scar tissue)

- **Never share scratch roots between concurrent fuzzers** — round 1
  produced a phantom ENOENT when the flow fuzzer rebuilt a repo the
  apply scenario was reading. One root per campaign.
- **/tmp gets wiped by external cleanup** — keep harnesses + repros
  in the repo (`fuzz/`), artifacts under a dated dir.
- **Pipeline exit codes lie**: `cmd | head -N; echo $?` reports head's
  status — captured "rc=0" panics that were real. Capture rc from the
  command itself.
- **Plan of record for next rounds**: land `fuzz/{fuzz_ir,fuzz_argv,fuzz_flow,fuzz_store}.py`
  + this doc as `fuzz/README.md`; add a compose/just target
  `fuzz-smoke` (≈200 iterations per campaign) so every hardening round
  starts with a 10-minute smoke, full campaigns by need.

## Harness skeletons (condensed but functional)

```python
# fuzz_ir.py — IR mutation via `grip plan --ir`
SEED = json.load(open("e2e/fixtures/golden/kitchen-sink.ir.json"))["ir"]
WEIRD = ["", "../../etc/passwd", "/abs", "~", "..", "//", "a"*5000,
         "\\u0000x", "😀", "{store}", "$HOME", "`cmd`", "$(cmd)", "%n"]
SCALARS = [0, -1, 1<<40, -(1<<40), 1.5, True, False, None, "", [], {}]
def mutate(o, rng, d=0):           # deep-copy, then one of:
    # dict: del key / recurse / insert weird key / retype / weird str
    # list: pop / append-mutated / retype / repeat
    # scalar: weird scalar or string
def byte_mutate(b, rng):            # xor byte / delete run / insert
                                  # b"{}[]\"',:0123456789 " / truncate
run: [BINARY, "plan", "--ir", case, maybe_module_name], timeout=15
flag: rc==101 or "panicked at" or rc<0 or timeout
```

```python
# fuzz_argv.py
TOKENS = subcommands + flags + ["0","-1","99999999999999999999","--",
  "", "😀", "/dev/null", "..", "~", "a"*300, "\x01\x02", "--jobs=abc"]
args = [rng.choice(TOKENS) for _ in range(rng.randint(0,6))]
input=b"" (stdin closed), timeout=10, same crash flags
```

```python
# fuzz_flow.py — env-repo eval mutation
build_base(): fresh repo (modules/hosts/payloads/env.toml + tarball)
targets = modules/*.ts + env.toml + hosts/*.ts
mutate_file: line ops (pop/insert-garbage/swap) OR raw byte flips
cmd: check | plan | check --host testhost, timeout=60
env: HOME=<sibling scratch>, GRIPSACK_HOME=<warm shared>, TRUST_ALL=1
```

```python
# fuzz_store.py — corruption vs maintenance commands
setup(): apply valid env ×2 (two generations)
corrupt(): pick victim ∈ {manifests, store dirs, current symlink};
  bit-flip payload / delete file / rmtree / JSON surgery
  (short hash "deadbeef", store_path → /tmp/victim, entries tamper,
  tree256="ab", modules=null) / symlink→file / unlink
then run each: generations, gc --dry-run, gc, store-verify,
  store-verify --repair, rollback, why-owns <weird path>; timeout=45
```

Both rounds' full history: CHANGELOG 0.17.12/0.17.13 entries map
findings → fixes; e2e/unit regressions live beside the fixes.
