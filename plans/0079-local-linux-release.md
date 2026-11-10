# 0079 — Local Linux/WSL releases

Status: owner-approved release-model cutover for 0.47.2. This supersedes the
hosted publication parts of 0002 §5 and the multi-platform release promises in
later historical plans. Those plans remain implementation history, not current
publication instructions.

## Scope

The supported prebuilt target is `x86_64-unknown-linux-musl`, covering Linux and
WSL2 on x86_64. Linux aarch64, macOS and native Windows binaries are not produced
or verified by the release process. Existing releases remain available; a new
catalog advertises only artifacts actually built for that release.

Distribution is a checksummed GitHub release tarball plus `catalog.json`. The
catalog remains the contract for `install.sh`, `strop update`, remote worker
selection and strop.dev. crates.io and Homebrew are historical installation
channels and receive no new versions under this model.

## Local qualification

Releases are built and qualified on the maintainer's Linux/WSL workstation with
Docker BuildKit. GitHub Actions does not rebuild or publish the release. The
routine local gate is:

1. An exact clean committed checkout; the workspace version and tag agree.
2. `docker compose run --build --rm test` (fmt, clippy, all tests).
3. `python3 verification/check.py --release`, `sh tests/release-catalog.sh` and
   `sh tests/install.sh`.
4. The changed feature's real UI/CLI smoke.
5. `.github/scripts/release-local.sh package <x.y.z>`, which builds through
   Compose into `dist/v<x.y.z>/`, then runs checksum, static-link, version,
   headless-parse, completion and find-budget (0093 §7) qualification against the
   extracted shipping binary.
6. The package-generated freeze `--check` validates the exact commit against the
   completion report produced from that same shipping binary (not the previous
   release's committed report).

The model/proof/native assurance lanes remain CI and explicitly requested deep
checks; they are not hidden or reported as run by the routine local release.
BuildKit caches are performance state, never qualification evidence. Release
outputs stay in a version-specific ignored `dist/v<x.y.z>/` directory and may be
copied to cold storage without rebuilding them.

## Publication

`.github/scripts/release-local.sh` implements `qualify`, `package`, `draft`,
`publish` and `verify`. Publication uses the exact local bytes:

1. Push the qualified commit and immutable `v<x.y.z>` tag.
2. `draft` verifies the remote tag identity and creates a draft GitHub release
   with the tarball, sidecar, completion report, notes and catalog.
3. Review the draft assets and notes. `publish` marks that existing draft public
   and latest; it never rebuilds or replaces assets.
4. `verify` compares the public catalog to the local artifact facts and installs
   the public tarball into a fresh directory.
5. Vendor `CHANGELOG.md` and `install.sh` into `stropdev/stropdev.github.io`,
   rebuild its changelog, push, and dispatch `rebuild` when needed. Fetch
   strop.dev and confirm the new version, changelog and installer.

`.github/workflows/release.yml` is dispatch-only policy guidance with read-only
permissions. A green documentation workflow is not release evidence. A release
is complete only when the public GitHub assets, catalog-driven installation and
website all prove the new version.
