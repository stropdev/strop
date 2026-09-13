# 0062 — Distribution: one release, clean Windows onboarding, preserved TUI

Status: requested distribution/release handoff; no installer, publication or host
configuration change is performed by this research. The user has no strong channel
preference: make installation and updates easy and truthful instead of accumulating
package formats.

This extends [0002](0002-build-and-release.md) and serves the native Windows GUI +
WSL execution design in [0061](0061-gui-windows-and-wsl.md). TUI/static/crates.io
contracts remain intact. GUI packaging does not authorize a native Windows
filesystem, Windows LSP/DAP or ConPTY implementation.

**Prerequisite ownership:** [0056](0056-architecture-prerequisites.md) AR11–AR12
close existing TUI install/update identity, transaction and release-catalog/site
drift first; AR09–AR10 deliver the non-graphical backend. This plan extends those
released foundations for Windows installation, WSL onboarding and GUI channels,
not another implementation of the generic repairs. PKG01–PKG14 behavior remains.
The separate [0057](0057-core-verification-and-assurance.md) VF01–VF20 release
qualifies the pre-worker core; [0058](0058-unified-native-worker.md) WK01–WK20 then
delivers and requalifies shared native execution/deployment. This plan extends both
with actual Windows/WSL installation effects, not an inherited proof badge.

## 1. Distribution decision

**One authoritative tagged GitHub release, one generated release catalog, two
product entry paths:**

1. **Existing TUI:** verified Linux/macOS tarballs, strop.dev installer and the
   existing Cargo/Homebrew/mise routes. Keep these first-class.
2. **Windows GUI:** a signed, per-user Windows installer linked prominently from
   strop.dev. It installs the native frontend/launcher; first use selects WSL and
   installs a matching Linux backend privately with explicit consent.

Use **NSIS in standard/current-user mode** for the initial Windows installer.
It supports a simple unprivileged install/uninstall story and winget's known
Nullsoft installer handling. Do not start with MSIX identity/store packaging,
a custom WSL distro, Flatpak/AppImage/FUSE or several Linux GUI package formats.
The chosen GUI is Windows-native; its WSL backend is the already-supported static
Linux program, not a Linux GUI runtime bundle.

**Winget is an additive channel for the same installer**, not a second installer
or owner of Linux distro internals. Prepare and test the manifest; public listing
can follow repository review. The first clean strop.dev release does not depend
on an external winget review completing, and the website must not advertise an
unavailable winget command.

No subscription/account/cloud broker is needed to use the editor. No administrator
privilege is required for ordinary Strop installation, updates or workspace access.
Installing WSL itself can require explicit Microsoft-supported elevation/reboot;
that is a visible prerequisite, not something hidden in Strop's installer.

## 2. Required delivery ledger

| ID | Required delivery |
| --- | --- |
| PKG01 | Explicit artifact/support matrix preserving all current TUI targets and adding the supported native Windows GUI target. |
| PKG02 | Extend the released 0056 catalog with exact GUI artifacts, digests, backend/protocol compatibility and immutable URLs. |
| PKG03 | Signed per-user Windows install, stable launch/Start-menu/Installed-apps integration and silent install/uninstall. |
| PKG04 | Clear first-run WSL detection, distro/user selection and explicit prerequisite/consent flow without destructive system changes. |
| PKG05 | Verified versioned WSL backend bootstrap/cache using 0056 installation identity and 0058's qualified artifact/deployment primitives; no overwrite of an independent TUI install. |
| PKG06 | Extend the qualified 0056/0058 ownership/update contracts for native Windows activation and GUI/backend rollback, preserving active work and worker leases. |
| PKG07 | Scoped uninstall/cleanup and optional data removal; never remove a WSL distro, project or unrelated tool. |
| PKG08 | Checksums, Windows signatures/timestamps and build provenance validated under their actual guarantees. |
| PKG09 | Extend 0056's single truthful release/site catalog with GUI onboarding and actual channel availability, with post-publish verification. |
| PKG10 | Required artifact/installation gates, source-bound protocol proof/conformance evidence and a resumable publication/promotion ledger. |
| PKG11 | Actionable doctor/progress/failure paths for unsupported/missing WSL, offline/proxy, bad artifacts, permissions, disk space and version mismatch. |
| PKG12 | Winget-ready tested manifest/installer semantics; if a public listing is claimed, acceptance and actual install/upgrade/uninstall are verified. |
| PKG13 | License/dependency/runtime/SBOM policy preserving publishable TUI crates and no-OpenSSL/static guarantees. |
| PKG14 | Clean-profile installation/update/rollback/uninstall evidence, integration docs/changelog and no lingering prototype/install scaffolding. |

