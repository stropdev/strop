# 0033 — Reviewer feedback: trustworthy failures and locations

Status: implemented for the 0.16.0 milestone. The external review is the evidence baseline:
WSL Ubuntu 24.04; findings 1 and 2 also reproduced on 0.15.1. This does not claim
RHEL coverage. The already-fixed count/G/cc/C cases will not be rechecked merely
to confirm the reviewer; existing regression and differential gates remain intact.

## Decisions before code

All four current findings and the command-line location gap are adopted.

1. **P1 — permalink identity.** Preserve complete HTTP(S) authorities and repository
   paths. SSH alias resolution must use OpenSSH's effective configuration (`ssh -G`),
   including Include/wildcard/HostName rules, not a partial home-grown config parser.
   Native configuration evaluation belongs on an owned worker, never input/render.
   Pure URL construction must consume resolved data. Resolution or unsupported-host
   failures must not copy a guessed/dead URL. Keep remote priority and immutable SHA
   pinning, and retain native file identity instead of reverse-parsing display labels.
   The reported HTTPS truncation is accepted as an observable defect; the currently
   visible HTTP parser does not itself explain the first-dot hypothesis, so the fix
   must cover the complete producer-to-copy path rather than assume that hypothesis.
2. **P1 — malformed language configuration.** Keep the existing per-layer recovery
   and configuration merging. Carry each layer's typed diagnostic, including its
   path, through discovery to the modeline and structured trace even if a valid
   fallback server starts. Success must not erase a configuration warning.
3. **P1 — missing LSP executable.** A terminal attach/spawn failure must reach both
   UI and trace with the executable and actionable installation/configuration hint.
   Preserve trust checks; do not execute a project command merely to probe it before
   trust. Avoid orphaned `--version` probes and duplicate startup attempts. Stale
   failures must not overwrite newer owners.
4. **P1 — silent normal keys.** Silence is a bug. `I`, `^`, `~` and `S` are currently
   advertised as live in the command table; blacklisting them would hide an existing
   fidelity contract. Repair their dispatch/implementation where advertised, and route
   genuinely unhandled/invalid completed input through the same Invalid action as
   `gI`. No four-key special case, and no false "live" help entry.
5. **P2 — CLI locations.** Support `+LINE FILE` and `FILE:LINE`, with a checked,
   one-based line at the CLI boundary and the existing typed zero-based line domain
   internally. Clamp an out-of-file line to the last content line. Preserve native
   filename bytes. `--` provides literal-operand escape for colon-suffixed names;
   malformed/overflowing explicit `+LINE` is an error, not line one. Help and real
   headless/terminal entrypoints must describe and exercise the same behavior.

## Ownership and integration

Main owns CLI/startup, normal-key routing, integration, all final validation and
roadmap/documentation. LSP discovery/error delivery and permalink resolution are
independent source slices. Their owners skip builds, tests, linters and formatters
while editing. Main validates the integrated tree.

No synchronous SSH/config/filesystem work is allowed on input → render. Preserve
the existing worker/ticket/event/replay boundary and update every affected caller.
Use named domains and typed errors at new boundaries, no compatibility aliases,
error suppression, guessed URLs or oversized mixed-responsibility modules.

## Acceptance

- FQDN HTTP(S), explicit SSH hosts and configured aliases produce the intended
  pinned URL; nested repository paths survive. Failed resolution changes neither
  clipboard/register content nor browser target and is visibly diagnosed.
- Isolated malformed global/project TOML fixtures demonstrate retained valid
  layering plus a visible diagnostic containing the actual path; trace records it.
- A missing configured executable reaches a terminal failure with a useful hint in
  both modeline and trace. Healthy configuration and initializationOptions forwarding
  continue to work; no server absence is misreported as success.
- Each reported key either performs its advertised Vim operation (differentially
  checked) or follows a truthful unsupported path. Invalid input cannot disappear
  into Pending after the machine has cleared its state.
- Actual CLI opens a local file at the requested line, including non-UTF-8/space and
  literal-colon cases; no location suffix silently becomes a new filename.
- Docker fmt/Clippy/tests and the protocol gate pass. Tests defend behavior and real
  errors, with no network, real HOME, timing sleeps or source-text assertions.

## Next tranche

Remote log access is a separate requested feature. Research TRAMP/OpenSSH and write
0034 before changing the remote-document architecture. The objective is a focused,
responsive read/search workflow, not a claim to replace every TRAMP capability.
No reviewer finding above is deferred. Broader remote editing, execution and other
capabilities will be prioritized explicitly in that design rather than inferred.
