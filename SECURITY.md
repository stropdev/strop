# Code signing policy

Strop publishes signed Windows executables and installers through
[SignPath.io](https://signpath.io/), using a certificate from the
**SignPath Foundation**. Signing applies to public release artifacts; it is not
used to publish developer experiments or unsigned internal validation binaries
as trusted releases.

The signing certificate identifies the issuing publisher, **SignPath
Foundation**, not an individual Strop developer. Authenticode signing and an
RFC 3161 timestamp attest that approved bytes came through the project's
reviewed build-and-signing process; signing does not replace release
qualification, reproducible-build evidence or independent artifact checks.

## Project governance

- Source repository: <https://github.com/stropdev/strop>
- Website and release instructions: <https://strop.dev/>
- License: MIT

The project owner is the committer, reviewer and signing approver:

- Tarek Nawara (`tknawara`)

The public GitHub collaborator/permission list is authoritative as the team
grows. GitHub accounts with source or release authority use multi-factor
authentication, and each signed release requires an explicit manual approval in
SignPath before publication.

## Build and approval controls

- Signed binaries are built from this public repository by automated GitHub
  Actions workflows, not from a private desktop copy.
- Product metadata names **Strop** and uses one consistent product version for
  every artifact in a release.
- Release artifacts first pass the repository's Compose/native quality,
  assurance and package gates.
- The release pipeline records immutable artifact URLs, SHA-256 digests,
  build provenance and the signing/publication outcome in the release catalog
  and promotion ledger.
- Only release artifacts whose expected digest and provenance match the
  approved source may enter signing; an unapproved or mismatched build fails
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
- Silent installation never performs hidden WSL provisioning or system
  configuration changes.
- Windows uninstall removes only Strop-owned application versions, launcher,
  shortcuts and registration by default. Linux workspaces, WSL distributions,
  projects and independently installed tools are never removed.
- Optional Strop-owned backend-cache cleanup names its explicit scope and
  preserves user data unless separately approved.

Free code signing is provided by SignPath.io; the certificate is issued by
SignPath Foundation.
