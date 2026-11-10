# 0089 — Independent functionality critique of 0.54 (scope, bloat, terminal, remote, discoverability)

- Status: **record** — adoption is the owner's decision and will be a separate plan
- Date: 2026-11-10
- Author: an independent reviewer agent (Claude Fable), commissioned by the owner;
  read-only against the `look/0.54` branch. B1 and B6 were fixed before 0.54.0 shipped.

The text below is the reviewer's report, unedited except for this header. The
evidence captures it cites were working material and are not committed.

---


Scope: scope, bloat, flow, correctness, discoverability. Not look-and-feel (0087/0088).
Branch `look/0.54`, commit a956cad plus uncommitted theme/golden edits. Build: host
`cargo build -p strop-editor` (debug, x86_64-unknown-linux-gnu, 416 MB); driven in tmux
140x40 with an isolated `$HOME`/XDG; a throwaway Alpine sshd container (`dev@127.0.0.1:2299`,
git + ripgrep inside) and a plain `alpine sleep infinity` container; the musl release
worker built with the repo's own `Dockerfile` `release` stage.

Labels: **[V]** verified by running or by reading the exact code; **[S]** inferred.

---

## 1. Executive summary (ranked)

1. **The product is two products stapled together.** ~62K of 213K LOC (29%) [V, `wc`] is
   remote/worker/containers/devcontainers/terminal/filesystem-transactions — none of it in
   the 0001 vision, all of it grown from a 0.16 "read an SSH log file" feature. The vim
   editor underneath is good; the remote stack is a TRAMP+VS-Code-Remote clone with its own
   wire protocol, deploy pipeline, cache GC and tar parser. It does work end to end with the
   official musl artifact (edit, save, shell, search, git over SSH and into a container — all
   verified), but a source-built editor cannot use any of it (R11). **Recommendation:** keep SSH as
   *one* feature (edit files over SFTP, run `rg`/`git`/LSP over `ssh exec`), delete the
   worker deploy model, containers and devcontainers (§8).
2. **Every local operation already goes through the remote wire.** strop spawns
   `strop --worker-stdio` on startup; rust-analyzer and the local shell are children of the
   *worker*, not the editor [V, `ps --forest`]. Local file IO, PTY bytes and LSP stdio all
   cross a `Content-Length` frame codec with credits and leases. This is the single biggest
   source of accidental complexity and the reason "simple" features (terminal, recovery)
   feel heavy. **Recommendation:** local = in-process; the worker is for SSH only.
3. **Terminal: right engine, wrong state machine.** libghostty-vt is a good choice; the
   pinned-snapshot inspection model is not. Five concrete frictions and three bugs found in
   a ten-minute walk (§3): cursor resets to 1:1 on re-entry, no key opens a terminal,
   `:qa` opens a picker titled "filesystem actions", F2 as the only sane escape.
   **Recommendation:** adopt nvim's model exactly (`:terminal`, `<C-\><C-n>`, live buffer
   follows output unless scrolled) plus two leader keys; drop held-paste review, pinned
   snapshots and `:terminal-refresh`.
4. **Discoverability is ex-only for everything outside the vim grammar.** Terminal,
   containers, remote edit/worker, recovery, collections, change review, indentation
   controls: 56 ex commands, zero leader keys for terminal/containers/recovery, help lists
   `:containers` under "Documentation" [V]. **Recommendation:** one `Space t` (terminal) and
   fold `:remote edit`/`:remote worker`/`:container-*` into one `Space a` action per
   context.
5. **The echo row truncates every long refusal with `…` and `:explain` does not retain
   it** [V]. Refusal text is the product's main UI for the remote stack, and users cannot
   read it. Fix: wrap into a card or retain every message in `:explain`.
6. **Recovery works but is silent** [V]. After SIGKILL and relaunch there is no offer; the
   draft only appears in `:recover list`. Vim's swap prompt is the baseline.
7. **Two vim-fidelity bugs in core** [V]: `x` then `u` leaves the buffer dirty (vim clears
   `'modified'` at the save point); a fast double `Esc` (`\x1b\x1b` in one read) is
   swallowed, so Esc-Esc to close a picker needs a pause.
8. **`strop` (no file) → `Space ?` → `q` exits the editor** [V]. First-run path, first
   thing a new user does, and it quits.
9. **Dev Containers depends on an npm CLI strop refuses to install** [V]. In a
   zero-config single-binary editor, a 9.2K LOC feature that dead-ends at "reference Dev
   Container CLI was not found" earns nothing. Cut.
10. **Seven overlapping job/mailbox mechanisms and 22 failure enums** glued by six
    identical `fn failure()` copies [V, audit §7]. The remote stack's safety vocabulary
    (`StepOutcome`/`VerifiedOutcome`/`Refusal`/`DeployRefusal`/`FsFailure`/…) leaks into
    user-facing text ("unsupported remote save capability", "a worker-admitting remote
    action admits it").

Where I disagree with the owner's instinct: the **terminal should stay** (the engine is
sound and `:terminal`+`Space |` are what vim users expect); **SSH should stay** (it is the
one remote feature with a real user story and the SFTP path is clean and dependency-free).
The owner is right about containers, devcontainers, and "many protocols".

