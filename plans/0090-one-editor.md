# 0090 — One editor: cut containers, a vim terminal, remote over plain SSH (0.55–0.56)

- Status: **in force** for 0.55 and 0.56; adoption record for
  [0089](0089-functionality-critique-0.54.md)
- Date: 2026-11-10
- Owner decisions (2026-11-10): cut containers and dev containers; adopt nvim's terminal
  model; replace the worker with plain `ssh exec` **provided the ergonomics match** (§5
  gate).

## 1. Why

0089 measured strop as two products: a vim editor that tests clean, and a
remote/containers platform that is 29 % of the code, has its own wire protocol over
three carriers, uploads a 50 MB copy of itself to every host, and sits under local
editing too (the local shell and LSP are children of a local worker process). It works
with the official musl artifact and nothing else. The terminal's engine is right and its
state machine is not. 19 bugs were found by using the product.

## 2. What strop is after this plan

- **Local**: in-process. `std::fs`, an in-process PTY, `Command::new` for LSP and tools.
  No worker process.
- **Remote** (`ssh://host/path`): one multiplexed OpenSSH connection per host.
  SFTP lists, reads and writes; `ssh exec` runs `rg`, `git`, the LSP server and the
  shell on the host. Nothing is installed on the host. A connected host is editable —
  one consent, the SSH login; `-R`/`:view` stay read-only.
- **Containers**: not a feature. `:terminal` then `docker exec -it …` is the path, and
  the docs say so.
- **Terminal**: nvim's model (§4).

## 3. Adoption of 0089, finding by finding

| Finding | Decision | Lands |
|---|---|---|
| §1.1, R6, R7, §8 0.55.1 containers + dev containers | **Cut** `strop-containers`, `editor/containers`, `editor/devcontainers`, their ex commands, `Space o → Containers`, the devcontainer offer, `container_editor` e2e, the `container-test` compose service and CI job | 0.55 |
| §1.3, §3 terminal | **Adopt** §4 | 0.55 |
| §1.4 discoverability | **Adopt**: `Space t`/`Space T`; `Space a` one typed-owner list titled "actions"; help gets "how to open" rows first | 0.55 |
| §1.5, B5 echo truncation | **Adopt**: the echo row wraps to at most 3 rows; `:messages` lists the last 50 messages; `:explain` retains refusals | 0.55 |
| §1.6 silent recovery | **Adopt**: opening a file with a recovery draft offers it (vim `ATTENTION` baseline: recover / open read-only / delete draft / edit anyway) | 0.55 |
| B1, B6 | Fixed in 0.54.0 | — |
| B2 B3 B4 B7 B10 B11 B13 | **Fix**. B3 (dirty after undo to save point) and B4 (double Esc) get Neovim differential cases | 0.55 |
| B8 B9 B16 B18 | Disappear with containers | 0.55 |
| B12 B15 B17 B19, R1, R2, R3, R5, R10, R11 | **Adopt §5**; these are properties of the worker design | 0.56 |
| R8 `--tail/--range/--follow` | **Cut** in 0.56 with the worker (they are worker reads); `strop -` reads stdin in 0.55 so `tail -f log | strop -`… is the replacement — **owner check** before deletion, recorded as a question in the 0.56 PR | 0.55 / 0.56 |
| §7.1 seven job mechanisms | **Adopt in 0.56**: `strop_core::worker` + `AppEvent` are the two that stay | 0.56 |
| §7.2 location types, §7.3 failure enums | **Adopt** where the worker deletion removes them; the rest F2 | 0.56 |
| §7.4–7.5 dead API, untested ex commands | **Adopt**: delete or test | 0.55 |
| §7.10 legacy flag spellings | **Cut** `--dump-compat`/`--replay-script` aliases | 0.55 |
| Kernel `frame` proof | **Retire with the worker** (0086 survival rule); edit-geometry and generation proofs stay | 0.56 |

## 4. Terminal (0.55)

```
Space t        new session in a horizontal split below, TERMINAL mode
Space T        new session in the current pane (= :terminal)
:terminal [cmd], :term
TERMINAL       every key goes to the child, including Esc and Ctrl-W
<C-\><C-n>     → NORMAL; the only escape chord (F2 and Ctrl-4 deleted)
NORMAL         a live buffer: follows output while the cursor is on the last line;
               k / Ctrl-U / search stop following; G resumes
i a I A        → TERMINAL at the child cursor
:q             hide the session; :bd! kills it
:qa            one notice "N terminals running — :qa! stops them"
re-entry       cursor and view restored, mode NORMAL — never 1:1
```