All PKG requirements are mandatory within their stated envelope. PKG12 requires
readiness/testing, not pretending the project controls external review time.
Additional channels and platforms in §13 are explicitly later. A release owner
records evidence/status per ID; silently dropping a required path needs user
approval and a roadmap entry.

## 3. Pre-prerequisite evidence and foundations to consume

The inspected repository already has useful release foundations:

The historical gaps below are assigned to 0056 AR11–AR12, whose closure is required
before extending the release for Windows. Preserve already-correct mechanisms;
do not defer these repairs into installer/GUI implementation.

- `.github/workflows/release.yml`: tags `v*`, serialized release runs, four TUI
  tarballs (Linux musl x64/arm64 and macOS x64/arm64), checksum checks, Linux
  dynamic-dependency verification, real headless artifact smoke, attestations,
  Cargo publication, Homebrew updates and site dispatch.
- `.github/scripts/publish-order.py` already derives dependency order, skips
  `publish = false` packages and rejects a publishable crate depending on a
  private workspace crate. Reuse it; don't invent another publication graph.
- `install.sh`: OS/architecture choice, confirmation, mandatory SHA256 sidecar,
  extraction and installation. It currently writes into the target path using
  install/cp fallbacks rather than a unified versioned install transaction.
- `strop update`: private staging, mandatory digest verification and atomic native
  replacement. Installation channel detection is currently path-substring based;
  don't stretch that heuristic into managing a GUI plus several WSL backends.
- Site ownership is the separate `stropdev/stropdev.github.io` repository: static
  pages/assets, copied installer, demo and release/changelog updates.
- Website updates in the current release workflow are best effort. The current
  roadmap's user-facing requirement is stronger: verify what actually reached users.

A browser inspection—not only reader-mode text—showed **both** an old `v0.19`
feature/hero line and a dynamic `v0.30.0` version chip on strop.dev. This is not a
claim that the release itself is 0.19; it demonstrates independent version strings
that can drift. Build all version/download/support claims from the release catalog.

No installer or backend bootstrap was executed during this research. A disposable
native Windows window was launched/driven from WSL for the GUI automation research;
that is not installation/signing/update evidence.

## 4. Product and artifact matrix

Preserve the current TUI names/layout unless a separately reviewed migration is
necessary:

| Product / target | Artifact / role |
| --- | --- |
| Linux x64 TUI/backend | `strop-VERSION-x86_64-unknown-linux-musl.tar.gz` and digest; consumes the real UI-server mode delivered by 0056. |
| Linux arm64 TUI/backend | Existing aarch64 musl equivalent; usable where the tested GUI/backend pairing supports that WSL architecture. |
| macOS x64/arm64 TUI | Existing native darwin tarballs and Homebrew/Cargo routes. |
| Windows 11 x64 GUI | `StropSetup-VERSION-x64.exe`, containing signed launcher/frontend/assets and matching release/backend metadata. |
| Release metadata | Versioned machine-readable catalog plus checksums/provenance/license inventory for the actual artifact set. |

Windows ARM64 and additional GUI operating systems are later supported targets,
not inferred from a dependency's portability. Keep architecture/OS/ABI names explicit;
a Windows browser must not be silently offered a Linux tarball as a native app.

