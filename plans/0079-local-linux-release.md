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

1. The changed feature's real UI/CLI smoke.
2. `.github/scripts/release-local.sh package <x.y.z>`. It first runs
   `qualify` on the same checkout: an exact clean committed tree whose
   workspace version matches, a green GitHub `ci` run for HEAD,
   `sh tests/release-catalog.sh`, `sh tests/install.sh` and the Compose `test`
   and `proofs` gates. Only then does it build through Compose into
   `dist/v<x.y.z>/` and run checksum, static-link, version, headless-parse
   and the find/search/symbols/history budgets (0093 §7, 0099 §7, 0101 §5.1)
   against the extracted shipping binary. `qualify` alone runs the gates
   without packaging.

*2026-11-17 (plan 0108 D3):* `package` used to trust that `qualify` had been
run; it now runs it itself, so no artifact is built from an unqualified
commit and no receipt file is needed. The TLC `model` lane is deleted.
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
