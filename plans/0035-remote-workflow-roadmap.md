# 0035 — Remote workflow roadmap: the full TRAMP-style capability inventory

Status: roadmap with implemented milestones. 0034 shipped the read-only SSH snapshot
reader in 0.16.0; 0036 delivers ranges/follow, addressing/completion, pooled connection
ownership, read-only directory browsing, supervised execution and remote LSP/Git in
0.17.0. This inventory orders the remaining TRAMP-style capabilities with dependency
gates and observable acceptance; it sets no dates or blanket parity promise. Remote
writes, arbitrary shell jobs, additional transports and automatic content restoration
remain outside the delivered scope.

## Method and sources

The [TRAMP manual TOC](https://www.gnu.org/software/emacs/manual/html_node/tramp/index.html)
and the configuration/usage chapters behind it were read and are paraphrased below, not
copied: quick start, [connection types](https://www.gnu.org/software/emacs/manual/html_node/tramp/Connection-types.html),
[inline](https://www.gnu.org/software/emacs/manual/html_node/tramp/Inline-methods.html)
and [external methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/External-methods.html),
[GVFS](https://www.gnu.org/software/emacs/manual/html_node/tramp/GVFS_002dbased-methods.html)
and [FUSE methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/FUSE_002dbased-methods.html),
[defaults](https://www.gnu.org/software/emacs/manual/html_node/tramp/Default-Method.html),
[multi-hops](https://www.gnu.org/software/emacs/manual/html_node/tramp/Multi_002dhops.html)
and [ad-hoc hops](https://www.gnu.org/software/emacs/manual/html_node/tramp/Ad_002dhoc-multi_002dhops.html),
[firewalls](https://www.gnu.org/software/emacs/manual/html_node/tramp/Firewalls.html),
[passwords](https://www.gnu.org/software/emacs/manual/html_node/tramp/Password-handling.html),
[connection caching](https://www.gnu.org/software/emacs/manual/html_node/tramp/Connection-caching.html),
[remote programs](https://www.gnu.org/software/emacs/manual/html_node/tramp/Remote-programs.html),
[ssh setup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Ssh-setup.html),
[remote processes](https://www.gnu.org/software/emacs/manual/html_node/tramp/Remote-processes.html),
[cleanup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Cleanup-remote-connections.html),
[renaming](https://www.gnu.org/software/emacs/manual/html_node/tramp/Renaming-remote-files.html),
[backup/lock/auto-save](https://www.gnu.org/software/emacs/manual/html_node/tramp/Auto_002dsave-File-Lock-and-Backup.html),
[encryption](https://www.gnu.org/software/emacs/manual/html_node/tramp/Keeping-files-encrypted.html),
[archives](https://www.gnu.org/software/emacs/manual/html_node/tramp/Archive-file-names.html),
[file name completion](https://www.gnu.org/software/emacs/manual/html_node/tramp/File-name-completion.html),
[home expansion](https://www.gnu.org/software/emacs/manual/html_node/tramp/Home-directories.html),
the developer chapters ([temporary files](https://www.gnu.org/software/emacs/manual/html_node/tramp/Temporary-directory.html),
[extension methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/Extension-packages.html),
[new operations](https://www.gnu.org/software/emacs/manual/html_node/tramp/New-operations.html))
and [tracing](https://www.gnu.org/software/emacs/manual/html_node/tramp/Traces-and-Profiles.html).
Full source list at the end. TRAMP's own performance statements are qualitative
(inline encodes through the shell, external methods spawn transfer programs); they are
reported as design facts, never as measurements of strop.

## Doctrine: what stays fixed while the capability grows

1. **One static binary, zero required setup (0002).** SFTP reading needs local OpenSSH;
   optional remote Git/LSP need a POSIX remote, `python3` and their respective tools.
   Future slices may use optional providers (`rsync`, `docker`, `kubectl`, `rclone`,
   …), detected at runtime, never bundled or required for local editing. An absent provider fails with a
   typed error naming the missing executable and the feature that wanted it. Absent
   capability never degrades into guessed behavior (no scp-shorthand guessing, no
   silent fallback transport, no partial success).
2. **OpenSSH is the SSH security boundary.** Aliases, `Include`/`Match`, keys, agent,
   `ProxyJump` and host-key policy remain in OpenSSH; bounded `ssh -G` resolves chosen
   hosts for links, not candidate enumeration. Future non-SSH providers use their own
   established authentication mechanisms under the same privacy/ownership gates.
   No second configuration system may silently override an existing provider's policy.
3. **Every remote operation is an owned, cancellable job.** 0034's publication,
   cancellation, deadline and child-reaping rules are the template for all later
   slices, not a special case for reads.
4. **No magic-filename layer.** TRAMP's transparency rests on Emacs's file-name-handler
   mechanism: every Elisp package inherits remote semantics without knowing about
   them. Strop deliberately has no extension runtime (0001 non-goal), so "transparent"
   is the wrong target. The honest target is *uniform*: every remote surface gets the
   same grammar, search, cancellation and honest errors as local ones, because it is
   the same buffer/picker/job machinery — but each capability is wired explicitly.
   Emacs-only integrations with no strop analogue are listed in the matrix with their
   disposition rather than dropped silently.
5. **Unsafe legacy stays non-default.** `rsh`, `telnet`, `ftp`, and clear-text
   `nc`-style transfers appear in TRAMP for host archaeology (routers, NAS boxes). The
   manual itself calls `rsh` obsolete and `telnet` equally insecure. In strop they are
   named non-goals unless an explicit opt-in slice is ever argued: never default,
   never silent, configuration-gated with a warning stating the exposure.
6. **Priorities follow 0028's definitions**: P1 = correctness/privacy/daily editing,
   P2 = structural/performance quality, P3 = optional capability. This program is
   capability expansion relative to a working local editor, so its slices are P2/P3;
   the P1-class obligations (credential privacy, stale-data exclusion, isolation)
   appear as gates *inside* every slice rather than as separate backlog rows. A defect
   against already-shipped behavior stays P1 regardless of this roadmap.

## Capability matrix

TRAMP capability (paraphrased from the manual) → strop disposition. "0034" names the
0.16 snapshot foundation; "0036" names the 0.17 read-only workspace delivery. "RW*n*"
is the slice below; "non-goal" is excluded, revisitable only through a new plan.

| TRAMP capability | What TRAMP does | Strop disposition |
|---|---|---|
| Remote file name syntax `/method:user@host#port:/path` | One syntax spans all methods; parts optional with defaults | `ssh://[user@]host[:port]/abs/path` plus negotiated `~/` and `~user/` homes (0036); no passwords in URIs. Multi-method syntax waits for a second provider |
| Editing and saving remote files | Buffer saves back over the same connection | Read-only workspaces (0036); write path is RW4, safety-gated |
| Inline transfer (`mimencode`/`uuencode`/perl fallback over the login shell) | File contents tunneled through the shell connection; compression above a size threshold | Non-goal: strop's SFTP channel is binary-clean; shell-tunneled encodings reintroduce quoting/robustness problems TRAMP needed for dumb hosts |
| External transfer (`scp`, `rsync`, `pscp`, `rcp`, `nc`, `fcp`) | Dedicated transfer program per copy; small files stay inline below a size limit | `rsync`/`scp` as optional providers for bulk/dir sync (RW11); `rcp`/`nc`/`fcp` non-goals (legacy/obsolete) |
| `sftp` GVFS method, `psftp` | SFTP via GVFS/PuTTY | Native bounded SFTP v3 codec (0034) is strop's equivalent, without GVFS/D-Bus or PuTTY |
| GVFS methods (afp, dav/davs, gdrive, mtp, nextcloud) and GNOME Online Accounts | Desktop-integrated mounts via D-Bus; credentials in Online Accounts | RW11 optional desktop/cloud adapters, or rclone where it covers the storage. D-Bus is not inherently incompatible with a static client, but a desktop service must never become a required dependency |
| FUSE methods (`sshfs`, `rclone`) | TRAMP mounts via FUSE and mostly reuses local file ops | `sshfs`/`rclone` as explicit opt-in providers (RW11): strop may *use* an existing mount via local paths and may offer to mount; it does not manage daemon lifetimes invisibly |
| Containers (docker, podman, kubernetes, toolbox/distrobox, flatpak, apptainer, nspawn; ELPA: incus, lxc, lxd) | `exec` into containers as a "host"; `*cp` variants for transfer | RW11, staged after lifecycle work: same URI shape, `docker/podman exec` + `cp` providers; no user semantics where the tools have none |
| Privilege methods (`su`, `sudo`, `sudors`, `surs`, `doas`, `run0`, `sg`, `ksu`, `androidsu`) | Change identity locally or as the last hop; sessions time out (~5 min) for security | RW10, sudoedit-shaped (per-command, no resident privileged session) preferred over persistent sessions; timeout semantics preserved |
| `sudoedit` method | Every file op is a separate `sudo` command; no background session; localhost only | RW10's model, adopted for safety before any persistent-escalation option |
| `sshx`/`scpx`/`plink`/`plinkx`, `krlogin` | Client/platform and Kerberos integration | RW11 platform/auth adapters, separately verified. WSL is not evidence of native Windows/PuTTY support; remote Windows file/process semantics require explicit coverage |
| `smb` method (`smbclient`), UNC paths | Share paths, domain-qualified users and per-share auth | RW11 optional SMB adapter with typed share identity and its own credential/context contract |
| Multi-hop/proxy rules and ad-hoc hop chains; restricted-shell hosts | Bastions, cascaded proxies and remote elevation | OpenSSH ProxyJump/ProxyCommand covers SSH routing now. Typed composed contexts and a discoverable hop UI remain planned with RW10/RW11; do not conflate routing with privilege escalation |
| Firewalls (HTTP CONNECT tunnels) | ProxyCommand with netcat; PuTTY built-in proxy | ssh config territory, same as above; strop adds nothing |
| Passwords: auth-source, memory cache/expiry, save-on-success | Reuse credentials across connections | RW9 integration with SSH agent/askpass and provider/system credential stores; no plaintext editor password database or secrets in traces |
| Connection property persistence | Cache remote facts across sessions | RW12 opt-in persistence with endpoint/incarnation provenance and invalidation |
| Host/user completion from config, known_hosts and history | Complete without surprise connections | RW2 implemented (0036): local candidate enumeration and READDIR on an authorized connection; ssh -G is not an enumerator |
| `~`/`~user` home expansion | Method-specific expansion | RW2 implemented (0036): negotiated expand-path@openssh.com; unsupported expansion is refused |
| Directory browsing (`dired`), two-argument file ops (`copy-file`, `rename-file`) across local/remote | Transparent directory editing and mixed operations | Read-only browsing/search/filter implemented (0036); mutation and mixed operations remain RW5/RW6 |
| Direct remote-to-remote copying (`scp -R/-3` under strict conditions) | Avoid the local relay hop when host keys and auth permit | RW6, opt-in with TRAMP's preconditions made explicit checks |
| Remote processes (`shell`, `eshell`, `compile`, `gdb`, `grep`; `INSIDE_EMACS`; direct-async mode) | Commands run where `default-directory` points; direct-async trades interactivity for startup cost; no signals/remote-pid in that mode | RW7/RW8: remote shell jobs, remote grep→picker, build/formatter/LSP/debugger. Every remote command is user-issued and owned; no command is assembled from a filename. Direct-async-style dedicated channels keep 0034's noninteractive rule |
| Remote program discovery (`ls`, `test`, `find`, `cat` required; `perl`/`grep` accelerators; `getconf PATH`) | Probe and cache the remote environment | RW7: minimal probing, cached per connection incarnation (RW3), honest failure when a needed tool is missing |
| Auto-save, file locks, backups; root-file backup exposure warnings | Configurable per connection; warns when root-owned files would land user-readable | RW12: no remote backups/locks by default; the TRAMP-documented exposure (root file → user-owned backup elsewhere) is the reason. Local staging of remote content, if ever added, is explicit and private |
| Renaming remote buffers (`tramp-rename-files`) | Rehome buffers to another host when networks change | RW3/RW12: reconnect isolation first; explicit rehome command only if field use asks |
| Cleanup (`tramp-cleanup-*`) | Flush connections, caches, passwords, buffers | RW3 implemented (0036): `:remote` lifecycle subcommands; last document/job/explicit pin releases its owned connection |
| Encryption of remote trees (`encfs`, experimental upstream) | Content/name encryption | RW15 planned encryption integration with an established external tool; no home-grown crypto or plaintext staging |
| Archive file names | Browse archives as directories | RW15 planned archive resource contexts, with decompression limits, safe member paths and explicit local/remote extraction ownership; not automatically supplied by READDIR |
| Adding methods/operations (ELPA extension packages, `tramp-add-external-operation`) | Third-party method and operation injection | RW13: no plugin runtime (0001). The rootle [provider-protocol](https://rootle.dev/docs/provider-protocol.html) shape — a declared adapter contract with documented lifecycle assumptions — is the precedent for *if/when* strop declares a provider interface; introducing the abstraction now, with one provider, is premature |
| Trace verbosity levels 0–11, per-connection debug buffers, file mirroring | Diagnose without reconnecting | RW13: extend strop-trace with remote levels incl. wire frames (already bounded in 0034 stderr handling), redaction by default |
| `INSIDE_EMACS`/`EMACSCLIENT_TRAMP` env propagation | Let remote shells detect the editor | Deferred into RW7 and only if a real consumer exists; no speculative env contract |

## Ordered delivery slices

Dependencies are implementation/verification gates, not arbitrary release-order waits.
Independent slices may proceed concurrently against agreed interfaces. Each published
capability needs observable CLI/TUI behavior and its own real fixture/model evidence.

### RW1 — Tail, range reads, follow with rotation (P2; carries 0034's P2 label)

Implemented in 0036 / 0.17.0.

Bounded SFTP offset/length reads handle logs beyond the snapshot cap. Follow reopens
the pathname and compares bounded overlap/content before claiming append continuity.
SFTP v3 size/mtime are not inode identity; shrink or changed content causes a visible
reset, without pretending to distinguish physical truncation from every rotation.
Same-size/same-mtime replacement must still refresh correctly (0036).
Acceptance: open a >256 MiB log's tail; `+`-style and byte-range entry both work;
append, truncate and rotate in the fixture produce visible state transitions, not
wrong content; Escape stops following and leaves the buffer usable.
Verification: extend `RemoteRead` (or a `Follow` model) with rotation/truncation
states and their transitions; executable oracle asserts the state machine against a
scripted fixture.

### RW2 — Addressing and completion UX (P2)

Implemented in 0036 / 0.17.0.

Use negotiated expand-path@openssh.com for `~`/`~user`; REALPATH is not a tilde
expander. Enumerate literal host candidates from config/Include/known_hosts/history
on workers without executing Match exec. Path completion uses an existing authorized
connection or cached data; an explicit connect/open action is required otherwise.
Accept: no hidden auth from Tab, no stale prompt overwrite, byte-native paths, useful
unsupported-extension errors, and candidate ordering independent of display aliases.

### RW3 — Connection lifecycle: pooling, cleanup, reconnect isolation (P2)

Implemented in 0036 / 0.17.0. Explicit connection pins also count as owners.

One owned connection per (user,host,port) incarnation shared by buffers and jobs;
explicit cleanup subcommands (flush one host / all); last-buffer-close tears down
children; reconnect after network loss is a new incarnation — no cache, generation or
child leaks across it. TRAMP's cleanup family and its ControlMaster pitfalls (it
overwrites ControlPath to dodge stale masters) are the case study: strop owns only
connections it spawned and never kills masters it didn't create (0034 rule).
Acceptance: fixture shows the second open of a host reusing the connection (one auth,
observable handshake count) and cleanup leaving zero live children; a killed network
path followed by `:e!` reconnects with fresh state; a stale result from the old
incarnation cannot publish.
Verification: `RemoteRead` grows incarnation/pool ownership invariants (or a sibling
model); oracle drives kill/reopen in the fixture.

### RW4 — Remote write path (next requested release; hard safety gate, depends on RW1–RW3 verified)

The user explicitly requested implementation after the 0038/0039 release, followed
by another downloadable release. It is no longer an indefinite optional item;
write the dedicated execution plan and preserve every safety criterion below.

Writable remote editing needs content-aware conflict detection: size/mtime alone
cannot detect same-size concurrent rewrites. Define a server version/lease or
cooperating-lock protocol for commit-time exclusion; if unavailable, state/refuse the
concurrency guarantee rather than claim race-free compare-and-rename. Use a
same-directory private temp and verified atomic replacement (POSIX-rename extension
where necessary), preserve metadata, and handle symlinks explicitly.
Read-only remains the default disposition; writability is per-open explicit.
Acceptance: fixture proves external modification ⇒ conflict error, not overwrite;
kill between temp-write and rename leaves no partial file; permission bits and mtime
round-trip; a symlink target is never clobbered; cancellation mid-write leaves the
remote original intact.
Verification: write-transaction model (temp→rename state machine with crash points),
fault-injected oracles on the real transport. This gate is the precedent for RW5+.

### RW5 — Remote directory surface and file operations (read-only: 0036; mutations: P3 after RW4)

Read-only entry/parent navigation, search and filtering are implemented in 0036 /
0.17.0. The mutation and metadata-column acceptance below remains P3, after RW4.

The file-tree buffer (0001 pillar 2) over SFTP READDIR/STAT: motions, `/`, filter;
edit-the-line rename, `dd` delete, yank/paste copy with confirmation; attribute
columns (owner, mode, size, mtime) with honest errors where the server hides them.
Acceptance: rename/delete/copy observable in the fixture including failure modes
(permission denied, disappearing target mid-operation); tree cross-filters with
remote grep once RW7 lands.

### RW6 — Cross-host copy (P3; depends on RW5)

Local↔remote copy as first-class picker/tree operations; remote→remote via local
relay by default; direct remote-to-remote (`scp -R`-style or `rsync` over a provider)
only opt-in and only after checking TRAMP's stated preconditions — same host key seen
from both sides, passwordless auth between them, no RemoteCommand — each verified,
not assumed. Password caches are not consulted for direct copies (TRAMP's own
restriction).
Acceptance: relay path works for any pair; direct path refuses with a precise
unmet-precondition error; large copies are cancellable jobs with progress, not
blocking UI.

### RW7 — Remote process execution (P3; execution gate, depends on RW3)

The owned, native-argv process boundary and Git/LSP use are implemented in 0036 /
0.17.0. Arbitrary shells, grep, builds and formatters below remain P3; they are
explicitly refused rather than executed locally. Cleanup covers owned groups after
disconnect detection, not escaped sessions, indefinite partitions or supervisor death.

Remote shell jobs (`ssh host -- cmd` on a dedicated channel, never a filename-derived
command), remote grep feeding the picker, build/formatter jobs whose output lands in
owned buffers. Environment scrubbing and history suppression follow TRAMP's lessons
(HISTFILE pollution) — but only on channels strop itself opens. Optional `rsync`/
`docker`/`kubectl` providers register here behind the same job contract.
Acceptance: every remote command appears with its full argv in the UI; cancellation
kills and reaps the remote-invoking child per 0034; missing remote tooling is a typed
error naming the tool; job output cannot publish into closed/superseded surfaces.
Verification: process-ownership model extended to remote channels; fault-injection on
channel teardown.

### RW8 — Remote services (read-oriented LSP/Git: 0036; remaining integrations: P3)

Implemented in 0036 / 0.17.0: full-file diagnostics, hover, definition/references,
source/header navigation, Git context/status, staged/unstaged diffs, log, blame,
commit/file navigation and source links. Services run on their endpoint; project
trust includes endpoint/root, and last-workspace closure retires its language server.
Partial/follow windows refuse full-document language services. Source links pin
commit rows; deleted/header rows and partial-file coordinates cannot fabricate URLs.

Still P3: remote debugger integration (0019), later language-service capabilities
not implemented locally, and Git mutations after RW4. Each needs owned replies,
disconnect invalidation, cancellation and real remote fixture evidence.

### RW9 — Interactive authentication, MFA, credential privacy (P3; credential gate)

For SSH, use agent/askpass mechanisms with an explicit authenticated prompt path.
For other providers, integrate established system/provider credential stores.
Do not build a plaintext password database or put secrets into argv, buffers, logs
or session snapshots. Any in-memory secret handling needs expiry and lifecycle tests.
Acceptance: askpass-mediated MFA login works in the fixture with the secret crossing
only ssh's path; grep over traces/session files shows no secret material; a host
requiring unmediated interactive input fails with instructions rather than a hang or
prompt-forgery.

### RW10 — Privilege escalation, local and remote (P3; depends on RW4, RW7 for remote)

`sudoedit`-shaped local elevation first: each open/save is one `sudo` command, no
resident privileged session, honoring sudo timeout semantics; `su`/`doas`/`run0` as
provider commands with the same shape. Remote escalation is the composed case
(ssh to host, then per-command elevation there) — TRAMP's rule that the elevation hop
must match the reached host becomes an explicit check.
Acceptance: elevated buffer saves run one visible command each; sudo timestamp expiry
surfaces as a re-auth path, never as stale write authority; no long-lived root shell
exists to attack.

### RW11 — Additional transports, staged and opt-in (P3; after RW3, per-provider)

Stage optional transports by demand and isolation: rsync/modern scp bulk transfer;
Docker/Podman/Kubernetes and other named container methods; sshfs/rclone/FUSE;
Android/adb; SMB/share paths; GVFS/desktop/cloud integrations where the environment
supports them. Each is a separate capability with its own identity/auth/tool contract.
Native Windows/PuTTY and Kerberos integrations remain explicit platform slices, not
inferred from WSL or generic SSH support. Unsafe legacy rsh/telnet/ftp/rcp/nc modes
are non-default compatibility proposals requiring explicit exposure review.
No optional desktop service or container daemon becomes a requirement for the static
editor. Existing mounts may be used as local paths, but mounting/cleanup is explicit.
Every provider implements the same contract: typed absence errors, bounded reads,
cancellation, honest identity in the modeline, and no capability silently beyond what
its tool actually offers (e.g. `docker cp` glob/ignore quirks).
Acceptance: each provider lands with fixture evidence for its own failure modes;
URI scheme documentation states one provider per slice; no release bundles "all
transports".

### RW12 — Remote caches, sessions, autosave, locks, backups (P3; mostly explicit defaults)

No remote content cache, no automatic session restoration (0034 boundary) until an
opt-in exists with an invalidation story at least as strict as TRAMP's OS-version
flush. No remote backup files, lock files or autosave-to-remote by default — the
TRAMP-documented root-file exposure is the standing reason; if local autosave of
remote buffers is ever added, staging is private from creation (the R3 session
discipline). Buffer rehoming across host renames waits for demonstrated need.
Acceptance: after a fresh start with the network down, strop restores nothing remote
and errors honestly; opt-in session restore replays only with validated provenance.

### RW13 — Tracing, diagnosis, and the provider question (P3)

Remote verbosity levels in strop-trace (metadata by default; wire frames opt-in,
bounded, redacted); per-connection debug views reusing existing trace surfaces
(0029). The custom-method extension story: TRAMP grows methods via Elpa packages;
strop has no plugin runtime and should not pretend to. Derive a capability/ownership
interface from concrete backends when they share real operations, using rootle's
provider-protocol precedent (adapter lifecycle, assumptions, bounded checks).
Keep filesystem access, process execution and workspace provisioning separate so SSH
and future container/Dev Container support share editor semantics without duplicate stacks.

### RW14 — Dev Containers workspace lifecycle (P3; explicitly later)

TRAMP's container methods address an existing container. The
[Dev Container specification](https://containers.dev/implementors/spec/) additionally
describes building/configuring a development workspace from `devcontainer.json`,
Features, mounts, users, ports and lifecycle hooks. These are complementary layers,
not two separate editing stacks.
The dedicated researched design and lifecycle/model sequence is
[0037 — Dev Containers and workspace contexts](0037-devcontainers-and-workspace-contexts.md).

Depend on RW11's container filesystem/execution backend and RW3/RW7/RW8 ownership.
Use the reference Dev Container CLI where suitable rather than reimplementing the
specification inside strop. Add explicit attach/open, create/up, rebuild and stop
actions, multi-container/Compose selection, workspace folder/user/environment mapping,
and remote engine contexts (including an engine reached through SSH).

An execution context carries engine endpoint, immutable container identity/incarnation,
workspace root and user. File identities, open buffers, LSP, Git, jobs and caches reuse
that context; rebuild/recreate invalidates old capabilities and cannot accept stale
replies from a previous container with the same name.

Trust is a hard gate: image builds, Features, lifecycle hooks, privileged options,
host mounts and credential forwarding never run merely because a repository contains
a Dev Container configuration. Display the requested actions/capabilities and require
explicit trust before execution. Secrets remain outside logs and session snapshots.
The static editor has no required container daemon; Docker/Podman/Dev Container CLI
are optional capability prerequisites with honest absence errors.

Acceptance: open an existing container workspace, provision and rebuild a trusted
fixture, run the same remote browsing/LSP/Git paths, reject stale replies after
recreation, and verify that an untrusted configuration runs no hooks or builds.
Model provisioning/attach/incarnation transitions and exercise the actual CLI/runtime;
no claim of Dev Container support lands before these checks.

### RW15 — Archives, encrypted resources and compatibility edges (P3)

Archive browsing reuses resource/workspace identity without pretending a member is a
host path. Bound decompression and nested archives, reject traversal/link escapes,
and define extraction/cache ownership and read-only/writeback semantics explicitly.
Encrypted resources use established tools/providers; credentials and plaintext
staging remain private and opt-in. Accept: malformed/archive-bomb/traversal fixtures,
wrong-key/lock/cleanup cases, no plaintext in traces, and protocol/ownership models
where asynchronous extraction or writeback introduces state.

## Cross-cutting gates (P1-class obligations inside every slice)

- **Write gate (RW4+):** no write path ships without conflict detection, atomicity or
  honest refusal, crash-point fault injection, and a write-transaction model with an
  executable oracle.
- **Credential gate (RW9+):** secrets traverse established provider/system credential
  mechanisms; no plaintext secret persists in editor traces or session state.
- **Execution gate (RW7+):** every remote command is user-visible argv under strop's
  ownership; no filename-derived command assembly ever returns.
- **Verification growth:** TLA+ coverage grows with capability — `SftpWire`/`RemoteRead`
  (0034) then follow-state, write-transaction, incarnation/pooling and remote-channel
  models, each with named Rust oracles over the real boundary. All of it stays what it
  is in 0034: bounded model checking of an abstraction with stated bounds and OS/fairness
  assumptions, never a proof of the Rust implementation, OpenSSH cryptography, or
  arbitrary server behavior.

## Non-goals and uncertainties

- Native Windows/PuTTY/Kerberos support is planned compatibility work, not current
  platform coverage. A remote Windows server also needs its own path/process tests.
- Emacs-style transparency via a filename-handler layer; plugin runtime (0001).
- In-strop multi-hop/proxy configuration while ssh config covers it; ad-hoc
  filename-embedded hop chains specifically disfavored (opaque authority in a path).
- Replacing provider authentication with an editor password database is not planned.
  Encrypted resources/archives (RW15) and remote locks (RW12/RW4) remain on the roadmap.
- Performance claims of any kind until measured on the existing bench paths; platform
  support beyond Linux/macOS release targets.
- Uncertainties stated, not hidden: whether SFTP v3 rename atomicity suffices or the
  POSIX-rename extension must be required; whether `docker cp` semantics are uniform
  enough for the tree buffer's operations; whether FUSE mount management can respect
  the doctrine or stays manual; the real demand ordering for RW11 providers.

## Sources

- TRAMP manual (2.8.2.31.1): [index/TOC](https://www.gnu.org/software/emacs/manual/html_node/tramp/index.html),
  [quick start](https://www.gnu.org/software/emacs/manual/html_node/tramp/Quick-Start-Guide.html),
  [connection types](https://www.gnu.org/software/emacs/manual/html_node/tramp/Connection-types.html),
  [inline methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/Inline-methods.html),
  [external methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/External-methods.html),
  [GVFS methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/GVFS_002dbased-methods.html),
  [FUSE methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/FUSE_002dbased-methods.html),
  [optional methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/Optional-methods.html),
  [default method](https://www.gnu.org/software/emacs/manual/html_node/tramp/Default-Method.html),
  [multi-hops](https://www.gnu.org/software/emacs/manual/html_node/tramp/Multi_002dhops.html),
  [ad-hoc multi-hops](https://www.gnu.org/software/emacs/manual/html_node/tramp/Ad_002dhoc-multi_002dhops.html),
  [firewalls](https://www.gnu.org/software/emacs/manual/html_node/tramp/Firewalls.html),
  [password handling](https://www.gnu.org/software/emacs/manual/html_node/tramp/Password-handling.html),
  [connection caching](https://www.gnu.org/software/emacs/manual/html_node/tramp/Connection-caching.html),
  [remote programs](https://www.gnu.org/software/emacs/manual/html_node/tramp/Remote-programs.html),
  [remote shell setup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Remote-shell-setup.html),
  [ssh setup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Ssh-setup.html),
  [file name completion](https://www.gnu.org/software/emacs/manual/html_node/tramp/File-name-completion.html),
  [home directories](https://www.gnu.org/software/emacs/manual/html_node/tramp/Home-directories.html),
  [remote processes](https://www.gnu.org/software/emacs/manual/html_node/tramp/Remote-processes.html),
  [cleanup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Cleanup-remote-connections.html),
  [renaming remote files](https://www.gnu.org/software/emacs/manual/html_node/tramp/Renaming-remote-files.html),
  [auto-save/lock/backup](https://www.gnu.org/software/emacs/manual/html_node/tramp/Auto_002dsave-File-Lock-and-Backup.html),
  [encryption](https://www.gnu.org/software/emacs/manual/html_node/tramp/Keeping-files-encrypted.html),
  [archive file names](https://www.gnu.org/software/emacs/manual/html_node/tramp/Archive-file-names.html),
  [temporary directory](https://www.gnu.org/software/emacs/manual/html_node/tramp/Temporary-directory.html),
  [extension packages](https://www.gnu.org/software/emacs/manual/html_node/tramp/Extension-packages.html),
  [new operations](https://www.gnu.org/software/emacs/manual/html_node/tramp/New-operations.html),
  [traces](https://www.gnu.org/software/emacs/manual/html_node/tramp/Traces-and-Profiles.html).
- [SFTP v3 draft](https://www.ietf.org/archive/id/draft-ietf-secsh-filexfer-02.txt);
  [ssh(1)](https://man.openbsd.org/ssh.1) (transport options, ProxyJump, askpass).
- rootle provider protocol (extension precedent): https://rootle.dev/docs/provider-protocol.html
- Internal: 0001 (pillars, non-goals), 0002 (static binary, WSL), 0005 (config),
  0009 (LSP boundaries), 0011 (surfaces/jobs), 0028 (priority definitions),
  0029 (tracing), 0033/0034 (shipped remote scope and its verification approach).
