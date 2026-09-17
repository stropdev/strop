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


def build_catalog(tag: str, dist: Path, base_url: str, published_at: str) -> dict:
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
    }


def facts(catalog: dict) -> dict:
    """The promotion-relevant facts: version, tag and artifact identities."""
    return {
        "version": catalog["version"],
        "tag": catalog["tag"],
        "artifacts": [
            {"target": a["target"], "name": a["name"], "sha256": a["sha256"], "url": a["url"]}
            for a in catalog["artifacts"]
        ],
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
                            args.published_at or utcnow())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    print(f"catalog: {len(catalog['artifacts'])} artifacts for {catalog['tag']}")


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