The GUI's WSL cache consumes the same released Linux artifact and 0056's implemented
UI-server contract, not a second engine fork or a backend invented during packaging.
Keep exact version/protocol identity and verify the released server behavior;
do not add a second release authority.

The TUI remains a zero-config static Linux binary. The native Windows GUI can have
its own bundled assets and documented Windows system dependencies; don't apply a
false “no dependencies whatsoever” label to fonts/graphics or weaken the TUI gate.

### Release catalog

Extend 0056 AR12's generated catalog rather than inventing another source of truth.
Add GUI product/target/immutable URL/size/digest, signing/provenance and exact backend
pairing to that contract. All consumers resolve one immutable release before
fetching components; do not mix a moving latest backend with an older GUI.

The first GUI/backends use an exact tested release pairing plus explicit protocol
version. Broad cross-version compatibility is unnecessary until there is a real
need and test matrix. Mismatch is a repairable error, not a best-effort connection.

## 5. Native Windows installation and launch

Use a current-user installation under `%LOCALAPPDATA%\Programs\Strop` with a
stable app identity. Prefer versioned application directories and a small stable
launcher/activation record rather than overwriting a running GUI executable.

The launcher, GUI and helper binaries that execute on Windows are signed. Resolve
only validated relative paths inside the owned installation; no PATH search or
workspace-controlled DLL/helper lookup. Limit DLL search to the application/system
locations actually needed. User-controlled workspace data cannot choose an updater
or arbitrary host executable.

Required installer behavior:

- Standard/current-user execution level by default, not “highest available” merely
  because the account can elevate. No service, driver, WSL feature or distro install.
- Start-menu shortcut, stable application identity/taskbar behavior and correct
  Installed-apps publisher/name/version/uninstall registration.
- Silent install/uninstall through the documented NSIS path, with reliable exit
  codes and no interactive WSL setup hidden in silent mode.
- Spaces/Unicode/long user paths and non-default install locations work. Do not
  derive installation paths from a hardcoded Windows username.
- Update stages a complete versioned bundle, verifies it, then atomically activates
  its launch record. A failed extraction or interruption leaves the old active
  version usable. No success message before activation is verified.
- Do not register arbitrary C:\ text-file associations while native Windows
  workspaces are unsupported. A labelled WSL-open entry/URI can be added only with
  checked distro/path semantics; no command-string protocol handler.

Normal Windows users launch Strop from the Start menu. WSL users may use a
`strop gui`/explicit GUI-launch entry once implemented: it preserves the selected
distro and Linux path through argv/typed location data. Avoid a wrapper that simply
passes a Linux path to an unrelated Windows filesystem opener. Register/install
that bridge without clobbering an existing user's independent TUI command.

## 6. First-run WSL onboarding

The Windows app is usable enough to explain prerequisites before the backend is
ready. WSL checks/startup/downloads run as owned background work; never freeze the
window while a distro starts.

### Existing WSL user—the primary journey

1. Detect WSL and enumerate installed distributions through supported WSL commands.
   Decode their actual output encoding; don't strip NUL bytes blindly or scrape
   localized status text into a fake structured result.
2. Offer explicit distro/user selection and show Linux home/architecture/backend
   status. Remember the successful Strop preference without changing the system's
   default distro or user.
3. Explain the one-time private backend location and exact version/download. Ask
   before installing there; a distro selector is not blanket permission to modify
   every distribution.
4. Verify or install the matching backend, complete the protocol handshake and
   show Ready only after the real process responds.
5. Open a Linux workspace. WSL `.config/strop/config.toml`, SSH and tool environments
   remain authoritative. Windows stores only GUI/installation preferences.

### No usable WSL

Give a concrete explanation and an explicit setup action using Microsoft's supported
WSL installation/update path. State elevation/reboot consequences before invoking
it. It is acceptable to hand the user the exact Microsoft command/documentation;
it is not acceptable to fake a working backend or provision a surprise custom distro.

Never automatically run `wsl --shutdown`, `--terminate`, `--unregister`, change
`.wslconfig`, default users, mount metadata, GPU drivers or container settings.
These can destroy work or affect unrelated applications. The native GUI + Linux
backend does **not** require WSLg/Vulkan/desktop portals in the distro.

