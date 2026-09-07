# 0036 — Remote workspace delivery: P2, LSP and Git

Status: accepted for implementation. The user expanded 0034/0035 while work was in
progress: ship RW1, RW2, RW3, remote LSP and remote Git semantics, update the website,
and publish the complete release. This supersedes the narrower snapshot-only scope.

## Scope and ordering

- **RW1 (P2):** byte ranges, bounded tails and follow mode with append/reset handling.
- **RW2 (P2):** remote home expansion and host/path completion without surprise auth.
- **RW3 (P2):** shared owned connections, explicit cleanup and reconnect incarnations.
- **RW5 read-only portion:** remote directory browsing as a real buffer: open entries,
  parent navigation and search/filter. Rename/delete/copy remain behind RW4.
- **RW7 prerequisite:** one owned remote-process boundary for Git and language servers.
- **RW8:** remote diagnostics/hover/navigation and the existing read-oriented Git
  surfaces (context/status, worktree/index diffs, log, blame, commit/file navigation,
  source links). A remote target must never touch the analogous local path.
- Remote buffer writes, Git mutations and other transports stay behind the explicit
  RW4/P3 safety gates. They are refused by capability, not routed to a local fallback.
  Full remote reads remain readonly; remote LSP does not imply enabling remote writes.
- Docker Rust gates, bounded models and real SSH/TUI/replay evidence precede hosted
  CI, versioned publication and the website update. No framework or stub-only release.

## Correctness decisions

### Address and document identities

`strop-remote` owns validated `RemoteEndpoint`, unresolved `RemoteLocation`, canonical
absolute `RemoteFile`, byte offsets/limits, transport/session ownership and process
launch descriptions. Endpoint fields are private and validated. An unresolved home
path is not an absolute path: resolve it through a negotiated SFTP extension, then
publish the canonical file identity. `%7E` can name a literal tilde directory without
silently turning into a home query. Native Unix path bytes survive every crossing.
The editor owns `FileTarget`, buffer/view capabilities, prompt ownership and replay.

### Pooling

A `RemoteClient` is cheap, cloneable, and pure to construct. On workers it acquires a
per-endpoint session actor. The manager retains weak references; a `ConnectionLease`
keeps a session alive for a document or explicit user connection. Last-lease release
signals shutdown without waiting on the input thread. Each actor owns its child,
runtime and monotonically numbered SFTP requests. Requests are serialized; no shared
mutable codec or stdin writer can be borrowed by two jobs.

Cancellation of an active exchange retires that physical connection and its epoch.
It does not replay that request silently. Other queued requests remain owned and may
start on a new connection; late bytes from the retired one cannot satisfy them.
Queued cancellation must not disrupt an active unrelated request. Explicit disconnect
invalidates the connection, and last-document cleanup cannot kill a user's external
SSH master. Default SSH safety options remain strict and dedicated.

### Ranges and follow

`ReadSelection` is typed: Full, Range(start, limit), Tail(limit). A bounded tail is
also follow's initial/current window. CLI: `--tail BYTES`, `--range START:BYTES`,
`--follow` with a remote operand; `+LINE` addresses the loaded window's line space.
Ex: `:tail [BYTES] URI`, `:range START BYTES URI`, `:follow [URI]`, `:unfollow`.
Defaults and limits have named byte-domain constructors; invalid numbers are errors.
Modeline/window metadata must say when content is partial and which bytes it shows.

Follow polls on owned jobs, not keystrokes. Reopen the pathname when polling: an old
handle alone cannot detect rename/rotation. SFTP v3 has no portable inode/generation
field, and size+mtime is not identity. Compare bounded overlapping content before
claiming append continuity; otherwise publish an explicit reset/replacement. A shorter
size is a shrink/reset, not proof of a particular filesystem operation. Same-size,
same-mtime replacement must still be observed. Empty files, long lines, partial UTF-8
at a tail boundary, truncation, eviction and cancellation get explicit behavior.
Cursor sticks to EOF only while the user is following EOF; browsing/searching pauses
that stickiness. Escape stops following and leaves the last real buffer intact.

Follow publications use an owned rope/snapshot mutation in strop-core, preserving
revision/selection invariants without materializing the whole rope on input/render.
Partial windows never masquerade as full LSP documents or full-file Git hunk inputs.

### Completion and homes

Host candidates come from local config/known-host/history data on a worker. `ssh -G`
resolves an already chosen host; it is not a host enumerator, and Match exec must not
run merely for completion. Remote path completion can use an already authorized live
connection; otherwise show cached candidates or require explicit connect. Completion
never opens an authentication prompt. Every result carries the prompt's owner and
current input identity and is rejected after edits, cancel, focus change or close.

Home expansion uses `expand-path@openssh.com` when advertised. SFTP REALPATH alone
does not expand tilde; lack of the extension is a typed refusal, never a guessed home.

