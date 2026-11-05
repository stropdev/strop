# SSH, remote editing and containers

Remote files are first-class: real buffers, real identity, never a local
lookalike. The short version:

```sh
strop +120 ssh://user@devbox/var/log/app.log
strop --tail 65536 ssh://devbox/var/log/app.log
strop --range 1048576:65536 ssh://devbox/var/log/app.log
strop --follow ssh://devbox/var/log/app.log
strop remote "ssh://user@devbox:2222/~/project/src/lib.rs"
strop ssh://devbox/etc/
```

`strop remote URL` and `--remote URL` are the same command — see
[cli.md](cli.md).

Remote files open read-only by default: motions, `/`/`?`, visual selection,
yank and splits work unchanged. Escape cancels an open or stops following;
`:e!` refreshes the snapshot. Follow sticks to EOF only while you stay
there. Shrink, replacement and changed overlap produce a visible reset;
size/mtime alone are not file identity. The modeline identifies partial
byte windows, whose line numbers are window-relative.

## Connecting

Press **Space o** or run **`:remote`** to choose a known/connected
destination, select **Add a host**, or choose **Containers** to attach to a
running container. Enter `host`, `user@host:port`, or a complete SSH path.
Enter is the connection action; opening the chooser does not authenticate.
New hosts start at `/`; successful directory choices are remembered
privately. `:remote home` uses negotiated remote-home expansion;
`:remote root` returns to `/`.

```vim
:tail 65536 ssh://devbox/var/log/app.log
:range 1048576 65536 ssh://devbox/var/log/app.log
:follow
:unfollow
:browse ssh://devbox/
:filter app
:remote connect ssh://devbox
:remote list
:remote disconnect ssh://devbox
:remote clear
```

Directory listings are searchable real buffers with kind, POSIX
permissions, server-reported byte size and native filename columns.
Missing attributes show `?`, distinct from mode `000` and size zero.
Directory sizes are not recursive totals. Links and special entries have
distinct type markers. Enter opens an entry; `-`, Backspace or `../`
returns to the parent and restores the selected child. `:filter` narrows
names; an empty filter restores the full listing.

SSH and container listings include hidden entries and do not evaluate
ignore files; the modeline names this fixed policy. Plain Ex open operands
keep their local meaning outside Directory buffers; inside Directory they
use that directory's captured namespace. Explicit `file://` and `ssh://`
locations select the namespace regardless of the current view. Tab
completes SSH hosts and paths without starting authentication: remote
candidates need a live authorized connection or cached data. Connections
are shared by endpoint and held by documents or explicit `:remote connect`
pins; clear/disconnect retires them without touching external SSH masters.

OpenSSH supplies aliases, keys, agent and ProxyJump configuration. Host
keys must already be trusted; authentication is noninteractive.
Percent-encode reserved path bytes (`%20`, `%23`, `%25`); native Unix
filenames stay intact. Reads are bounded to 256 MiB of UTF-8 text;
range/tail edges exclude incomplete UTF-8 characters. Home expansion
requires the server's `expand-path@openssh.com` extension.

## Explicit remote editing

On a complete, non-following file snapshot, run **`:remote edit`**. The
editor verifies the displayed content against the server and grants this
document write authority. Edit normally, then use **`:w`** or **`:wq`**.
Saves run off the input thread; a receipt for an older revision never
clears newer unsaved edits.

The write path checks content and metadata, stages inside a private 0700
transaction directory, preserves mode/ownership/mtime/extended attributes,
atomically replaces the file, and syncs the file and changed directories.
Unsupported preservation refuses the save. It requires an owned regular
single-link file and exact stored path spelling. Symlinked ancestor
directories are resolved and revalidated; a symlink as the final component
is refused. Edited snapshots remain bounded to 256 MiB. Protocol
lock/staging paths are reserved; private lock files remain to keep a stable
lock identity — do not delete active locks.

**Concurrency is cooperative, not universal compare-and-swap.** Strop's
remote-save participants share a lock. Completed changes by other programs
are detected before commit, but a nonparticipating writer can race the
final check/rename window. `:w!` never bypasses that conflict check or
grants authority to a read-only snapshot. Escape in normal mode requests
cancellation. If a save may have crossed rename, local edits stay dirty and
the outcome is explicitly unconfirmed. **`:remote verify`** uses the frozen
prepared attempt and any available receipt to reobserve and sync matching
saved content, including after a worker restart in the same Linux
boot/mount namespace and principal. It never blindly overwrites or
retries. Missing evidence or a changed namespace remains unconfirmed.
Conflicts preserve local edits. Successful `:e!` refresh revokes the
permit; pending/unconfirmed saves must settle or be verified first. A
failed refresh does not discard edits or permissions. Forced close may
leave an unconfirmed remote outcome and a private orphan stage.

Remote contents and write permits are never restored from persisted
sessions. See the [remote-save contract and safety
limits](../plans/0040-remote-editing-and-saving.md).

## Remote services