Offline/uninitialized/permission-limited distributions remain explicit states.
A user may install the Windows frontend before WSL is usable; do not lie that a
workspace can already open. Retry after prerequisite repair is an explicit action.

## 7. Backend bootstrap: fixed, verified, scoped

Install backend versions under an owned per-user Linux data path, for example:

```text
$XDG_DATA_HOME/strop/backends/VERSION-TARGET/strop
# default: ~/.local/share/strop/backends/...
```

Do **not** overwrite `~/.local/bin/strop`, Cargo/Homebrew/mise installs or a user's
project toolchain. GUI backend pairing and an independently used TUI can differ
without one silently updating the other.

Reuse 0058's qualified artifact verification, private staging, immutable activation
and lease-safe cleanup rather than another worker installer. GUI backend installation
and execution-worker caches have explicit role/owner receipts; sharing an executable
does not conflate their lifetimes or authorize cross-role cleanup.

Recommended flow:

1. Native frontend resolves the immutable release/backend metadata and downloads
   the archive on the host, or accepts an explicitly selected verified offline
   bundle. No need to require curl/network access inside every distro.
2. Verify the expected digest against the authenticated GUI release metadata before
   transfer. Transfer bytes through the owned WSL process/stdio path, not via a
   guessed `/mnt/c/Users/...` temporary file.
3. Run a **fixed, versioned bootstrap** in the selected distro with data arguments:
   version, target, expected digest and owned installation intent. Do not interpolate
   labels/paths into shell text or run project scripts. Use non-login execution so
   shell startup banners cannot corrupt the result/protocol.
4. Create private staging in the correct owned parent, validate archive structure,
   extract only the expected regular executable/assets, reject traversal/absolute
   names/links/devices/unexpected entries, set permissions and verify the actual
   binary/version plus UI-server and native-worker protocol compatibility.
5. Publish the version directory without clobbering a different/in-use version;
   record a bounded machine-readable receipt. A failed/interrupted install cleans
   up only its proven private staging, not a shared parent or another install.
6. Launch the verified absolute Linux path via `wsl.exe --distribution ... --user ...
   --exec ...`, with an explicit Linux startup directory. Don't inherit the Windows
   install folder as an accidental `/mnt/c` workspace.

Reuse the completed 0058 no-Python bootstrap/deployment contract and static Strop
artifact. This release adds the actual Windows→selected-WSL adaptation, not a second
helper runtime or generic installer. No global Python/node/Rust/Zig/toolchain,
hidden remote network dependency or temporary unverified executable is permitted.

Return actual selected distro/user, architecture, backend path/version/digest and
protocol result. A Windows wsl.exe PID is not an editor-session identity. Active
backend versions are leased/owned; cleanup cannot infer safety from a recycled PID.
No local port, SSH daemon or persistent system-wide Strop service is required.

## 8. Updates, activation and rollback

### One authority per installed component

Consume 0056 AR11's installation identity/receipts and verified native transaction.
Native Windows installation/backend caches extend those typed ownership records;
they do not reintroduce path-substring guesses or another unsafe update path.
Legacy unmanaged TUI handling is already closed by the prerequisite release.

- TUI tarball installs use the verified native update transaction.
- Cargo/Homebrew/mise installs route to the relevant manager rather than overwriting
  its files. Reuse the completed receipt/admission policy and current useful
  manager guidance; do not infer new overwrite authority from pathname spelling.
- Windows installed GUI updates use the same signed installer/activation mechanism
  as first install, including when winget originally invoked that installer.
  Registry/version metadata stays consistent so winget can detect upgrades.
- GUI-private WSL backends are version-paired dependencies, not independent “latest”
  auto-updates. A new GUI can prepare its own backend version without disrupting a
  running older window/backend.

Update checks/downloads are bounded background jobs. Activation/restart is explicit
and honors unsaved documents, in-flight filesystem saves/mutations, debug sessions
and terminals. Do not kill them to finish an updater. A program that remains running
continues against its original version; new windows use the new verified pair.