---

## 2. Feature inventory

LOC = non-generated `.rs` in the crate(s) and `editor/` modules named, tests included
(tests are 20–45 % of each). "Tests" = `#[test]` count in those modules. Discoverability:
W = welcome card, K = which-key/leader, H = `Space ?` help, X = ex command only, E = echo
row offer, C = CLI.

| Feature | LOC | Plans | Tests | Discovery | Transports | Verdict |
|---|---|---|---|---|---|---|
| Vim grammar, preview, visual, registers, macros (`strop-grammar`, `editor/normal`, `visual`, `registers`, …) | ~12K core + render 13K | 0001, 0008, 0016, 0074, 0082 | 42 grammar + nvim differential | W K H | — | **keep** |
| Pickers + query language (`strop-picker`, `editor/picker`) | 18.8K | 0051, 0053, 0063, 0077 | 167 + 87 | K H | — | **keep**, trim 16 `Kind`s |
| Search workspace + replace + collections (`editor/collections`, `changes`, `prepare`, `transact`) | 2.4K + 4.0K | 0043, 0044, 0053 | 30 + 22 | K H | — | **keep** search/replace; **simplify** collections |
| Git (`strop-git`, `git_memory`, blame, permalink) | 9.6K | 0001 P3, 0010, 0018, 0032 | 71 + 33 | K H | libgit2 + `git` | **keep** |
| LSP (`strop-lsp`, `editor/lsp`) | 18.3K | 0009, 0059, 0067 | 106 + 69 | K H | stdio via worker | **keep**; move spawn in-process |
| Completion + snippets | 8.3K | 0059, 0069 | 53 | H | — | **keep** |
| Syntax (`strop-syntax`) | 4.5K | 0022, 0039, 0066, 0073, 0083 | 72 | — | — | **keep** |
| Terminal (`strop-terminal` + ghostty-vt + `editor/terminal`) | 9.0K + 7 MB vendored C | 0017, 0055, 0065, 0075, 0076 | 25 + 25 | H X | PTY **via worker protocol** | **keep, rework flow** (§3) |
| Remote SSH/SFTP (`strop-remote`, `editor/remote`, `remote_completion`) | 12.2K | 0034–0036, 0038, 0040, 0075 | 56 + 34; 10 sshd e2e (env-gated) | K(`Space o`) H C | `ssh -s sftp` (own SFTPv3 codec), `ssh exec` | **keep, simplify** (§4) |
| Worker stack (`strop-worker{,-client,-deploy,-protocol}`) | 15.0K | 0056, 0058, 0086 | 153; 26 sshd e2e, 1 in CI gate | invisible | own framed protocol, deploy via SFTP/`docker cp`, cache GC | **cut deploy; keep in-process serve for SSH only** |
| Containers (`strop-containers` minus devcontainer, `editor/containers`) | 5.1K | 0036, 0037, 0058 | 64 + 10; 2 e2e | H(misfiled) X | `docker cp` tar streams, `docker exec`, `docker inspect` | **cut** |
| Dev Containers (`strop-containers/devcontainer`, `editor/devcontainers`) | 9.3K | 0037, 0081 | 21 + ~40 | E | npm `devcontainer` CLI | **cut** |
| Filesystem workspace: Review→Apply→Receipt, `:fs *`, filename draft (`strop-fs`, `editor/filesystem`, `directory`) | 12.0K | 0054, 0040 | 41 + 32 | K(`Space e`) H | local + worker | **simplify** to oil-style edit-buffer; drop receipts/verify/undo ledger |
| Trace/replay/export (`strop-trace`, `editor/trace`, `replay.rs`) | 5.2K | 0029 | 32 + 17 | C | — | **keep** (dev tool), stop documenting in README |
| Recovery (`editor/recovery`) | 1.8K | 0056 AR04 | 14 | X | — | **keep, surface it** |
| Sessions (`session/`) | 0.6K | 0001 | — | automatic | — | keep |
| Headless driver | 0.7K | 0006 | — | C | — | keep |
| Explain (`editor/explain`) | 0.8K | 0067 | 11 | X | — | keep |
| Notify/feedback (retained notices, `Space a` multiplexer) | 3.0K | 0081 | 20 | K | — | **simplify** |
| Self-update (`update.rs`, ureq/rustls) | 0.7K | 0062, 0079 | — | C | HTTPS | keep |
| Bench | 0.8K | 0025 | — | C | — | keep |
| Indentation policy (`structural_indent`, `indent`, `analysis`) | ~2.6K | 0072, 0084 | — | H X | — | keep |
| Debugger | 0 | 0019, 0060 | — | — | — | not built; leave |

Protocols/transports in the tree today [V]: (1) OpenSSH `ssh -s sftp` with an in-house
SFTP v3 codec (`transport/wire.rs`, 775 lines); (2) `ssh host <fixed POSIX line>` discovery
bootstrap; (3) the strop worker protocol (`Content-Length` frames, class byte, JSON control
+ binary chunk streams, credits, leases, incarnations) over three stdio carriers: local
child, `ssh exec`, `docker exec`; (4) `docker cp … -` tar streaming with an in-house tar
reader (`tar.rs`, 597 lines) for read-only container listing/read; (5) `docker inspect`/`ps`
JSON; (6) the Dev Container reference CLI JSON; (7) LSP JSON-RPC (inside the worker
carrier); (8) libgit2 + `git` CLI. That is eight, and five of them exist only to serve the
remote/container story.