Full-file snapshots support remote LSP diagnostics, hover,
definition/references and source/header navigation, plus Git context,
staged/unstaged diffs, log, blame and commit/file navigation once the
endpoint has an admitted worker. Run **`:remote worker [URI]`** to
authorize a verified worker without granting a file write permit, or
**`:remote edit`** on a complete snapshot to admit the worker and
separately verify edit authority. Services run **on the remote host**,
never against a local lookalike path. Project-command trust is scoped to
the endpoint and remote root (`:trust`). Partial windows and following
refuse full-document language services; Git mutations, remote save-as and
arbitrary shell/filter commands remain unsupported.

SFTP browsing and ranged reads need no remote worker or Python. Protected
editing, Git, LSP and search run through a verified native worker in the
selected namespace; worker admission may write a private executable cache
on that host. With that admission, **`:terminal`** and **Space a → Terminal
here** also run an interactive PTY on the remote host, with the endpoint and
launch directory retained in the modeline, pane title, buffer list and
`:explain`. **`:terminal-local`** remains the explicit host-local alternative.
Without a worker, terminal execution refuses rather than starting locally.
A restricted/SFTP-only account remains read-only, with typed
refusals instead of Python, shell, SFTP-write or local-path fallbacks. The
worker artifact must match the target and protocol; a foreign-target
artifact can be supplied with `STROP_WORKER_BINARY`. Git/LSP also need
their respective programs in the remote workspace. Owned process groups
are cleaned up when the worker observes lease loss; network partitions
delay detection, descendants creating new sessions can escape the group,
and a killed worker cannot guarantee cleanup.

`strop-remote` owns SFTP and the SSH worker transport; the editor owns
views, write permits and replay. See the [workspace
contract](../plans/0036-remote-workspace-execution.md), [protocol
evidence](../plans/0034-ssh-log-buffers.md), the [prioritized remote
roadmap](../plans/0035-remote-workflow-roadmap.md), and [Dev Containers
design](../plans/0037-devcontainers-and-workspace-contexts.md).

## Containers

Run **`:containers`** or choose **Space o → Containers** to select a
running container. Directory browsing, file reads and tab completion stay
read-only and do not install software. Run **`:container-worker`** in an
attached container buffer to authorize a verified native worker in that
container's selected Docker engine, canonical id, StartedAt, user and
working directory. The default path uploads the matching static artifact
into that user's private executable cache. For shell-less images, set
`STROP_CONTAINER_WORKER_PATH` to an already installed matching binary; the
worker is verified in place, without upload. Missing tools, wrong targets
and recycled containers refuse typed rather than executing Git/LSP on the
host or through an implicit `docker exec sh` fallback. Generic attachment
starts at the inspected WorkingDir, falling back to `/`; an explicit path
wins. WorkingDir is a navigation hint, not repository/mount proof.

Worker admission enables Git/LSP, scoped Search and interactive terminals.
Container definitions, references, diagnostics and document/workspace symbols
open through the same canonical container identity and native root; picker
previews use the admitted worker and never the analogous host path. Closing
the last document in a container LSP workspace retires that workspace's
server without affecting local, SSH or other container roots.
**`:container-edit`** is the separate file-authority step: it verifies the
displayed snapshot against that incarnation and then permits editing. Saves
use the worker's conditional Store baseline, protected staging and atomic
publication; conflicts and unconfirmed outcomes retain verification evidence
and never become `docker cp` or blind rewrites. Worker admission alone leaves
files read-only. Filesystem mutations and project replacement remain refused.

## Dev Container configurations

Selecting a project root performs bounded, read-only discovery of
`.devcontainer/devcontainer.json`, `.devcontainer.json` and one-subfolder-deep
`.devcontainer/<folder>/devcontainer.json`. JSONC and multiple configurations
are supported. A single non-focusing **Dev container configuration found**
offer has Review, Later and workspace-scoped suppression actions; changed
configuration digests invalidate earlier decisions. Discovery never invokes
Docker, the reference CLI, lifecycle hooks, builds, pulls or network
authentication.

**Space a → Review options** shows the selected configuration, provider,
engine, users, workspace/mount mapping and redacted effects before consent.
Host initialization, mounts/sockets, privileges/capabilities, devices, host
networking, credential forwarding and ports are explicit trust inputs.
Attach existing, create/start and rebuild are different actions. The optional
reference Dev Container CLI versions 0.87–0.90.x are supported; Strop never
installs it automatically and ordinary editing needs no such dependency.
Image, Dockerfile and Compose main-service configurations are supported;
unsupported shapes remain inspectable and refuse rather than becoming a
generic `docker run`.

Setup logs stream into a real read-only buffer, with cancellation and partial
resources retained for inspection. Dirty local buffers require Save/Stay/Cancel
before switching. Successful transitions verify the returned container and
workspace mapping before opening it; rebuild retires the previous incarnation
without deleting containers or volumes. Remote engines, automatic port
forwarding and template/Feature marketplaces are outside this workflow.

In a container without an init/reaper at PID 1, repeated Docker exec
bootstrap processes can remain as zombies after they exit. Strop closes its
worker lease, but cannot reap process-table entries adopted by an unrelated
PID 1. For sustained worker sessions without that zombie accumulation,
start the container with an init that reaps children.
