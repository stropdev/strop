# 0034 — SSH log buffers

Status: implemented for the 0.16.0 snapshot milestone; publication follows the gates below.

## Research and decision

TRAMP is much broader than remote reads: remote filenames, editing/saving, directory
browsing, multiple transports, multi-hop connections and remote process integration.
Its [quick start](https://www.gnu.org/software/emacs/manual/html_node/tramp/Quick_002dStart-Guide.html)
uses `/method:user@host:/path`. Strop should not claim to replace that contract.

For the requested open/search-log workflow, use `ssh://[user@]host[:port]/absolute/path`.
The URI names a real read-only buffer, not a preview widget or a local cache file.
OpenSSH supplies aliases, Include/Match rules, keys, agents and ProxyJump. A bounded,
read-only SFTP v3 codec runs over an owned `ssh -s ... sftp` connection. It implements
OPEN, FSTAT, READ and CLOSE, not SSH authentication or encryption.
No remote shell command is assembled from a filename. No SSH/TLS implementation,
remote shell assumptions, openssl dependency or required daemon is added to strop.

The useful advantage over a general remote editor is scope: a visible, cancellable
read, honest errors, no remote language discovery, and the same local resolver and
navigation once loaded. This is not a measured performance claim against Emacs.

The initial `openssh-sftp-client` evaluation found that its protocol crate serializes
paths through serde's UTF-8-only `Path` implementation. That cannot meet native-byte
identity. A vendored dependency patch would also break registry publication semantics.
The selected reader therefore encodes native path bytes directly, caps every response
before allocation, validates request IDs/statuses/lengths, and tests against OpenSSH's
real server. No wire-rewriting shim or guessed filename is used.
The file protocol follows [SFTP v3](https://www.ietf.org/archive/id/draft-ietf-secsh-filexfer-02.txt);
transport/security options follow [ssh(1)](https://man.openbsd.org/ssh.1).


## User contract

- `strop ssh://devbox/var/log/app.log`, `strop +120 ssh://devbox/var/log/app.log`,
  `:e ssh://devbox/var/log/app.log`, `:view` and path-bearing splits share the loader.
  `FILE:LINE` remains local CLI syntax; remote colons belong to the URI. Encode literal
  reserved bytes in paths (`%20`, `%23`, `%25`); Unix native filename bytes survive.
- Absolute remote paths only; no ambiguous scp shorthand, passwords in URIs, query
  strings or fragments. Validated private address fields; malformed input fails
  before spawning. `--` keeps its literal local-filename meaning.
- OpenSSH must be installed locally and the host must offer SFTP. Authentication is
  noninteractive (existing keys/agent/config). Unknown/changed host keys are refused,
  never auto-accepted. The error explains establishing trust/authentication with SSH
  outside strop. A dedicated connection disables TTY, forwarding and backgrounding;
  no multiplexed master is created or killed by cancellation.
- A successful read captures the byte length reported by the opened handle. Appends
  during the read are excluded. A short/truncated read, invalid UTF-8, non-regular
  file or a snapshot exceeding 256 MiB is an explicit error, not partial success.
  Concurrent rewrites are not an atomic filesystem snapshot; do not claim otherwise.
- Fetches leave the current buffer usable. Escape cancels a pending navigation;
  superseding it, closing its owner, moving focus or quitting cannot publish stale
  data. Every child/process group is signalled and reaped off input/render. Connection
  and transfer have a bounded deadline; stderr is drained with bounded retention.
- Loaded buffers support existing motions, incremental `/`/`?`, `n`/`N`, visual
  selection, yank, splits and local horizontal scrolling. URI identity is not a local
  filesystem path. Modeline identifies the SSH source and read-only state; local Git
  and LSP discovery do not attach to the remote file.
- Remote snapshots cannot become writable with `:set noro` or be saved with `:w!`.
  Reopening a loaded URI selects it; `:e!` without a path explicitly refreshes a remote
  snapshot, preserving position and keeping old content if the refresh fails.
- No automatic on-disk content cache/session restoration. Opt-in full-content tracing
  may contain remote data just like local data; metadata-only tracing does not.
  Recorded opens replay without invoking SSH/network.

## Implementation boundaries

1. Extract the existing Unix child/group ownership from shell jobs into
   `strop_core::process::OwnedProcess`: spawn with a cancellation hook installed
   first, retain the unreaped PID until signalling is revoked, take pipes, observe
   exit without reaping, terminate, wait and RAII cleanup. Existing shell jobs migrate
   to it. A bounded capture helper serves `ssh -G`; no unowned configuration subprocess.
2. `strop-remote` is a separate library crate: validated `RemoteFile`/`AddressError`,
   owned SFTP reads and descriptive `ReadStage`/`ReadFailureKind`/`RemoteReadError`.
   `crates/strop-remote/src/address.rs` owns URI/serde; `transport.rs` and `transport/`
   own the worker-local runtime, v3 codec and diagnostics. The crate depends on core
   workers/buffers, not Editor, CLI or rendering. No generic provider framework yet.
   Its read API consumes `&RemoteFile, &CancelToken` and returns
   `Result<Buffer, RemoteReadError>`; successful buffers have no local path.
3. `DocumentSource::Remote(RemoteFile)` supplies provenance. Extend the existing
   open ticket/result/codec path with a typed local-or-remote target, retaining origin,
   revision, focus and cancellation checks; no second competing navigation machine.
   Split the I/O module by load/save responsibility if integration exceeds its ceiling.
4. CLI startup, ex commands, rendering and replay consume these same boundaries.
   Local `PathBuf` callers remain explicitly local; URI parsing happens only at textual
   user-entry boundaries. No filename display string becomes an identity.
   The application-specific local-or-remote `FileTarget` stays in `strop/src/files.rs`;
   its remote wire envelope cannot be confused with legacy local path strings.

## Acceptance and proof

- Hermetic URI, rejection, native-byte and read-only/stale-publication regressions.
- Isolated localhost sshd/SFTP fixture with private keys and known_hosts; no real HOME
  or external network in tests. Exercise an alias, port, spaces/metacharacters, Unicode,
  missing file, failed authentication, host-key rejection, bounded/truncated read and
  cancellation. No wall-clock sleeps; synchronization by pipes/events where needed.
- Actual CLI and TUI open/search/yank/refresh/split/quit; display-cell capture confirms
  remote identity. Full recorded trace replays successfully with SSH unavailable.
- Docker fmt/Clippy/tests and protocol model gate pass; static dependency graph retains
  the rustls-only/libgit2 no-default-features rule. Release platforms remain Linux/macOS;
  unsupported process-supervision platforms report an explicit error.

## Prioritized extensions, not silently implied capabilities

- The user promoted P2 ranges/tails/follow, addressing/completion, connection reuse,
  read-only directory browsing and remote LSP/Git into the next delivery: [0036](0036-remote-workspace-execution.md).
- Remote writes, mutations and additional transports remain explicitly prioritized
  in [0035](0035-remote-workflow-roadmap.md). Dev Container provisioning is a
  complementary later layer in [0037](0037-devcontainers-and-workspace-contexts.md).

## Formal verification requested during implementation

Use the existing TLC Docker gate, with two bounded models rather than one oversized
cross-product: `SftpWire` for request identity, framing and snapshot bounds;
`RemoteRead` for view ownership, single terminal outcomes, cancellation and child/PID
lifetime. Check fairness-qualified eventual completion/reaping separately from safety.
Keep deliberate reply-ID, over-read, stale-publication, duplicate-terminal and PID-reuse
mutants; tool/parser failures must not count as successful mutation kills.

Every model invariant needs a named executable oracle over the real Rust boundary.
TLC is bounded exploration of an abstraction, not a proof of Rust byte decoding,
OpenSSH cryptography, arbitrary server behavior or OS progress. State the explored
bounds and scheduling/OS assumptions with the observed results. Follow rootle's
[provider verification approach](https://rootle.dev/docs/provider-protocol.html#process-lifecycle--what-rootle-assumes-about-your-adapter).

The complete remaining TRAMP-style workflow is tracked in 0035; this tranche does
not silently grow into remote writes, command execution or additional transports.

## Observed snapshot-milestone verification

- Real OpenSSH over an isolated localhost TCP port opened/search/yanked a log and a
  byte-native filename. The actual TUI exercised cancellation of a controlled stalled
  SSH child (socket EOF confirmed descendant death), refresh, split and clean quit.
  Its complete full-content capture replayed successfully.
- The Docker gate runs private sshd inetd fixtures, with no real HOME or network:
  native/metacharacter/newline filenames, multi-frame reads, auth and host-key
  refusal, missing file, invalid UTF-8, oversize rejection, and replay with SSH disabled.
- `SftpWire`: 78 distinct states in the honest bounded configuration.
  `RemoteRead` safety: 7,020,664 distinct states, two requests/two PID values,
  focus/revision/incarnation caps of two. Qualified progress: 46,762 states, two
  competing requests/one reusable PID, focus/incarnation caps one and revision zero.
  Progress projects out unrelated editing variation; the broader safety run retains it.
- Seven faulty modes were rejected by the intended predicate: wrong reply ID,
  over-read, unbounded allocation, stale publication, duplicate terminal delivery,
  released-PID signalling and skipped cleanup. Cleanup also violates the isolated
  `EventuallyReaped` property. Separate witnesses reach success, UTF-8 failure,
  admission/publication, cancelled-child cleanup and stale-success rejection.

| Model property | Executable boundary evidence |
|---|---|
| ReplyIdentity | `wire_tests::a_reply_for_another_request_cannot_supply_a_file_handle` |
| PacketBounded | `wire_tests::packet_bound_is_checked_before_reading_or_allocating_payload`; nested/trailing frame rejection |
| SnapshotBounded | `wire_tests::snapshot_length_is_validated_before_content_allocation`, over-read and early-EOF tests |
| CompleteBeforeSuccess | `wire_tests::failed_close_cannot_complete_a_snapshot`, invalid-UTF-8 rejection, real SSH read/replay |
| FreshPublication / CancellationWins | `editor::io::remote_tests` Escape/supersede/focus cases; controlled real TUI stall/cancel |
| TerminalOnce | Core worker cancellation/terminal reservation tests and cancelled service completion rejection |
| NoUnownedChild | Shared shell descendant cancellation test, real SSH fixture exit, TUI descendant socket EOF |
| SignalOwnedPid | Core `OwnedProcess` mutex/revoke-before-reap invariant plus process cleanup tests; PID reuse interleavings are model-checked, not claimed to be forced in the OS fixture |
| ReadonlyPublication | Remote search/yank plus `:set noro`, deletion and `:w!` refusal regression |

These observations check the stated boundaries. They are not a Rust refinement proof,
an atomic remote-filesystem snapshot, or a guarantee of OS progress during failure.
