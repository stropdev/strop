#!/usr/bin/env python3
"""Generate and verify the strop release catalog, and keep the promotion ledger.

The catalog is the single source of truth for current version / artifact /
digest facts (0056 AR12): install.sh, `strop update` and the website all
resolve "latest" through it instead of scraping APIs independently. It is
attached to every GitHub release, so
https://github.com/stropdev/strop/releases/latest/download/catalog.json
always serves the catalog of the latest release — that URL is the documented
handoff to the website repo (stropdev/stropdev.github.io), which fetches it
on rebuild.

WK05 (0058): the catalog also carries the worker compatibility manifest —
the wire protocol version, the minimum editor version that can drive a
worker at all, and the target triples whose artifacts embed worker mode —
plus per-artifact byte sizes, so an editor makes its deploy-vs-fallback
decision from catalog facts alone and verifies uploaded bytes against
recorded digest+size. The worker is the same static binary (`strop
--worker-stdio`), so the manifest's target list is exactly the artifact
target list; the release workflow extracts protocol/min-editor from the
pinned source constants (never hand-edited here).

The promotion ledger records what was promoted when — crates.io, homebrew
tap, GitHub release, site redeploy, public download facts — so a partially
promoted release is observable and resumable instead of hiding behind a
misleading fully-promoted status.

Subcommands:
  catalog   build dist/catalog.json from the verified build outputs
  verify    compare the locally built catalog against the publicly served one
  ledger    record one promotion step (idempotent per step name)
"""
import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = 1
PRODUCT = "strop"
DEFAULT_REPO = "stropdev/strop"
ARTIFACT_RE = re.compile(r"^strop-(?P<version>[^-]+)-(?P<target>.+)\.tar\.gz$")


def utcnow() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_version(tag: str) -> tuple[int, int, int]:
    parts = tag.removeprefix("v").split(".")
    if len(parts) != 3 or not all(part.isdigit() for part in parts):
        raise ValueError(f"not a semver triple: {tag}")
    return int(parts[0]), int(parts[1]), int(parts[2])


def worker_manifest(version: str, targets: list[str], protocol: int,
                    min_editor: str) -> dict:
    """The WK05 compatibility manifest, validated against the release facts.

    min_editor must not postdate the release itself: a catalog claiming its
    worker needs a newer editor than the release it ships with is a lie.
    """
    if protocol < 1:
        raise ValueError(f"worker protocol must be >= 1, got {protocol}")
    if parse_version(min_editor) > parse_version(version):
        raise ValueError(
            f"worker min_editor {min_editor} postdates release {version}")
    return {
        "protocol": protocol,
        "min_editor": min_editor,
        "targets": sorted(targets),
    }


def build_catalog(tag: str, dist: Path, base_url: str, published_at: str,
                  worker_protocol: int, worker_min_editor: str) -> dict:
    version = tag.removeprefix("v")
    artifacts = []
    for tarball in sorted(dist.glob("strop-*.tar.gz")):
        match = ARTIFACT_RE.match(tarball.name)
        if not match:
            raise ValueError(f"unexpected artifact name: {tarball.name}")
        if match.group("version") != version:
            raise ValueError(f"{tarball.name} does not match tag {tag}")
        sidecar = tarball.with_name(tarball.name + ".sha256")
        if not sidecar.is_file():
            raise ValueError(f"missing checksum sidecar for {tarball.name}")
        recorded = sidecar.read_text(encoding="utf-8").split()[0]
        computed = sha256_of(tarball)
        if recorded != computed:
            raise ValueError(f"sidecar digest does not match {tarball.name}")
        artifacts.append({
            "target": match.group("target"),
            "name": tarball.name,
            "sha256": computed,
            "bytes": tarball.stat().st_size,
            "url": f"{base_url}/{tag}/{tarball.name}",
        })
    if not artifacts:
        raise ValueError(f"no strop-*.tar.gz artifacts in {dist}")
    return {
        "schema": SCHEMA,
        "product": PRODUCT,
        "version": version,
        "tag": tag,
        "published_at": published_at,
        "artifacts": artifacts,
        "worker": worker_manifest(
            version, [a["target"] for a in artifacts],
            worker_protocol, worker_min_editor),
    }


