# Security and platform policy

## Active support

Strop currently supports **Linux x86_64 only**, including Linux x86_64 inside
WSL. The static release target is `x86_64-unknown-linux-musl`; native GNU/Linux
qualification remains required. Installation and execution occur inside Linux,
not through a native Windows executable or installer.

macOS, ARM/aarch64 and native Windows expansion require an explicit decision
and fresh target-specific qualification before support is advertised. Historical
artifacts and evidence are retained, not reused as proof for changed source.
Linux editor, worker, protocol, privacy and release controls remain required.

## Deferred Windows signing policy

No native Windows signing or installer channel is currently offered. The
following signing requirements are retained for a possible explicitly approved
Windows expansion, not a claim of available signed releases:

- Public Windows executables/installers would use [SignPath.io](https://signpath.io/)
  with a **SignPath Foundation** certificate, not developer experiments.
- The certificate identifies its issuing publisher, SignPath Foundation, not
  an individual developer. Authenticode and RFC 3161 timestamps would attest
  approved bytes from the reviewed process, not replace release qualification,
  reproducible-build evidence or independent artifact checks.

## Project governance

- Source repository: <https://github.com/stropdev/strop>
- Website and release instructions: <https://strop.dev/>
- License: MIT

The project owner is the committer, reviewer and signing approver:

- Tarek Nawara (`tknawara`)

The public GitHub collaborator/permission list is authoritative as the team
grows. GitHub accounts with source or release authority use multi-factor
authentication. If Windows signing is reactivated, each signed release requires
explicit manual approval in SignPath before publication.

## Build and approval controls

- Release binaries must be built from this public repository by automated GitHub
  Actions workflows, not from a private desktop copy.
- Product metadata names **Strop** and uses one consistent product version for
  every artifact in a release.
- Release artifacts first pass the repository's Compose/native quality,
  assurance and package gates.
- The release pipeline records immutable artifact URLs, SHA-256 digests,
  build provenance and publication outcome in the release catalog and promotion
  ledger. A future signing channel must also record its signing outcome.
- Only release artifacts whose expected digest and provenance match the
  approved source may be published; an unapproved or mismatched build fails
  closed.

## Privacy

Strop does not transfer user files, source text, debug targets, terminal
content or telemetry to networked systems as a background feature. Network
transfers happen only for operations the user explicitly requests, such as
language servers, SSH, containers, update checks/downloads, Git remotes or
other configured tools. Optional traces and release diagnostics record content
only under their documented opt-in controls; terminal content defaults to
metadata-only recording. Installation and uninstallation do not change system
configuration beyond the explicit registered application files and settings.

Third-party packages and invoked tools retain their own privacy policies;
Strop's dependency review and SBOM identify those components rather than
presenting their behavior as Strop's own.

## User controls

- The site documents installation, update, uninstall and supported-platform
  boundaries.
- Installation never performs hidden WSL provisioning or system configuration
  changes. WSL must already provide a supported Linux x86_64 environment.
- Uninstall and optional Strop-owned cache cleanup must name their explicit
  scope and preserve user data unless separately approved. Linux workspaces,
  WSL distributions, projects and independently installed tools are never removed.
- Any future Windows installer must preserve these ownership boundaries for its
  application versions, launcher, shortcuts and registration.