### Remote execution and services

SFTP remains shell-independent. Remote process/LSP/Git services require a POSIX remote
execution environment and fail explicitly when the required tools are absent. One
`RemoteCommand` owns executable, native argv and absolute remote cwd; filenames are
quoted as inert argv, never interpolated as executable shell text. The SSH transport
and remote process group have an explicit shutdown contract; local ssh death alone
must not be claimed to prove all remote descendants were reaped. The implementation
must use a supervised remote command lifecycle or state and test its exact limits.

The remote filesystem and process/workspace identities are separate from SSH wire
details. A future container backend and Dev Container lifecycle adapter reuse those
boundaries, including LSP/Git routing and incarnation-based stale-result rejection.
Do not add unimplemented container enum variants or a provider framework now.

Language server configuration keeps valid layering and visible diagnostics. Local
workspace configuration, filesystem probes and project paths must not be applied to
a remote workspace. Server positions/URIs describe the remote filesystem, while the
editor maps them back to that endpoint's file identities. Requests retain server,
document incarnation, revision and encoding ownership. Partial/follow windows refuse
full-document language services. Closing the last owning workspace retires its server.

Git queries run against the remote worktree with native path/record parsing. Local
libgit2 stays local. Surface provenance carries endpoint + repository + revision;
diving from log to files to diff to a file keeps that provenance. Shell output is
bounded and cancellable. Source links evaluate SSH aliases on workers and pin the
correct revision. Unsupported/mutating operations report capability refusal, never
silently mutate the local cwd or an unversioned remote file.

## Cross-slice contracts

- Remote client API: `RemoteClient::new()`, `read(&RemoteLocation, ReadSelection,
  &CancelToken) -> Result<RemoteSnapshot, RemoteReadError>`; `RemoteSnapshot` owns
  canonical `file`, readonly `buffer`, typed `window`, and a `ConnectionLease`.
- `ReadSelection::Full`, `Range { start: RemoteOffset, length: ReadLimit }`,
  `Tail(ReadLimit)`; `RemoteOffset::new(u64)`, `ReadLimit::new(u64)` checked against
  memory limits, `ReadLimit::DEFAULT_TAIL`. Window metadata exposes start, length,
  whole-file size and `is_complete()` without raw-unit confusion.
- `RemoteClient::connect(endpoint, token) -> ConnectionLease`, `disconnect(endpoint)`
  and `disconnect_all()` worker-side; `list_connected(directory, token)` never
  authenticates or creates a new connection. `expand_home` is folded into read.
- `RemoteClient::open` may return a typed file or directory resource for explicit user
  opens. `list` may connect only after an explicit browsing/open action; completion
  continues to use the non-authenticating `list_connected` path.
- `RemoteEndpoint` has `host()`, `user()`, `port()`; `RemoteFile` has `endpoint()`,
  `path()`, `with_path(PathBuf)` checked; `RemoteLocation::parse`, `endpoint()` and
  `absolute_file()` distinguish unresolved entry from a canonical file.
- Remote process API: `RemoteCommand` with checked executable/native argv/cwd,
  `command(endpoint, command) -> std::process::Command` for an owned stdio client,
  and `run(endpoint, command, cancel) -> Result<CommandOutput, RemoteCommandError>`
  for bounded finite commands. No process launches in constructors.
- Main owns Editor top-level/IoState/AppEvent/trace integration and CLI/commands.
  `Editor::remote_file()` remains the canonical source accessor;
  `remote_window_complete()` gates LSP/full-file Git;
  `remote_client()` gives a cheap clone for worker inputs.
- LSP/Git owners may add their subsystem modules and types, but report changes
  needed in Main-owned fields/event plumbing rather than race edits to those files.
- All agents skip validation while source changes are concurrent. Main runs the
  integrated gates and exercises the real surfaces after integration.

## Acceptance and release

1. Actual SSH fixtures: reuse, queued/active cancellation, reconnect, last-owner
   cleanup; byte-native names, range/tail UTF-8, append/shrink/same-size replacement,
   completion staleness and no-auth behavior.
2. Remote LSP fixture emits diagnostics and cross-file locations; the opened target
   has the same endpoint, correct positions, and no corresponding local-file access.
3. A remote-only Git repository exercises context, log/blame/diff/dive/source links;
   mutations cannot fall through to local Git. Ordinary local Git/LSP remain green.
4. Model-check connection epochs/ownership and follow publication alongside the
   repaired SftpWire/RemoteRead models, with non-vacuity witnesses and meaningful
   mutants. Rust oracles exercise the corresponding real boundaries.
5. Docker fmt/Clippy/tests, static release checks, hosted CI, real terminal use and
   replay pass. Update README/changelog, roadmap status and site docs/demo/capability
   descriptions to match exercised behavior. Release through the repository workflow,
   with true topological crate publication order including strop-remote.