### Rollback

Retain at least the previous verified GUI/backend pair and an atomic activation
record. Rollback is a deliberate selection of that known pair, not downloading an
arbitrary older executable from a guessed URL. Keep versioned configuration/recovery
compatibility explicit: an older binary must not silently corrupt or discard a newer
schema. Back up/migrate safely, or refuse an unsupported rollback with a reason.

Cleanup removes only unused owned versions after checking active leases and current/
previous pointers. It never deletes an active binary or interprets an unknown file
in the install tree as disposable. There is no requirement to retain infinite
versions or turn this into a general package manager.

## 9. Uninstall and data ownership

Default Windows uninstall removes owned application versions, launcher, shortcuts
and registration. User preferences/recovery/project data are preserved unless the
user separately approves their removal with explicit paths/scope.

Offer optional cleanup of **Strop-owned backend caches in selected distributions**.
Only act on proven receipts/owned paths; do not touch independent TUI installations,
SSH hosts, Docker containers, language tools, project files or the whole `.config`
tree. In-use backends require an explicit close/keep decision.

Uninstall must **never** unregister, terminate, remove or reset a WSL distro; delete
its home/VHD; change default users; remove SSH credentials; or uninstall unrelated
prerequisites. Winget manages the Windows package, not a right to erase Linux data.

## 10. Integrity, signing, provenance and licensing

Keep these guarantees distinct:

- **SHA256** identifies expected bytes and detects corruption/mismatch. A sidecar
  fetched from the same compromised origin is not an independent publisher signature.
- **Authenticode** establishes Windows publisher/signature policy for the launcher,
  GUI/helpers and installer. Use SHA256 signing and RFC3161 timestamping; verify
  under the application policy and expected publisher, not merely “some signature.”
- **GitHub artifact attestation** establishes build provenance for the expected
  repository/ref/workflow. Generate it for the actual immutable distributed artifacts
  and verify the actor/predicate/source, not just any attestation blob.
- **TLS/origin policy** and immutable release references remain part of bootstrap/
  update trust. Use existing rustls-only/no-OpenSSL dependency policy.

The native GUI can embed the exact Linux-backend digest/catalog for its own release
in signed application resources. This gives the bootstrap a known expected backend
without inventing a new cryptographic update framework. Release ordering must
produce the Linux artifact digests before signing the corresponding GUI bundle.
For independently fetched catalogs, define authenticated verification explicitly;
a mutable latest.json plus an unauthenticated hash is not equivalent.

A trusted code-signing identity and timestamp service are **release prerequisites
owned by the release maintainer**. Research cannot manufacture that identity.
Use the project's chosen certificate/signing service, protect keys/credentials and
record renewal/rotation policy. No self-signed production workaround, repository PFX
or disabled signature check to make CI green. Signing improves trust but does not
promise that a brand-new publisher will never see SmartScreen warnings.

Preserve all upstream notices/licenses and produce a machine-readable dependency/
license inventory or SBOM with the artifact set. GPUI/platform packages can be
private/pinned for the GUI; do not make publishable TUI crates depend on unpublished
workspace/Git-only GUI packages. The existing publication-order check already
protects private workspace dependencies. Audit the resolved graph: source language
or a feature name is not proof that OpenSSL is absent.

## 11. Website and winget

### strop.dev is the primary entry

Keep the site static/simple and extend 0056 AR12's catalog-driven pages for the GUI.
No new application backend or independently maintained version source is needed.

Required presentation:

- **Windows GUI (WSL workspaces)**: clear primary download, supported Windows/WSL
  envelope, signed publisher and one short onboarding explanation.
- **Terminal editor**: existing Linux/macOS install choices and manual verified
  tarballs; Windows users are directed to run the TUI inside WSL, not a nonexistent
  native console port.
- Manual/offline downloads and verification instructions remain available.
- Installation/update/uninstall, architecture, prerequisites, known limits and release
  notes link to the same version/support facts. “No Windows filesystem support yet”
  is stated before installation, not buried in an error.
