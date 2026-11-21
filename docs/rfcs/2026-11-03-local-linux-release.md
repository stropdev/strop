---
rfc: "2026-11-03-local-linux-release"
title: "Local Linux and WSL release procedure"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-11-03"
updated: "2026-11-21"
issue: "https://github.com/stropdev/strop/issues/174"
milestone: "v0.47.2"
supersedes: ["2026-09-13-distribution-and-wsl-onboarding"]
superseded_by: []
blocked_on: []
legacy_id: "0079"
---

# RFC: Local Linux and WSL release procedure

v0.47.2 switched publication to the locally qualified x86_64-unknown-linux-musl tarball, checksum and GitHub catalog; hosted release builds, macOS/arm64 artifacts and new Cargo/Homebrew packages stopped. The procedure is still implemented by .github/scripts/release-local.sh; 0086 retired its completion freeze and 0108 made packaging run qualification itself. The release script attaches the tarball, checksum and catalog; it sends the notes as the release body. There is no completion-report asset or separate hosted release workflow.

[Tracking issue #174](https://github.com/stropdev/strop/issues/174) · [Published v0.47.2](https://github.com/stropdev/strop/releases/tag/v0.47.2) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0079-local-linux-release.md).

Supersedes: [0062](2026-09-13-distribution-and-wsl-onboarding.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

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
   `sh tests/release-catalog.sh`, `sh tests/release-candidate.sh`,
   `sh tests/install.sh` and the Compose `test` and `proofs` gates.
   Only then does it build through Compose into a fresh candidate directory
   under `dist/v<x.y.z>/<full-commit>/` and run checksum, static-link, version, headless-parse
   and the find/search/symbols/history budgets (0093 §7, 0099 §7, 0101 §5.1)
   against the extracted shipping binary. `qualify` alone runs the gates
   without packaging.

*2026-11-17 (plan 0108 D3):* `package` used to trust that `qualify` had been
run; it now runs it itself, so no artifact is built from an unqualified
commit and no receipt file is needed. The TLC `model` lane is deleted.
BuildKit caches are performance state, never qualification evidence.

*2026-11-21 ([#252](https://github.com/stropdev/strop/issues/252)):*
The same version can have multiple candidate commits. `package` never adopts
an existing tarball, even one with a matching sidecar. After the shipping
binary passes, the script atomically renames `.building` to a bundle ID derived
from the commit and the checksums of the archive, sidecar, catalog and notes.
It prints the resulting directory. Old version-only outputs and other commits'
bundles are untouched. To retry the **same** candidate, first move its entire
`dist/v<x.y.z>/<full-commit>/` directory aside, including any partial build.
Keep the path and contents together if archiving a qualified bundle.

## Publication

`.github/scripts/release-local.sh` implements `qualify`, `package`, `draft`,
`publish` and `verify`. Publication uses the exact local bytes:

1. Push the qualified commit and immutable `v<x.y.z>` tag.
2. `draft` checks the clean HEAD against the remote tag, resolves its qualified
   bundle and checks the bytes against the bundle ID. It uploads the tarball,
   sidecar and catalog, and uses the notes as the release body.
3. Review the draft assets and notes. `publish` repeats the source and bundle
   checks, then downloads the draft's assets and compares their names and
   contents and the release body to the local bundle before marking the draft
   public and latest. Neither promotion step rebuilds or replaces assets.
4. `verify` compares the public catalog to the local artifact facts and installs
   the public tarball into a fresh directory.
5. Vendor `CHANGELOG.md` and `install.sh` into `stropdev/stropdev.github.io`,
   rebuild its changelog, push, and dispatch `rebuild` when needed. Fetch
   strop.dev and confirm the new version, changelog and installer.

`.github/workflows/release.yml` is dispatch-only policy guidance with read-only
permissions. A green documentation workflow is not release evidence. A release
is complete only when the public GitHub assets, catalog-driven installation and
website all prove the new version.
