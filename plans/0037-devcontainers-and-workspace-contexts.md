# 0037 — Dev Containers and remote workspaces are complementary

Status: researched, planned for later (P3). Requested explicitly during 0036. This
plan preserves the architectural insight; it is not a claim of implemented Dev
Container support and does not expand the current release into provisioning.

## Decision

**One editor/workspace stack, two separate layers.**

- TRAMP-style transport answers: where is this resource, how do we read/list it,
  where do processes run, and who owns the connection/operation?
- Dev Containers answers: how is a development environment selected, built, created,
  configured, resumed and attached from declarative metadata?
- A running container does not need a Dev Container configuration to be browsable.
  A Dev Container does not need an SSH server inside it. Provisioning yields a
  container execution/filesystem context; existing buffers, LSP, Git, jobs and replay
  consume that context rather than a second editor implementation.

Keep `strop-remote`'s filesystem/protocol and process-launch boundaries distinct from
SSH policy. Derive container implementations from those real boundaries when RW11
lands. Do not introduce an unimplemented provider framework or container enum branch
merely to reserve a name. Provisioning may warrant a separate crate once implemented;
its dependency points toward workspace/transport contracts, never into Editor/UI.

## Research: what the specification actually includes

Primary sources:

- [Dev Container specification](https://github.com/devcontainers/spec/blob/main/docs/specs/devcontainer-reference.md)
- [`devcontainer.json` reference](https://github.com/devcontainers/spec/blob/main/docs/specs/devcontainerjson-reference.md)
- [Reference CLI](https://github.com/devcontainers/cli)
- [TRAMP container/inline methods](https://www.gnu.org/software/emacs/manual/html_node/tramp/Inline-methods.html)

The specification enriches container/orchestrator metadata; it does not replace the
container engine. Image, Dockerfile and Docker Compose configurations are distinct
cases. Compose identifies the main `service`, and optional `runServices` controls
related services. Support must be explicit per case, not inferred from Docker support.

Configuration discovery precedence is `.devcontainer/devcontainer.json`, then
`.devcontainer.json`, then `.devcontainer/<folder>/devcontainer.json`. Multiple
configurations are legitimate; the UI must allow selection. The input format is JSON
with comments, not strict JSON. Image `devcontainer.metadata` labels and Feature
metadata merge with the chosen file according to property-specific rules: not a
blanket last-write-wins map. Privilege/security fields can accumulate from Features.

Important separations:

- `containerUser` applies to container operation; `remoteUser` applies to injected
  developer processes and lifecycle commands. UID/GID synchronization is a Linux
  bind-mount concern, not an unconditional cross-platform action.
- `containerEnv` is creation-time environment; `remoteEnv` and `userEnvProbe` affect
  later developer processes. Environment changes need not recreate a container.
- `workspaceMount` makes source available; `workspaceFolder` can select a subdirectory
  (notably in monorepos). A local-looking path is not proof that the engine host can
  see it; remote Docker contexts/cloud engines need a deliberate source strategy.
- `initializeCommand` runs on the implementing host side. Creation commands run in
  the selected container. These trust domains must not be conflated.
- Creation hooks run in specified order (`onCreateCommand`, `updateContentCommand`,
  `postCreateCommand`); `waitFor` determines the attach-ready boundary. It defaults
  to `updateContentCommand`, so attach-ready is not the same as all background setup
  finished. Object-valued hooks run named commands in parallel and require all to
  succeed for that stage. Resume/attach hooks have their own lifecycle semantics.

The current reference CLI provides `read-configuration`, `build`, `up`,
`run-user-commands`, and `exec`. `exec` applies `remoteUser`, `remoteEnv` and
`userEnvProbe`, which a bare `docker exec` does not automatically reproduce. The
README currently lists `stop` and `down` as unimplemented: do not invent those CLI
commands. Lifecycle teardown must use explicit owned engine/orchestrator operations
or a future verified CLI capability. Current CLI builds/up can generate a Feature
lockfile; use frozen-lockfile behavior where reproducibility is required and never
silently modify repository configuration in a read-only/trust-preview path.

## Shared context and identity contract

A provisioned workspace resolves to typed data with:

- engine endpoint/context (local, remote engine, or engine reached via SSH);
- immutable container ID plus a workspace incarnation, never container name alone;
- chosen main service/configuration identity and owned sidecar IDs;
- canonical workspace root and file identity inside that context;
- developer user/environment profile and capability/trust decisions;
- lifecycle state and ownership receipts for resources strop actually created.

A container rebuilt with the same name is a new incarnation. Old file results,
completions, Git jobs, LSP replies, process leases and caches cannot attach to it.
Filesystem paths, executable argv and provisioning descriptions remain separate
native domains. No URI display label becomes a filesystem identity or shell program.

Existing mechanisms to reuse:

- real text buffers/pickers, shared grammar/search and source capabilities;
- request/document/revision/focus ownership and single terminal outcomes;
- connection/process cancellation and lifecycle/error reporting;
- LSP document URI/position mapping and Git repository provenance;
- full-content replay's native-launch suppression and metadata privacy policy.

The container filesystem backend should use the engine's actual file/exec facilities,
not require SSH inside the container or temporarily mount a container path as a
misleading local file. Partial/unsupported operations return typed capability errors.

## Security and ownership gates

No build, Feature, lifecycle hook, environment probe, package install or remote
execution occurs just because a repository contains configuration. Show the selected
configuration and requested effects before an explicit trust decision. Trust binds
to the relevant configuration/Feature/image metadata and engine/workspace identity;
changed privileges or configuration require renewed consent.

The review must surface host-side initialization, host mounts (especially engine
sockets and broad filesystem roots), devices, privileged mode, capabilities,
security options, host networking, credential forwarding and requested published
ports. Do not quietly broaden a loopback port to a public listener. Credentials and
secret-bearing environment values never enter command previews, trace content,
model fixtures or persisted sessions. Existing engine/CLI authentication mechanisms
remain responsible for registry credentials.

Attach to an existing container does not transfer ownership of its lifetime. Closing
strop may stop only resources it owns under the configured shutdown policy. Never
remove user volumes, unrelated Compose services or another application's containers.
Cancelled create/rebuild operations reconcile actual engine state before claiming
cleanup; a partial failure is a visible state with owned resource receipts, not a
pretend rollback. No resident privileged shell is introduced.

## Ordered implementation slices and acceptance

### DC1 — Existing-container context (P3; depends on RW11 container backend)

Split in two for delivery (9 Sep 2026): **DC1a — browse/read**: `strop-containers`
owns local-engine probe, canonical container identity (inspect resolves
name/prefix to the 64-hex id; a name that later resolves differently is a
stale-identity refusal), supervised `docker exec` reads/lists with deadlines
and bounded output, and read-only editor buffers for listings and files.
`Filesystem::Container(ContainerId)` joins the shared namespace identity
(0042); the engine is local-only until a remote-engine field is earned.
Accept: no SSH daemon, no analogous local path accessed, restart between
inspect and read detected where cheap, detach closes only what strop opened,
nothing is ever stopped or removed. **DC1b — LSP/Git/writes policy** remains
open: service routing into a container context, and the write capability
decision (likely refuse, like early SFTP).

Attach without provisioning. Resolve immutable engine/container/user/workspace
identity; browse/read files and run the same LSP/Git paths in that context. Accept:
no SSH daemon is required; no analogous local path is accessed; container restart or
recreation invalidates old requests; detaching stops only strop-owned processes.

### DC2 — Configuration discovery and trust preview (P3; depends on DC1)

Discover/select JSONC configurations and inspect merged metadata through a versioned
reference-CLI contract. No hooks/builds during preview. Accept: multiple configs,
image-label/Feature privilege merging, actionable invalid-config/tool-version errors,
and a malicious untrusted fixture whose marker command is never executed.

### DC3 — Create/build/up and attach readiness (P3; depends on DC2)

Use the reference CLI, with bounded owned jobs and streamed logs. Support image and
Dockerfile cases first, then Compose/main-service/sidecars with explicit capability
coverage. Apply remote user/environment/workspace semantics through the CLI rather
than approximating them. Accept: attach begins only after the configured `waitFor`
boundary; background setup remains visible; failures and cancellation do not produce
an apparently ready workspace; lockfile policy is explicit.

### DC4 — Resume, rebuild, stop and ownership reconciliation (P3; depends on DC3)

Rebuild starts a new incarnation and retires old processes/services/caches. Respect
shutdown action, preserve source/volumes and distinguish owned from attached resources.
Accept: mid-build cancellation, failed lifecycle hooks, lost engine connection,
same-name recreation, shared Compose services and restart all retain correct ownership.
Use verified engine operations for stop/cleanup until the reference CLI provides them.

### DC5 — Remote engines and polished workflow (P3; depends on DC4)

Resolve host paths/mounts against the actual engine host, including SSH-reached
engines. Add intentional source-copy/volume strategies, port/user/environment UI and
reproducible Features/Templates workflows where supported. Accept: no host-path
aliasing, no implicit public ports, and consistent browsing/LSP/Git behavior across
local engine and remote engine fixtures. Native Windows/client-specific behavior is
its own tested capability, not inferred from WSL or a Windows remote host.

## Model and executable verification contract

Before each slice ships, model its protocol changes alongside real engine/CLI tests.
Suggested `DevContainerLifecycle` safety properties: `NoUntrustedEffects`,
`NoWrongContainer`, `NoStaleWorkspacePublication`, `OnlyOwnedResourcesStopped`,
`NoReadyBeforeRequiredHooks`, and `LeaseScopedToIncarnation`. Model explicit receipts,
create/attach/rebuild epochs, hook readiness and cancellation/reconciliation states.
Keep mutations that omit trust, ignore incarnation or stop an unowned resource.

Progress properties require stated CLI/engine/OS scheduling and failure assumptions;
bounded model exploration is not a proof of arbitrary Dockerfiles or lifecycle code.
Executable fixtures must exercise image, Dockerfile, Compose, user/environment,
mount, trust refusal, cancellation and same-name rebuild transitions. Existing
Rust/SSH models continue to guard shared boundaries. Screenshots/demos drive the real
editor and real fixture engine. Documentation names exactly the supported spec/CLI
capabilities; no claim of complete Dev Container parity before the inventory is met.