- Do not advertise GUI/debugger/terminal capabilities before their release ledgers
  are complete. Screenshots/demos drive real released binaries, not staged-only UI.
- Preserve AR12's authoritative/generated installer/site inputs; do not introduce
  a separately drifting Windows/WSL bootstrap or download catalog.

The observed v0.19/v0.30.0 split is the regression example: a browser-level release
check must validate the visible hero, version chip, chosen download and install
command together. Reader-mode text alone cannot validate dynamic release UI.

### Winget

Use one stable proposed identity such as `StropDev.Strop`, validated/reserved before
publication; it is not claimed to exist now. Manifest publisher/name/version match
Installed Apps. Use the actual Nullsoft installer type, user scope, silent switches,
architecture, minimum OS, immutable URL and SHA256 of the **final signed installer**.

Generate the manifest after signing, not from an unsigned build whose digest later
changes. Test install/upgrade/uninstall and metadata detection against a local
manifest before submission. Submit through the normal winget community process;
track acceptance separately. strop.dev adds the winget command only after the
published package is actually available and exercised.

Do not make winget silently install/configure WSL distributions. The Windows package
can install while WSL is absent and provide the explicit first-run setup flow.
No custom package server or duplicate winget-specific updater is needed.

## 12. Release pipeline, tests and promotion state

Evolve the existing tag workflow; do not hand-publish around it. Version changes
remain workspace Cargo version + member pins + checked Cargo.lock. Keep the current
TUI platforms and real artifact verification, not an installer-only gate.

Entry requires completed 0056 installation/catalog/server foundations, 0057 baseline
assurance and 0058 native-worker deployment/assurance migration. These stages support
0061 GUI delivery with Windows artifacts and real onboarding/publication evidence.
Package the exact worker-capable backend; do not introduce another worker installer
or overwrite an unrelated TUI installation/cache.

The 0057/0058 assurance contract applies to bootstrap/version/activation/publication,
not just their message formats: extend the state/claim mapping, proved production
decisions, calibrated models and actual observer/helper/native-effect correspondence.
A transaction proof does not prove a new installer's filesystem calls or editor/
backend identity assumptions. Tag publication requires `model`, `verify`, `tlaps`
and `core-assurance` on the exact candidate plus all signature/artifact/native
installation gates. New effects never ship on a prior core proof alone.

Recommended dependency order:

1. Validate source/version/lock/protocol compatibility and run required TUI/engine/
   debugger/terminal/GUI gates.
2. Build/verify Linux/macOS TUI/backend artifacts and their manifests/digests.
3. Build the native Windows GUI/launcher against the matching backend metadata;
   bundle assets/notices; verify on Windows; sign/timestamp final executables and
   installer; compute final artifact digests.
4. Verify clean install/first-run/backend pairing, actual native GUI+WSL operation,
   update/rollback/uninstall and package metadata.
5. Publish the immutable artifact set/provenance and publishable crates in dependency
   order under tag/version guards. Skip private GUI packages correctly.
6. Promote Homebrew/site/download catalog and submit eligible winget manifests.
7. Verify the public user journey and record channel-specific status before marking
   the release/promotion complete.

Replace “exactly four tarballs means the release is complete” with an explicit
expected artifact matrix that still requires those four TUI products and the
applicable GUI artifacts. Never weaken the static/readelf or real headless parse
gate when adding Windows packaging.

Publication is partly irreversible. A site or winget failure does not justify
rewriting a published tag/artifact or pretending a crate was unpublished. Keep a
resumable ledger: built/verified, immutable artifacts published, site promoted,
channel submitted/available, public checks passed. An “already published” message
requires matching version/provenance verification, not blind success by substring.

### Required installation/upgrade cases

Use fresh disposable Windows profiles/VMs and explicitly owned WSL test distributions
or fixture roots. Never run destructive installer tests against the user's real
home/default distro. No automatic wsl --shutdown as test cleanup.

