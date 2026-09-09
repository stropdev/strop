#!/usr/bin/env python3
"""Emit one exact changelog section; incomplete release notes fail publication."""
import argparse
from pathlib import Path


def notes_for_version(changelog: str, version: str) -> str:
    version = version.removeprefix("v")
    lines = changelog.splitlines(keepends=True)
    headings = [
        index
        for index, line in enumerate(lines)
        if line.startswith("## ") and line[3:].split(maxsplit=1)[:1] == [version]
    ]
    if len(headings) != 1:
        raise ValueError(f"expected exactly one changelog section for {version}; found {len(headings)}")
    start = headings[0] + 1
    end = next((index for index in range(start, len(lines)) if lines[index].startswith("## ")), len(lines))
    body = "".join(lines[start:end]).strip()
    if not any(line.strip() and not line.lstrip().startswith("#") for line in body.splitlines()):
        raise ValueError(f"changelog section for {version} has no release notes")
    return body + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="release version, with or without a leading v")
    parser.add_argument("changelog", nargs="?", type=Path, default=Path("CHANGELOG.md"))
    args = parser.parse_args()
    try:
        notes = notes_for_version(args.changelog.read_text(encoding="utf-8"), args.version)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(notes, end="")


if __name__ == "__main__":
    main()