---

## 3. Terminal

### 3.1 What I did [V]

Opened `strop main.rs`; `:terminal`; typed `ls -la`, `seq 1 60`, `echo MARKER_END`; `F2`;
`gg`, `/MARKER`, `yy`, `Ctrl-^`, `p` into `main.rs`; back with `Ctrl-^`; `Space w v`;
`:terminal` in the new pane (session #2); `sleep 300`; `F2`; `Space w q`; `Space b`;
`:terminal ls -la` (session #3, exits); `:qa`; `:qa!`. Also `Esc` and `Ctrl-C` go to the
child (correct).

### 3.2 What works

- PTY, VT rendering (ghostty-vt), colours, resize/reflow, scrollback into a rope, `/` and
  `yy` over the scrollback, multiple sessions, hidden sessions in `Space b`, `:terminal CMD`
  runs and retains output with exit status, `:qa!` kills children (`sleep 300` was gone).
- The implementation choice — libghostty-vt statically compiled through Zig, PTY via
  rustix-openpty, a rope projection of screen+scrollback — is sound. I would keep it. The
  cost is a Zig 0.16 toolchain for source builds and 7 MB of vendored tarballs, acceptable
  for a prebuilt-binary product.

### 3.3 Friction and bugs

| # | Observation | Evidence |
|---|---|---|
| T1 | No key opens a terminal. `Space` which-key has no terminal entry; only `:terminal`. The help's Terminal section is 7 rows of ex commands and chords. | [V] which-key capture; `keymap/bindings/terminal.rs` has zero key bindings, only doc rows |
| T2 | **Cursor position lost on re-entry.** Inspect at 72:123, `Ctrl-^` away, `Ctrl-^` back → 1:1 and the view is at the top of scrollback, not the prompt. | [V] statusline `72:123` → `1:1` |
| T3 | **Re-entry lands in inspection mode at line 1**, so typing a command (`echo BACK`) runs `e c h o` as motions and refuses `c`, then `A` flips to input and `CK⏎` reaches the shell. | [V] |
| T4 | **Pinned snapshot**: in Normal the buffer freezes; new output needs `Space a → Refresh` or `:terminal-refresh`. nvim, kakoune, zed, wezterm all show a live buffer and simply stop auto-scrolling while you are scrolled up. The pin exists to protect a yank highlight (0055 §7) — a solution to a problem the others never had. | [V] docs + `lifecycle.rs` |
| T5 | Mode chip stays `TERM` in both input and inspection; the only difference is a hint string on the right. Input mode should be a distinct mode (`TERMINAL`), inspection should be plain `NORMAL`. | [V] captures |
| T6 | Pane title shows `[RO]` for a live terminal in input mode. | [V] `╭ 2 terminal #2 · launch … [RO]` |
| T7 | `:qa` with live sessions opens a picker **titled "filesystem actions"** listing Focus/Stop per session; there is no "stop all and quit". nvim: one `E947` and `:qa!`. | [V] capture saved `bug-qa-filesystem-actions-title.txt` |
| T8 | Three ways to leave input (`F2`, `Ctrl-\ Ctrl-n`, `Ctrl-4 Ctrl-n`), and the help lists all three plus a Windows caveat. The Windows GUI was deleted in 0.53 (0086); `F2` and `Ctrl-4` are leftovers of it. | [V] help rows 155–161, STATUS.md |
| T9 | Held-paste consent flow (`:terminal-paste`, `:terminal-paste-cancel`, `Space a → Review held paste`) for multiline paste when the child did not enable bracketed paste. No other editor or multiplexer does this; it costs three ex commands and a ticket model. `terminal-paste-cancel` has no test [V, audit]. | |
| T10 | Terminal PTY lives in the **worker process**; keystroke → editor → frame codec → worker → PTY → chunk → editor. Works, but it is why a local shell needs "admission" vocabulary in error text and why `:terminal` in an SSH directory *refuses* instead of opening a local shell. | [V] `ps --forest`, `lifecycle.rs:498` |
| T11 | No `:r !cmd`, no `:terminal` into a split by default (replaces the current pane), no `Space |`-style "run and insert output" except via pipe of a line. | [V] EX_COMMANDS |
| T12 | The echo row covers the line above the prompt in inspection mode (line 73 `MARKER_END` hidden behind "terminal snapshot — i/I/a/A enter input"). | [V] |

### 3.4 Redesigned flow (key by key)

Target: a vim user who knows nvim's `:terminal` is productive in 30 seconds; no new
vocabulary.

```
Space t        open terminal in a horizontal split below (new session), cursor in it, TERMINAL mode
Space T        open terminal in the current pane (= :terminal)
:terminal [cmd]  as today; :term alias
TERMINAL mode  every key to the child, incl. Esc, Ctrl-C, Ctrl-W
<C-\><C-n>     → NORMAL (nvim's chord, the only one; drop F2 and Ctrl-4)
NORMAL         ordinary buffer: live, follows output while the cursor is on the last line;
               scrolling up (k, Ctrl-U, /…) stops following, G resumes — exactly nvim
i a A I        → TERMINAL mode, cursor at the child cursor (not "input vs snapshot")
Space w …      panes as today; closing a pane hides the session (today's behaviour, good)
Space b        sessions listed as `term://…` buffers (today, good)
:q on the buffer  hide; :bd!  kill
:qa            "E947-style" one notice: `2 terminals running — :qa! stops them`; no picker
Ctrl-^ / re-entry  restore cursor, mode = NORMAL (nvim) — never reset to 1:1
```

Delete: pinned snapshots, `:terminal-refresh`, `Space a → Refresh`, held-paste review and
both `:terminal-paste*` commands (send pastes; bracketed when the child asks, raw
otherwise — what every terminal does), `F2`, `Ctrl-4`, `:terminal-stop` (use `:bd!`),
the `TERM`/`[RO]` presentation special cases. Add: `Space t`/`Space T`, `:r !cmd`
(vim), `term://` buffer naming, a `TERMINAL` mode chip.

Implementation: keep `strop-terminal` (VT, projection, launch). Move the PTY into the
editor process for local sessions (`rustix-openpty` is already a dependency of
`strop-worker`); keep the worker PTY path only for SSH. The snapshot/pin machinery in
`editor/terminal/lifecycle.rs` (528 lines) and `input.rs` (298) shrinks by more than half.

---

## 4. Remote, containers, protocols

### 4.1 Map

```
                  ┌──── SFTP v3 (own codec) ── read-only browse/read/tail/follow ──┐
 editor ── ssh ───┤                                                                 ├─ remote host
                  └──── exec "printf uid uname cache" (bootstrap) ──┐              │
                       exec "<cache>/strop --worker-stdio" ─────────┴─ worker protocol ── PTY, rg, git, LSP, store/prepare/apply/verify
 editor ── docker ─┬── cp <id>:<path> -   (tar stream, own tar reader) ── read-only browse/read
                   ├── inspect / ps        (identity, StartedAt incarnation)
                   └── exec <id> <cache>/strop --worker-stdio ── worker protocol (after docker cp upload of the 50 MB artifact)
 editor ── devcontainer CLI (npm) ── read-configuration / up / rebuild ── then the docker path above
 editor ── local child: strop --worker-stdio ── worker protocol ── local fs, PTY, LSP spawn
```

Transports: 3 carriers for one protocol + SFTP + tar-over-docker-cp + two CLIs. Overlap:
SFTP read vs worker read [V, `namespace.rs:176` two read paths]; `docker cp` read vs worker
read [V]; `strop_fs` called directly by the engine *and* served through the in-process
worker [V, audit §4].

### 4.2 What I ran [V]

- `Space o` → `ssh://dev@devbox:2299/` (read from the isolated `~/.ssh/config`) → `/`
  listing, `:remote home`, open `project/notes.txt` read-only. Works, fast, no worker.
  Every connect prints `git discovery failed: SSH Git requires an admitted worker` in the
  echo row — a failure message for a feature the user did not ask for.
- `:remote edit` with the debug editor: `unsupported remote save capability: worker artifact
  exceeds the 268435456-byte supply bound`. Correct refusal, meaningless to a user.
- `:terminal` in the SSH directory: `no admitted worker on ssh://…: interactive terminals
  there need one (a worker-admitting remote action admits it); use :ter…` — truncated, and
  the hint does not name `:remote worker`.
- `Space a` on the remote file: one entry, "Dismiss notice". No "edit", no "open terminal
  here", no "admit worker".
- `:containers` → picker of 8 running containers → attach `alpine:3.20` → listing shows
  `?` for every size and `d?????????` for every mode; statusline `main / [RO]` (the *local*
  git branch); `:e etc/hostname` opens the file but the statusline says `[scratch] [RO]`.
- `:container-worker`: `this editor is built for x86_64-unknown-linux-gnu, but
  x86_64-unknown-linux-musl needs a matching worker artifact; set STRO…` (truncated).
- Dev container: `.devcontainer/devcontainer.json` with `"image": "alpine:3.20"` →
  offer appears in the echo row → `Space a → Review` → a 27-line trust-review buffer →
  `Space a → Probe the optional reference CLI` → `unavailable: reference Dev Container CLI
  was not found; install it explicitly`. Dead end without `npm i -g @devcontainers/cli`.

**With a deployable worker** (I built the 52 MB musl release with the repo's `Dockerfile`
`release` stage and ran *that* binary as the editor, because the glibc build cannot deploy —
see R11) [V]:

- `:remote edit` on `notes.txt`: upload + verify + handshake in < 20 s → "remote editing
  enabled (cooperative locks; other programs can still race)". `o…⏎ :w` → "written
  remotely"; the file changed on the host. Host now holds
  `~/.cache/strop-worker/{objects,receipts,leases,staging,cache.lock}` and two sshd sessions
  (sftp + exec) plus the worker process.
- `:terminal` in the SSH directory → remote `/bin/sh` in `/home/dev/project` (the user's
  login shell is bash; the worker launched `sh`). `id`/`hostname` correct.
- `Space /` `remote` → remote `rg` results with per-file previews, scoped to the remote
  root. Fast and correct.
- `Space g l` on the already-open `notes.txt` → **"not a git repo"**; opening `lib.rs` after
  admission → the log appears. Git discovery is run once at open and not re-run after
  admission (B15).
- `lib.rs` opened `[RO]`: the write permit is per document, so every file needs its own
  `:remote edit` (docs say so; it is still the wrong grain).
- Containers with the musl editor: `:container-worker` on the alpine container → "container
  worker 0.54.0 ready … at /root/.cache/strop-worker/objects/…" in ~20 s (52 MB `docker cp`);
  `:e tmp/note.txt` → `[scratch] [RO]` in the statusline (B8); `:container-edit` →
  "container editing enabled (conditional saves still protect the baseline)"; `:w` →
  "written in container", verified with `docker exec cat`. `:terminal` → `/ #` shell in the
  container. The statusline branch chip read `master` — the **SSH** repo's branch — while
  browsing the container (B16).

So the machinery is real and, when the artifact matches, robust. The cost is what §4.3
describes: a user with a source build, a non-matching version, a glibc-only host, or a
restricted account gets a refusal in a vocabulary they cannot act on, and every host they
touch gains a 52 MB cache directory and a lock file.

### 4.3 Findings

| # | Finding | Evidence |
|---|---|---|
| R1 | **Four authority steps where one is needed.** `:remote connect`, `:remote worker`, `:remote edit`, `:container-worker`, `:container-edit`, `:trust`. VS Code Remote and TRAMP have one: connect. The split exists because the design treats "may run code on the host" and "may write this file" as separate consents, but the user already authenticated as that principal — the host is theirs. | docs/remote.md, EX_COMMANDS |
| R2 | **The deploy model uploads strop itself** (50 MB release, 416 MB debug — refused) to a per-user cache, verifies SHA-256, runs cache GC with OS locks, binds incarnations. It needs a musl artifact matching the exact version; a glibc source build can only talk to glibc hosts; version skew between editor and worker is a refusal. Every editor upgrade re-uploads. TRAMP/`vim scp://` need nothing on the host; VS Code uploads a server but has a team to maintain it. | `worker_catalog.rs`, `bootstrap.rs:117`, `strop-worker-deploy` |
| R3 | **Local editing depends on the worker**: no in-process fallback for local reads/PTY/LSP spawn. | `ps --forest`; `namespace.rs:121` |
| R4 | **Refusals are the UI** and they are truncated with no retention. | §6 B5 |
| R5 | **SFTP read path and worker read path coexist** for the same namespace; container `docker cp` read path and worker read path coexist. | audit §4 |
| R6 | **Containers**: read-only browsing is implemented by streaming tar archives of directories through `docker cp` and parsing them in-house (597 lines) to avoid requiring `ls` in distroless images. The listing loses mode/size (`?`), and anything beyond reading requires the 50 MB upload. The common case (a dev container with a shell) is served better by `docker exec -it <id> sh` in the terminal. | `read.rs`, capture |
| R7 | **Dev Containers**: 9.3K LOC of JSONC parsing, trust review, effects redaction, lifecycle jobs — gated on an npm CLI strop will not install; `image`-only configs could be a `docker run`, which the design refuses on principle. | capture, `devcontainer/cli.rs` |
| R8 | **`--tail/--range/--follow` log-reader flags** are the fossil of the original 0034 "SSH log buffers" use case. They are CLI flags *and* ex commands, with window-relative line numbers and their own `selection.rs` (301 lines). `tail -f | strop -` would serve the same need. | docs/cli.md, CHANGELOG 0.16/0.17 |
| R9 | Test weight is honest but thin in CI: 26 sshd e2e tests exist, **one** runs in the `ssh-pythonfree` gate; the rest need `STROP_REQUIRE_SSH_TESTS=1`. `container_editor` has 2 tests. | Dockerfile:43–50, compose |
| R10 | Vocabulary leak: "admitted", "incarnation", "lease", "permit", "Store baseline", "supply bound", "namespace" appear in echo-row text. | captures |
| R11 | **A glibc-built editor can never deploy a worker over SSH**, and `STROP_WORKER_BINARY` does not rescue it. Discovery maps every Linux host to `x86_64-unknown-linux-musl` (`bootstrap.rs:117`); the release catalog is keyed by the *editor's* triple (`workers.rs:151–164`, `local_binary_target()` returns `TARGET_TRIPLE`); the verdict is `NoArtifactForTarget` (`manifest.rs:83`). With the debug gnu editor I got "worker artifact exceeds the 268435456-byte supply bound"; with `STROP_WORKER_BINARY=<musl release>` I got "no worker artifact for target x86_64-unknown-linux-musl". docs/remote.md promises the override works. Only the official musl release can do remote work — `cargo install`/source users cannot. | [V] |
| R12 | The remote stack adds almost no external dependencies (only `rustix-openpty`, `unicode-normalization`, `tinyvec` are unique to it [V, `cargo tree` diff]) because the SFTP codec, tar reader, wire protocol, deploy cache and GC are all hand-written: ~37K LOC of in-house protocol code is the cost, not the dependency tree. | [V] |

### 4.4 The simplest architecture that keeps the valuable 20 %

The valuable 20 %: open `ssh://host/path`, browse, edit, save; `rg` and `git log/blame`
over there; a shell over there; LSP over there if the server is installed. That is TRAMP.

```
ssh://  ── one OpenSSH ControlMaster-style connection per endpoint (ssh -o ControlMaster=auto …)
        ├─ SFTP: list / read / write (atomic rename via SFTP rename; SFTPv3 codec already exists)
        ├─ exec: rg --json, git <args>, <lsp-server>  (stdio piped; no strop on the host)
        └─ exec: $SHELL  via `ssh -tt` for :terminal
local   ── in-process: std::fs, rustix-openpty, Command::new(lsp)
```

- Delete `strop-worker`, `strop-worker-client`, `strop-worker-deploy`, `strop-worker-protocol`
  (15K LOC, 4 crates), `strop-containers` (7.9K), `editor/containers`, `editor/devcontainers`
  (5.9K), `worker_catalog.rs`, the `--worker-stdio` mode, the kernel's `frame` proof (keep
  the edit-geometry proofs).
- Keep `strop-remote` SFTP (`transport/`), `hosts/` (ssh config parsing), `pool`. Replace
  `worker_transport.rs`/`deploy_provider.rs`/`bootstrap.rs` (1.6K) with a 200-line
  `exec.rs` that runs a remote argv through `ssh -- host <quoted argv>`.
- Saves: SFTP write to a temp name + rename; compare mtime/size+hash before overwrite
  (vim's "file changed" semantics). Drop the transaction directory, xattr preservation,
  lock files and `:remote verify`. Honest limitation note: no cross-process lock — same as
  every other editor.
- Risk: remote LSP needs the server on the host (already true); remote search needs `rg`
  on the host (today: worker runs `rg` anyway — "Git/LSP also need their respective programs
  in the remote workspace"). Nothing lost.
- What is lost: container browsing without a shell, dev container provisioning, the
  Lean-proved frame bound. Users relying on these: none visible (no issues, no demo tape,
  2 e2e tests).

---

## 5. Discoverability

| Surface | Finding |
|---|---|
| Welcome card | Good: `space`, `space ?`, `:w`. Nothing about `:terminal` or remote. Card is drawn **over** the command card at 30 rows [V, `bug-welcome-over-command-30rows.txt`]. |
| Which-key (`Space`) | 20 entries. No terminal, no containers, no recovery, no `:explain`, no theme. `o` says "open remote destination" — it is also the only way to "Select project root here" (which triggers dev-container discovery) and to reach Containers. |
| `Space a` | A multiplexer: retained-feedback actions + terminal actions, else filesystem actions, else LSP code actions — the picker is always titled "filesystem actions" [V, `filesystem/actions.rs:186–260`]. The if-chain order is load-bearing, which AGENTS.md itself calls over the limit. A user cannot predict what `Space a` will show. |
| `Space ?` help | 276 rows, 16 sections. `:containers`/`:container-worker`/`:container-edit` under **Documentation**. Terminal section is chords, no "how to open". "Languages" lists 13 names while docs claim 26 (LSP list vs syntax list, unlabeled). |
| `:` completion | Good: card with descriptions; 56 commands. Ex-only features: terminal (7 cmds), remote (`tail range follow unfollow browse filter remote trust`), containers (3), review (`apply/cancel/save/undo-change`), occurrence selection (4), `recover`, `explain`, `theme`, `tab-size`, `indent-style`, `search-options`, `collection`, `format`, `rename`, `symbols`, `fs`. |
| Echo row | Truncates with `…`; the dismissed/non-retained messages are gone. `:explain` retains only "owner-scoped" failures (remote-edit yes, terminal refusal no) [V]. |
| Pickers | Esc once = field Normal, Esc twice = close, and "edit query · esc close" says otherwise. Action pickers open in insert mode so `j`/`k` type into the filter [V]. |
| Recovery | No prompt on relaunch; `:recover list` only [V]. |
| Docs | `docs/remote.md` is 211 lines of caveats; the README "tour" is accurate. |

---

## 6. Bugs (repro steps; all [V] unless noted)

| # | Bug | Repro |
|---|---|---|
| B1 | `strop` (no file) → `Space ?` → `q` **exits the editor** instead of returning to the scratch buffer. With a file open, `q` returns to the file. | as stated |
| B2 | Terminal inspection cursor resets to 1:1 on buffer re-entry. | `:terminal`, output, `F2`, `/MARKER⏎` (72:123), `Ctrl-^`, `Ctrl-^` → `1:1` |
| B3 | Undo back to the saved state leaves the buffer dirty (`●`, `:qa` refuses). | headless: `keys x`, `keys u`, `state` → `"dirty":true`; `buffer.rs:61` is a plain bool, `undo.rs` never clears it |
| B4 | Rapid double Escape is swallowed. | tmux `send-keys Escape Escape` in a picker → still open; with 0.5 s between → closes. (`\x1b\x1b` decoded as Alt-Esc [S]) |
| B5 | Echo row truncates refusals; `:explain [feedback]` does not retain them. | `:terminal` in an SSH dir → `…use :ter…`; `:explain` shows only the remote-edit record |
| B6 | Welcome card paints over the command/completion card. | `strop` at 140x30 → `:te<Tab>` → rows 8–10 interleaved (`bug-welcome-over-command-30rows.txt`) |
| B7 | `:qa` with live terminals opens a picker titled **filesystem actions**. Same title for remote notices and dev-container actions. | `:terminal`, `sleep 300`, `F2`, `:e main.rs`, `:qa` |
| B8 | Container file buffer identity lost: statusline shows `[scratch] [RO]` for `container:…/etc/hostname`; directory view shows the local git branch `main`. | `:containers` → alpine → `:e etc/hostname` |
| B9 | Container listing shows `?` size and `d?????????` mode for every entry of a plain alpine container (docs promise attributes). | `:containers` → alpine |
| B10 | Collection card border leaks through the Search overlay (`╮`/`╯` fragments at the right edge). | `Space /`, `println`, `Ctrl-O`, `Space R` (`bug-collection-border-leak.txt`) |
| B11 | `strop replay --script` on a trace recorded by `strop headless … --log-file` fails: `input precedes the initial document snapshot`. README advertises this flow. | as stated |
| B12 | Every SSH/container connect emits `git discovery failed: … requires an admitted worker` as an error, unprompted. | `Space o` → host |
| B13 | Pane title `[RO]` on a live terminal in input mode. | `Space w v` with a terminal |
| B14 | `:terminal` replaces the current pane (nvim does too, but with `Space w` being the pane namespace, most users will want a split; `Space t` absent). Not a bug, logged as friction. | |
| B15 | Git discovery is not re-run after worker admission: `Space g l` on the file that triggered `:remote edit` says "not a git repo"; a file opened afterwards shows the log. | musl editor: open `notes.txt`, `:remote edit`, `Space g l` → "not a git repo"; `:e …/lib.rs`, `Space g l` → log |
| B16 | Statusline branch chip shows another namespace's branch: `main` (local) while browsing a container, `master` (the SSH repo) while browsing a different container. | `:containers` → alpine → statusline `main / [RO]` / `master / [RO]` |
| B17 | `STROP_WORKER_BINARY` is ignored for SSH deployment (R11); the documented escape hatch does not work. | gnu editor + `STROP_WORKER_BINARY=<musl>` → `no worker artifact for target x86_64-unknown-linux-musl` |
| B18 | The dev-container offer re-fires for the local `.devcontainer` while editing an SSH file, after the user already chose Review for that digest. | `.devcontainer/devcontainer.json` in cwd, open `ssh://…/lib.rs`, `Space g l` → `● Dev container configuration found` |
| B19 | Remote `:terminal` launches `/bin/sh`, not the user's login shell (`bash` per passwd). | sshd container, `:terminal` → `~/project $` ash prompt |

---

## 7. Code bloat (evidence)

From my own reading plus a read-only audit I delegated (findings re-verified by spot checks
where marked [V]).

1. **Seven job/mailbox mechanisms** [V, audit]: the `AppEvent` bounded bus
   (`events/channel.rs`, 594 lines); 51 `std::sync::mpsc` sites with one forwarder thread per
   subsystem (`events.rs:86`); `strop_core::worker` (`spawn`/`Ticket`/`Completion`, ~760
   lines, ~30 sites); four hand-written "latest value + wake" mailboxes (terminal, completion,
   notify, devcontainer); the picker's `Mutex`+2 `Condvar` worker with a watchdog; strop-lsp's
   private tokio runtime bridged over mpsc; and the worker-client/remote transports' reader
   threads. ~40 raw `thread::spawn` outside tests bypass `strop_core::worker`.
2. **Location types** [V, audit]: `ResourceLocation` (568 refs) is canonical, yet
   `DocumentSource::{File,Container,Remote}` re-encodes the same facts (170 refs), container
   identity has three types (`ContainerId`, `ContainerIdentity{id: String}`, `ContainerRef`),
   plus `RemoteFile`, `RemoteLocation`, `WorkspaceKey`, `FileLocation`, `NativePath`.
3. **22 failure enums** across remote/worker/fs/containers, glued by 17 `-> FsFailure`
   adapters; six byte-identical `fn failure(kind, detail) -> FsFailure` copies [V, paths in
   audit]; `io_failure` duplicated in `strop-fs/src/lib.rs:30` and `strop-worker/src/serve.rs:70`
   with diverging `DirectoryNotEmpty` mapping [V].
4. **Dead or test-only public API** [V]: `Session::write_stdin`, `is_relayed`,
   `token_cancelled` (worker-client), `requires_consent`, `features_json` (devcontainer) have
   zero callers; a dozen more are used only by tests.
5. **Untested ex commands** [V, audit]: `terminal-paste-cancel`, `unfollow`,
   `search-options`, `indent-style`, `containers`, `h` have zero test mentions.
6. **Dual read paths** per namespace [V, `namespace.rs:176`]: SFTP vs worker, `docker cp` vs
   worker; engine calls `strop_fs` directly *and* through the in-process worker.
7. **Test scaffolding in `src/`**: `editor/conformance.rs`, `contract_probes.rs`,
   `transaction_conformance/` (~2K lines) are `#[cfg(test)]` — fine, but they belong under
   `tests/`.
8. **Oversized files** past the ~800 ceiling AGENTS.md sets: `strop-remote/worker_transport.rs`
   788, `transport/wire.rs` 775, `devcontainers/ui.rs` 775, `strop-containers/devcontainer/config.rs`
   779, `exec.rs` 730 — all in the remote stack.
9. **Config knobs**: only 10, all sensible [V `:explain`]. Not a bloat source.
10. **CLI**: `--tail/--range/--follow` + `:tail/:range/:follow/:unfollow` (8 surfaces for log
    reading); `--dump-compat`/`--replay-script` legacy flag spellings kept "for muscle memory"
    on a 0.x product.
11. **Repository**: `.claude/worktrees/*` contain full copies of the tree (found by grep;
    untracked). Not shipped, but they double every repo-wide search.

Positive: zero `#[allow(dead_code)]`, 8 `#[allow]` total, ~20 non-test `unwrap`s in a 98K
LOC crate — the discipline rules are being followed. The bloat is architectural, not
hygienic.

---

## 8. Sequenced proposal

### 0.55 — cut and fold (one release)

1. **Delete containers and dev containers**: `strop-containers`, `editor/containers`,
   `editor/devcontainers`, `:containers`, `:container-worker`, `:container-edit`,
   `Containers…` in `Space o`, the devcontainer offer, `container_editor` e2e, the
   `container-test` compose service. −15K LOC, −1 crate, −2 CLIs. Tell users: use
   `:terminal` → `docker exec -it`.
2. **Terminal flow** per §3.4: `Space t`/`Space T`, nvim chord only, live buffer, cursor
   retention, `TERMINAL` chip, `:qa` notice. Delete held-paste review, refresh, F2/Ctrl-4,
   `:terminal-stop`. Local PTY in-process.
3. **Fix B1–B19.** B3 (dirty after undo) and B4 (double Esc) are fidelity bugs and should
   get differential-harness cases.
4. **Echo row**: never truncate — wrap to a 2–3 line echo area or open the message as a
   card; retain the last 50 messages in `:explain [messages]`; add `:messages`.
5. **Recovery prompt on open** when a draft exists for the file (vim `ATTENTION` baseline).
6. **Help**: move container rows out (deleted anyway); give Terminal and Remote "how to
   open" first rows; label Languages as "LSP servers".
7. **`Space a`** becomes one list built from typed owners (data, not if-chain): code
   actions when LSP has some, plus remote/terminal/filesystem entries valid for the buffer;
   title it "actions".
8. **STATUS.md**: retire 0037, 0058's deploy slices, 0065's snapshot clauses.

Risk: a user attached to container browsing. Mitigation: the terminal plus `docker exec`.
Nobody loses SSH.

### 0.56–0.57 — rework

1. **Remote simplification** (§4.4): remove the worker protocol and deploy pipeline;
   `ssh exec` for `rg`/`git`/LSP/shell; SFTP write for saves. Fold `:remote worker` and
   `:remote edit` into connect (a connected host is editable; read-only stays `-R`/`:view`).
   −20K LOC, −4 crates. Keep the Lean kernel for edit geometry; retire the frame proof with
   its mutants (0086 survival rule: a proof about deleted code is not about shipped code).
2. **Filesystem workspace**: collapse Review→Apply→Receipt→Verify→Undo to oil.nvim's
   model: edit the directory buffer, `:w` shows the diff, confirm, done. Drop `:fs verify`,
   `:fs operations`, `:fs undo`, receipts.
3. **One job mechanism**: `strop_core::worker` + the `AppEvent` bus; delete the four
   mailboxes and the forwarder threads.
4. **One location type**: `ResourceLocation` everywhere; delete `DocumentSource::Container`
   (gone) and `DocumentSource::Remote`'s duplicate path.
5. Drop `--tail/--range/--follow` once `strop -` (stdin) exists; keep `:e ssh://…`.

### Do not

- Do not remove the terminal; the engine is right and `:terminal` is table stakes for the
  nvim audience.
- Do not remove SSH editing; it is the one remote feature with a story, and SFTP read-only
  works well today.
- Do not touch the grammar, pickers, search workspace, git surfaces, LSP, completion,
  syntax; they are the product and they tested clean here.
- Do not add a plugin runtime or tabs to compensate (0001 non-goals still hold).
- Do not keep "old way / new way" pairs during the remote rework (AGENTS.md): the SFTP
  read path becomes the only read path in the same change that deletes the worker.

---

## Appendix — evidence files

`/tmp/claude-1000/-home-tarek-workspace-strop/1b68bfc9-64e0-4443-ae53-32722ddac200/scratchpad/bug-welcome-over-command-30rows.txt`,
`…/bug-qa-filesystem-actions-title.txt`, `…/bug-collection-border-leak.txt`, `…/trace.jsonl`,
`…/undo.keys`, `…/strop-musl` (the 0.54.0 musl release used as the deployable worker/editor).
Containers `stropcrit-sshd` and `stropcrit-box`, their images and the dedicated tmux server were
removed after the run; no files in the repository were touched.
