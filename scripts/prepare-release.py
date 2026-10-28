#!/usr/bin/env python3
"""Use the published release tag as the workspace version in the CI checkout.

Manual builds keep the committed version. Release builds run `cargo update
--workspace` afterwards to synchronize workspace entries in Cargo.lock while
preserving locked third-party dependencies, then build with --locked.
"""

import os
import re
import sys
import tomllib
from pathlib import Path


# Cargo's SemVer format, including prereleases and build metadata. Requiring the
# v prefix also keeps the existing release asset naming convention.
NUMBER = r"(?:0|[1-9][0-9]*)"
PRERELEASE_ID = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
RELEASE_TAG = re.compile(
    rf"v({NUMBER}\.{NUMBER}\.{NUMBER}"
    rf"(?:-{PRERELEASE_ID}(?:\.{PRERELEASE_ID})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?)"
)


def prepare_version(manifest: Path, event: str, tag: str) -> str:
    text = manifest.read_text(encoding="utf-8")
    version = tomllib.loads(text)["workspace"]["package"]["version"]
    if event != "release":
        return version

    match = RELEASE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"Invalid release tag {tag!r}; expected vMAJOR.MINOR.PATCH (SemVer).")
    version = match[1]

    # Rewrite only the workspace table; dependency versions and comments survive.
    table = re.search(r"(?ms)^\[workspace\.package\][^\n]*\n(.*?)(?=^\[|\Z)", text)
    if table is None:
        raise ValueError("Could not find [workspace.package] in Cargo.toml.")
    body, count = re.subn(
        r"(?m)^([ \t]*version[ \t]*=[ \t]*)(?:\"[^\"\n]*\"|'[^'\n]*')",
        lambda field: f'{field[1]}"{version}"',
        table[1],
    )
    if count != 1:
        raise ValueError("Expected one version field in [workspace.package].")
    updated = text[:table.start(1)] + body + text[table.end(1):]
    if updated != text:
        manifest.write_text(updated, encoding="utf-8", newline="\n")
    return version


def main() -> None:
    try:
        version = prepare_version(
            Path("Cargo.toml"),
            os.environ.get("GITHUB_EVENT_NAME", ""),
            os.environ.get("RELEASE_TAG", ""),
        )
    except (KeyError, OSError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        sys.exit(1)
    print(f"Build version: {version}")
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8", newline="\n") as stream:
            stream.write(f"version={version}\n")


if __name__ == "__main__":
    main()
