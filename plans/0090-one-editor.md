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