def facts(catalog: dict) -> dict:
    """The promotion-relevant facts: version, tag, artifact identities and
    the worker compatibility manifest."""
    return {
        "version": catalog["version"],
        "tag": catalog["tag"],
        "artifacts": [
            {"target": a["target"], "name": a["name"], "sha256": a["sha256"],
             "bytes": a["bytes"], "url": a["url"]}
            for a in catalog["artifacts"]
        ],
        "worker": catalog["worker"],
    }


def record_step(path: Path, tag: str, step: str, status: str, detail: str | None,
                at: str | None) -> dict:
    if path.is_file():
        ledger = json.loads(path.read_text(encoding="utf-8"))
    else:
        ledger = {"schema": SCHEMA, "tag": tag, "steps": []}
    entry = {"step": step, "status": status, "at": at or utcnow()}
    if detail:
        entry["detail"] = detail
    steps = [existing for existing in ledger["steps"] if existing["step"] != step]
    steps.append(entry)
    ledger["steps"] = steps
    path.write_text(json.dumps(ledger, indent=2) + "\n", encoding="utf-8")
    return ledger


def cmd_catalog(args: argparse.Namespace) -> None:
    catalog = build_catalog(args.tag, args.dist, args.base_url,
                            args.published_at or utcnow(),
                            args.worker_protocol, args.worker_min_editor)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    print(f"catalog: {len(catalog['artifacts'])} artifacts for {catalog['tag']} "
          f"(worker protocol {catalog['worker']['protocol']})")


def cmd_verify(args: argparse.Namespace) -> None:
    local = facts(json.loads(args.local.read_text(encoding="utf-8")))
    public = facts(json.loads(args.public.read_text(encoding="utf-8")))
    if local != public:
        sys.exit("public catalog facts diverge from the build outputs:\n"
                 f"  local:  {json.dumps(local, sort_keys=True)}\n"
                 f"  public: {json.dumps(public, sort_keys=True)}")
    print(f"public catalog matches build outputs for {local['tag']}")


def cmd_ledger(args: argparse.Namespace) -> None:
    ledger = record_step(args.file, args.tag, args.step, args.status, args.detail, args.at)
    print(f"ledger: {args.step}={args.status} ({len(ledger['steps'])} steps recorded)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    catalog = commands.add_parser("catalog", help="build the release catalog from dist/")
    catalog.add_argument("--tag", required=True, help="release tag, e.g. v0.33.0")
    catalog.add_argument("--dist", type=Path, default=Path("dist"))
    catalog.add_argument("--out", type=Path, default=Path("dist/catalog.json"))
    catalog.add_argument("--base-url",
                         default=f"https://github.com/{DEFAULT_REPO}/releases/download",
                         help="download base URL recorded in artifact urls")
    catalog.add_argument("--published-at", help="ISO timestamp; default: now (UTC)")
    catalog.add_argument("--worker-protocol", type=int, required=True,
                         help="wire protocol version the embedded worker speaks "
                         "(extracted from strop-worker-protocol source by the "
                         "release workflow)")
    catalog.add_argument("--worker-min-editor", required=True,
                         help="oldest editor version that can drive a worker "
                         "(extracted from strop-worker-deploy source by the "
                         "release workflow)")
    catalog.set_defaults(run=cmd_catalog)

    verify = commands.add_parser("verify", help="compare local and publicly served catalogs")
    verify.add_argument("--local", type=Path, required=True)
    verify.add_argument("--public", type=Path, required=True)
    verify.set_defaults(run=cmd_verify)

    ledger = commands.add_parser("ledger", help="record one promotion step")
    ledger.add_argument("--file", type=Path, required=True)
    ledger.add_argument("--tag", required=True)
    ledger.add_argument("--step", required=True)
    ledger.add_argument("--status", required=True)
    ledger.add_argument("--detail")
    ledger.add_argument("--at", help="ISO timestamp; default: now (UTC)")
    ledger.set_defaults(run=cmd_ledger)

    args = parser.parse_args()
    try:
        args.run(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