- Existing WSL2 distro, several distros/users, no default preference and non-default
  user home; exact selection persists without changing system defaults.
- WSL missing/unsupported/uninitialized/slow/permission-limited; GUI remains responsive
  and diagnostics distinguish prerequisites from backend corruption.
- First install, silent install, user paths with spaces/Unicode, non-default location,
  unprivileged execution and consistent Start-menu/Installed-apps identity.
- Corrupt/missing/oversized/incorrect-architecture archive, bad checksum/signature,
  malformed catalog, wrong protocol/backend version, traversal/link archive entries,
  disk-full/interrupted transfer and concurrent install attempts.
- Offline/proxy/restricted distro networking; host download/verified offline bundle
  path works without an unverified fallback or hidden package installation.
- Update while documents are dirty, save/rename outcomes pending, debugger attached
  and terminal running; no forced data loss or orphaned owned process.
- Old/new windows and backend versions coexist safely; rollback selects the matching
  verified pair; schema incompatibility refuses honestly.
- Uninstall preserves projects/preferences by default; optional owned-cache cleanup
  respects active leases and never removes a distro/independent TUI/toolchain.
- Website rendered version/download/install/docs agree with the actual artifact set;
  manual/checksum/provenance links work. Winget is tested only when genuinely available.

Run the existing Compose locked gates plus native Windows build/installer/UI/WSL
lanes. Signatures/attestations are verified after download, not only before upload.
No normal source-test pass, successful extraction or mocked bootstrap response is
installation proof. Record artifact hashes, OS/WSL versions, screenshots and actual
installed/backend versions in the PKG ledger.

## 13. Explicit later scope

- Public winget availability if external review is pending; manifest readiness and
  honest status remain required. Additional stores/managers are not needed to launch.
- MSIX/Store, enterprise all-users MSI, custom WSL images and broad Linux GUI packaging.
- Native Windows workspace/process support, Windows ARM64 and other GUI platforms,
  after 0061's explicit platform/storage milestones.
- Arbitrary adapter/toolchain provisioning, container lifecycle installation or
  managing SSH hosts as a side effect of installing Strop.
- A generalized package manager, cross-version backend compatibility framework,
  always-running updater daemon or cloud account service.

None permits dropping the clean existing-WSL install/update/rollback/uninstall
journey or publishing unsigned/unverified artifacts under a signed-release claim.
Required scope reductions require user approval and 0028's evidence/re-entry ledger.

## 14. Primary references and implementation anchors

- Repository: `.github/workflows/release.yml`, `.github/scripts/publish-order.py`,
  `install.sh`, `crates/strop/src/update.rs`, [0002](0002-build-and-release.md) and
  [0004](0004-website-and-demo.md): current release/update/site contracts.
- [strop.dev](https://strop.dev/) and
  [site repository](https://github.com/stropdev/stropdev.github.io): current public
  install surface and actual ownership; dynamic/static version evidence above.
- [NSIS multi-user documentation](https://nsis.sourceforge.io/Docs/MultiUser/Readme.html):
  standard execution level/current-user installation and silent mode choices.
- [Winget manifest guidance](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest):
  installer types, silent behavior, digests, Installed-apps matching and submission.
- [SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool):
  signature policy, SHA256, timestamping and verification outcomes.
- [GitHub attestation verification](https://cli.github.com/manual/gh_attestation_verify):
  artifact identity, expected repository/workflow/predicate/source checks.
- [WSL commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands) and
  [WSL interoperability/filesystems](https://learn.microsoft.com/en-us/windows/wsl/filesystems):
  distro/user selection, argument/path semantics and destructive system commands.
- [0061](0061-gui-windows-and-wsl.md) and
  [0055](0055-embedded-terminal-tui-and-gui.md): GUI/backend/terminal ownership,
  native execution envelope and actual-surface testing requirements.
- [0058](0058-unified-native-worker.md): native-worker artifact/deployment identity,
  lease-safe cache ownership and the completed assurance migration consumed here.