Deleted: pinned snapshots, `:terminal-refresh`, the held-paste review and both
`:terminal-paste*` commands (pastes are bracketed when the child asks, raw otherwise),
`:terminal-stop`, the `TERM`/`[RO]` special cases. Added: `TERMINAL` mode chip,
`term://` buffer names, `:r !cmd`. The PTY moves into the editor process for local
sessions; remote sessions are `ssh -tt host $SHELL -l` (fixes B19) in 0.56.

## 5. Remote over plain SSH (0.56) — with a go/no-go gate

**Shape.** `strop-remote` keeps `hosts/` (ssh config), `transport/` (SFTP v3) and `pool`.
A new `exec.rs` runs a quoted argv on the host over the multiplexed connection
(`ControlMaster`/`ControlPersist` in a private socket directory). Saves are SFTP write to
a temporary name + rename, refused when the remote file's size/mtime changed since load
(vim's "changed since reading"). Deleted: `strop-worker`, `strop-worker-client`,
`strop-worker-deploy`, `strop-worker-protocol`, `worker_catalog.rs`,
`--worker-stdio`, the deploy cache and GC, `:remote worker`/`:remote edit`/`:trust`
(connect = editable), the kernel `frame` module and its proofs/mutants.

**Gate (owner condition: "as long as ssh exec gives us similar ergonomics").** Before
anything is deleted, the exec path is built beside the worker and measured against it on
the same sshd container, same host, warm connection:

| Operation | Pass if |
|---|---|
| open a 1 MB file | ≤ worker time + 100 ms |
| save | ≤ worker time + 100 ms, and the changed-on-disk refusal works |
| workspace search (`rg`) first results | ≤ worker time + 150 ms |
| `git log` / blame for a file | ≤ worker time + 150 ms |
| LSP: hover/definition round-trip after attach | ≤ worker time + 50 ms |
| `:terminal` keystroke echo | no perceptible difference (≤ 20 ms p95 added) |
| first connect to a fresh host | faster than today (no 50 MB upload) |

Pass → delete the worker in the same change (no old/new pair). Fail on any row → stop,
keep the worker, record the numbers in §7 and ask the owner. Numbers, not impressions,
go in the record.

## 6. Gates

Every release: compose `test`, `model`, `proofs`, CI green, golden snapshots, a real
tmux smoke of every surface the release touched, and for 0.56 the SSH journeys in the
`ssh-pythonfree` lane extended to edit/save/search/git/shell over the exec path.

## 7. Implementation record

### 0.55.0

Shipped: containers and dev containers deleted (−21.5K lines, one crate, one CI
lane); the terminal on Neovim's model (`Space t`/`Space T`, `<C-\><C-n>` only, a
live NORMAL buffer that follows output, view restored on re-entry, `:q` hides,
`:bd!` stops, `:qa` one notice, `term://` names, `TERMINAL` chip, `:r !cmd`,
remote login shell); local terminal PTYs run in the editor process; B2–B4, B7,
B10, B11, B13; the echo row wraps to three rows and `:messages` keeps the last 50;
recovery drafts are offered on open (and a new session no longer overwrites an
earlier session's drafts — found while building the offer); `Space a` is one
typed-owner list; `strop -` streams stdin; untested ex commands tested; legacy
flag spellings removed. The Neovim differential harness now compares `&modified`.

Departures and notes:

- strop reads its own input bytes (`crates/strop/src/terminal/decode.rs`): crossterm
  0.28/0.29 decodes `ESC ESC` in one read as one key (B4) with no fixed release.
- Local PTYs reuse `strop-worker`'s `launch_pty`/supervisor, so `strop-terminal`
  depends on `strop-worker` for now. The 0.56 worker removal must move that code
  into `strop-terminal` first.
- The legacy `0x1c` byte that tmux without kitty keys sends for `Ctrl-\` decodes as
  Ctrl-4 in crossterm; it is treated as the same chord, not a second one.
- `:r FILE` is not implemented; the message points to `:r !cmd`.
- Known cosmetic: in NORMAL on a terminal buffer the statusline column can carry
  the child cursor's column.
- `--tail/--range/--follow` stay until 0.56 decides them with the worker.

### Remote go/no-go: worker vs plain ssh exec (§5 gate, measured 2026-11-10)

**Verdict: GO.** Every §5 row passes on the gate's terms (same sshd, same host, warm
connection). Per §5 the worker is deleted in the change that moves the editor onto the
exec path — not in this measurement change; nothing was deleted here.

**What was measured.** `crates/strop-remote/src/exec/` (beside the worker, unused by the
editor yet): one OpenSSH master per endpoint (`ssh -M -N -f`, `ControlPersist=10m`,
socket in a private 0700 directory), quoted-argv exec and `ssh -tt` terminals as
channels on it, and an SFTP v3 session on the same master for whole-file read, save
(temp name + `posix-rename`, refused when size/mtime moved since load) and listing,
with pipelined SFTP requests. The worker path is today's shipped code: deployed release
worker, `Worker::read`, `prepare`+`apply` Store with an mtime baseline, `exec`,
`exec_pty`.

**Method.** `docker compose run --build --rm ssh-bench` (test
`worker_ssh::exec_bench`, opt-in `STROP_SSH_EXEC_BENCH=1`). One container from the
repo's `builder` stage (Alpine 3.24, OpenSSH 10.3p1, rg, git) runs the `worker_ssh`
fixture's real `sshd` on 127.0.0.1 and both clients; release builds on both sides; the
deployed worker is the stripped release `strop` (50,609,520 bytes). Host: AMD Ryzen 9
9950X3D (32 threads), 30 GB, Docker 29.9 on WSL2 (kernel 6.18). Samples interleave
worker and exec operation by operation; N = 30 after 3 warmup; nearest-rank p50/p95 in
ms. Workspace: the repo's `crates/` tree (git repo, 31 commits on the blamed file) plus
1 MiB text files. Operations: open = read a 1 MiB file whole; save = write 1 MiB with
the changed-on-disk check; search = `rg --json RemoteEndpoint .` until the first match
line; git = `git log -n 50 --format=… -- file` and `git blame --porcelain -- file` to
exit; LSP = a stdio echo server (`cat`; no language server is in the image) — attach
is spawn → first framed reply, round trip is one 160-byte framed hover request echoed
on the attached server, i.e. transport RTT only; terminal = one byte into an
interactive `/bin/sh` on a remote PTY until its echo returns (the ssh client on a local
PTY, as the editor would run it); first connect = from nothing to a first directory
listing (worker: discovery + SFTP + 50 MB upload + verify + launch + handshake + list,
cache evicted between samples outside the timer; exec: master authentication + SFTP
session + list, master stopped between samples).

**Gate run — loopback (RTT ≈ 0):**

| Operation | worker p50 / p95 | exec p50 / p95 | Δ p50 / p95 | Gate | Result |
|---|---|---|---|---|---|
| open a 1 MB file | 10.2 / 26.9 | 8.4 / 19.0 | −1.8 / −7.9 | ≤ +100 ms | PASS |
| save a 1 MB file | 29.1 / 40.4 | 9.7 / 16.1 | −19.4 / −24.3 | ≤ +100 ms | PASS (refusal: below) |
| `rg --json` first result | 6.2 / 21.9 | 12.6 / 28.3 | +6.4 / +6.4 | ≤ +150 ms | PASS |
| `rg --json` complete | 25.5 / 51.0 | 27.3 / 45.0 | +1.9 / −5.9 | — | info |
| `git log -n 50 -- file` | 22.1 / 31.4 | 14.1 / 30.4 | −8.1 / −0.9 | ≤ +150 ms | PASS |
| `git blame --porcelain` | 27.7 / 48.0 | 16.0 / 26.8 | −11.7 / −21.2 | ≤ +150 ms | PASS |
| LSP attach (spawn → first reply) | 2.8 / 11.0 | 6.2 / 13.0 | +3.4 / +2.1 | — | info |
| LSP round trip after attach | 0.3 / 0.6 | 0.3 / 0.5 | −0.0 / −0.1 | ≤ +50 ms | PASS |
| `:terminal` keystroke echo | 0.6 / 3.5 | 0.5 / 3.4 | −0.1 / −0.1 | ≤ +20 ms p95 | PASS |
| first connect, fresh host | 2481 / 2956 | 135 / 153 | −2346 / −2803 | faster | PASS |
| reconnect, worker already cached | 1214 / 1307 | 136 / 161 | −1078 / −1146 | — | info |

The changed-on-disk refusal is asserted over the real sshd by
`worker_ssh::exec_journeys::save_is_atomic_preserves_mode_and_refuses_a_changed_file`
(another writer's change is refused and kept; `Expect::Any` overrides; save-as never
clobbers; mode is preserved; a symlink is written through).

**Sensitivity run — 20 ms RTT** (`-e DELAY_MS=10`: netem 10 ms each way on loopback;
not the gate's conditions, recorded because real hosts are not on loopback). Warm rows
N = 30 after 3 warmup; first connect N = 5 after 3 warmup (each worker sample is a
minute):

| Operation | worker p50 / p95 | exec p50 / p95 | Δ p50 / p95 | Within §5 budget? |
|---|---|---|---|---|
| open a 1 MB file | 26.6 / 31.6 | 86.3 / 94.8 | +59.7 / +63.1 | yes (+100) |
| save a 1 MB file | 64.0 / 74.6 | 87.7 / 98.1 | +23.6 / +23.6 | yes (+100) |
| `rg --json` first result | 25.1 / 34.7 | 50.2 / 54.8 | +25.1 / +20.1 | yes (+150) |
| `git log -n 50 -- file` | 42.3 / 51.1 | 53.4 / 58.1 | +11.1 / +7.0 | yes (+150) |
| `git blame --porcelain` | 42.5 / 50.0 | 55.7 / 64.5 | +13.2 / +14.5 | yes (+150) |
| LSP attach | 47.4 / 56.2 | 68.2 / 75.3 | +20.8 / +19.1 | info |
| LSP round trip after attach | 20.7 / 25.7 | 20.4 / 25.0 | −0.3 / −0.8 | yes (+50) |
| `:terminal` keystroke echo | 22.3 / 27.6 | 20.7 / 27.7 | −1.6 / +0.1 | yes (+20 p95) |
| first connect, fresh host | 74,298 / 74,515 | 435 / 437 | −73,863 | yes (faster) |

Reading it: exec costs about one extra round trip per process (a channel open on the
master before the exec request), and an open costs four (open, fstat, the pipelined
reads, close) against the worker's one streamed request. Open is therefore the first
row to cross its budget as latency grows — at roughly 35–40 ms RTT by extrapolation —
unless the stat is pipelined with the open and the close behind the reads (two round
trips; follow-up work, not done here). The worker's first connect is dominated by
round-trip-bound SFTP upload and read-back verification of 50 MB: 74 s at 20 ms RTT.
One attempt of that run also failed outright with `handshake failed: worker handshake
over ssh failed: no welcome within 10s` after a successful upload — the worker path has
its own failure mode under latency that the exec path does not.

**Not captured by the numbers (ergonomics in real use):**

- Per-exec login shell. sshd runs every exec through the user's login shell
  (`$SHELL -c`); bash reads `~/.bashrc` even then, so a heavy rc file (nvm, conda)
  adds its cost to *every* `rg`/`git`/LSP spawn. The worker spawns argv directly. The
  fixture's login shell is busybox `sh`.
- Non-POSIX login shells. Quoting is POSIX single quotes (`'\''`); fish accepts it, but
  csh/tcsh mishandle `!` and newlines inside quotes — such users need `SHELL` to be
  POSIX on the host or a `/bin/sh -c` wrapper.
- Authentication stays non-interactive (`BatchMode=yes`, `StrictHostKeyChecking=yes`):
  a password-only host or an unknown host key is a refusal carrying ssh's own words,
  exactly as today. The master makes a one-time interactive login possible later (run
  the master on a terminal once, everything after reuses it) — not built.
- `:terminal` must run `ssh -tt` on a local PTY. Over plain pipes the mux client cannot
  read terminal modes (`tcgetattr: Not a tty`): control characters (Ctrl-C) are lost
  and window size cannot follow. On a local PTY both work (measured above).
- Typeahead sent before the remote shell configures its line editor can be dropped (seen
  under delay on the exec path; the benchmark waits for the first prompt on both).
- SFTP mtimes are whole seconds: a same-size rewrite by another program within the
  second of the load is not detected. No cross-process lock (as in vim).
- The master is a background `ssh` per host that outlives an editor crash until
  `ControlPersist` (10 min idle) expires; a stale socket is removed on the next connect.
- Nothing is written to the host: no `~/.cache/strop-worker`, no lock file, no
  50 MB object, no version skew, and source/glibc builds work (R2, R11).

**Integrator's condition before deletion (2026-11-10).** The gate rows pass on
loopback, but at 20 ms RTT exec's file open already costs four round trips to the
worker's one and extrapolates past its +100 ms budget near 35–40 ms RTT — ordinary
for remote work. "Similar ergonomics" must hold over a WAN, so the worker is deleted
only after open is pipelined to two round trips and the open/save/search rows pass
again at **50 ms RTT** (`DELAY_MS=25`). The numbers go here beside the loopback run.


**WAN condition met (measured 2026-11-10).** Open was pipelined, then re-measured on the
same container, host and method as the gate run above (the same bench, worker side still
present, so the comparison is like for like).

What changed in the exec path: an open is two round trips — OPEN and STAT in the first
flight; every READ, a one-byte read past the end (the file grew), FSTAT (the handle's
attributes must match the STAT's) and CLOSE in the second, with CLOSE's status still
checked. A file that changed under the read is read again. A save is three — LSTAT with the
temporary's OPEN, then the WRITEs with CLOSE and the mode SETSTAT, then the rename with the
published attributes. A first pass sent the read's tail only after the last read reply had
arrived (three round trips): it passed at 50 ms RTT by 0.9 ms (+99.1 / +98.1 against
+100), so the tail now rides the same flight as the reads; the table below is that final
shape.

`DELAY_MS=25` (50 ms RTT; netem 25 ms each way on loopback), N = 30 after 3 warmup,
nearest-rank p50/p95 in ms, warm connection. First connect from the earlier pass (N = 3
after 3 warmup; each worker sample is three minutes at this delay):

| Operation | worker p50 / p95 | exec p50 / p95 | Δ p50 / p95 | Gate | Result |
|---|---|---|---|---|---|
| open a 1 MB file | 54.5 / 56.1 | 104.0 / 104.8 | +49.5 / +48.7 | ≤ +100 ms | PASS |
| save a 1 MB file | 147.9 / 168.8 | 156.2 / 204.1 | +8.3 / +35.3 | ≤ +100 ms | PASS |
| `rg --json` first result | 53.7 / 54.5 | 107.5 / 108.8 | +53.8 / +54.4 | ≤ +150 ms | PASS |
| `rg --json` complete | 104.7 / 105.6 | 113.9 / 115.9 | +9.2 / +10.3 | — | info |
| `git log -n 50 -- file` | 71.8 / 72.6 | 109.7 / 112.3 | +37.9 / +39.8 | ≤ +150 ms | PASS |
| `git blame --porcelain` | 71.8 / 72.0 | 111.7 / 112.4 | +40.0 / +40.5 | ≤ +150 ms | PASS |
| LSP attach | 101.9 / 102.3 | 154.4 / 155.3 | +52.5 / +53.0 | — | info |
| LSP round trip after attach | 50.7 / 50.9 | 50.5 / 50.7 | −0.2 / −0.2 | ≤ +50 ms | PASS |
| `:terminal` keystroke echo | 50.7 / 50.8 | 50.6 / 50.8 | −0.1 / −0.0 | ≤ +20 ms p95 | PASS |
| first connect, fresh host | 183,614 / 183,901 | 928 / 956 | −182,686 | faster | PASS |

Loopback, same build: open 4.9 / 6.3 → 4.1 / 4.7, save 19.3 / 25.0 → 5.5 / 6.5, `rg`
first result 3.4 / 3.9 → 6.7 / 7.6, `git log` 21.6 / 21.7 → 9.1 / 9.9, `git blame`
21.6 / 21.8 → 10.6 / 11.7, LSP round trip 0.5 / 0.6 → 0.3 / 0.4, echo 0.6 / 0.7 → 0.5 /
0.5 (worker → exec, p50 / p95); every gate row passes. The exec journeys (argv/cwd quoting,
atomic save with the changed-on-disk refusal, listing) passed over the same sshd.

Reading it: exec now costs one round trip more than the worker for open and for a process
spawn (the channel open on the master), comfortably inside every budget at 50 ms RTT; the
first open budget is crossed only past roughly 100 ms RTT. The worker remains unusable to
connect at this latency (three minutes per fresh host).

### 0.56 record

Shipped (the worker deleted in the change that moved the editor, per §5):

- **Local in process.** `LocalFs` (`editor/namespace/local.rs`) calls the strop-fs kernel
  directly — observation, whole reads, listings, reviewed batches, document Store and
  verification — under one execution context. The inotify watcher moved from the worker into
  `strop_fs::notify` and runs on one editor thread that subscribes, settles onto the notify
  queue and pumps hints. Git and language servers spawn with `Command`; terminals run on the
  in-process PTY whose launch/supervision moved (not copied) from `strop-worker` into
  `strop-terminal/src/pty/`.
- **Remote over one master.** `RemoteClient`'s per-endpoint actor authenticates the master
  (`ExecHost::connect`, cancellable) and runs its SFTP session as a channel on it;
  `RemoteClient::save` (Expect: Unchanged stamp / Any for `:w!` / Absent for save-as),
  `host()` (connects on a job when needed) and `connected_host()` (no I/O, for the input
  path) feed rg, git, LSP and terminals. Masters the client started are stopped when it
  drops. A remote terminal is `ssh -tt` on a local PTY with typeahead held until the first
  output byte.
- **One consent.** `:remote worker`, `:remote edit`, `:remote verify` and remote `:trust`
  are gone; a connected host is editable and a remote project layer's server runs there.
- **Deleted.** `strop-worker`, `strop-worker-client`, `strop-worker-deploy`,
  `strop-worker-protocol`, `worker_catalog.rs`, `bootstrap.rs`, `worker_transport.rs`,
  `deploy_provider.rs`, `selection.rs`, the remote follow/view machinery, the worker-only
  failure enums and `map_client`, the capability gating that only modelled worker
  admission, strop-core's worker cache/deploy/session/recovery policies, `--worker-stdio`
  and its release smoke step, the catalog's `worker` manifest, the kernel `frame` module
  with its theorems, Audit claims and mutants, and the `WorkerSession` TLC model.
- **R8 decided: `--tail/--range/--follow` and `:tail/:range/:follow/:unfollow` are cut.**
  Reimplementing them over SFTP (ranged reads, polling follow, window-relative line
  numbers) would have kept the window type through the document model, completion, Git,
  LSP and the statusline for a log-reading niche that `ssh host tail -f FILE | strop -`
  covers. Owner check stays open as a question on the 0.56 PR.
- **Remote file management over SFTP.** `RemoteKernel` (`editor/namespace/remote.rs`)
  implements strop-fs's `StepKernel` over `RemoteClient::observe/mutate`, so the batch
  orchestration (missing parents as steps, dependency order, cancellation, the
  unconfirmed cascade, verify) is the local one. Steps: exclusive create, MKDIR, v3 RENAME
  (refuses an existing name; never `posix-rename`), copy = whole read + exclusive create
  with the mode, REMOVE/RMDIR. Each step re-observes its source (kind, size, mtime) and
  destination just before running. Limits: no Trash (removal permanent, said in the
  review), no recursive directory copy, no atomicity across steps, no lock. Touched remote
  directory views reload after apply (a host is not watched).
- **Lanes.** `ssh-pythonfree` now runs the exec journeys against a real listening sshd and
  the editor journeys (`remote_ssh`: open, edit, save, changed-on-disk refusal, `:w!`,
  save-as, file management and its refusals, search, `Space g l`, terminal with Ctrl-C and
  typeahead, failure refusals)
  through the real binary. `ssh-bench` is an exec-only latency bench (opt-in).

**Known flake (open).** `picker::preview::watchdog::stalled_preview_recovers_after_physical_reader_returns`
runs a 20 ms stall timeout and failed twice under full-suite load (0.54 and
0.56 integration); 240 runs at 48-way parallelism did not reproduce it. It is
recorded here rather than patched blind; the fix lands with a reproduction.
