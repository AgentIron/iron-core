#!/usr/bin/env python3

from __future__ import annotations

import re
import sys
from pathlib import Path


def bump(version: str, release_type: str) -> str:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if match is None:
        raise ValueError(f"unsupported Cargo version: {version}")

    major, minor, patch = (int(part) for part in match.groups())

    if release_type == "patch":
        patch += 1
    elif release_type == "minor":
        minor += 1
        patch = 0
    elif release_type == "major":
        major += 1
        minor = 0
        patch = 0
    else:
        raise ValueError(f"unsupported release type: {release_type}")

    return f"{major}.{minor}.{patch}"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: bump_cargo_version.py <patch|minor|major>", file=sys.stderr)
        return 1

    release_type = sys.argv[1]
    cargo_toml = Path("Cargo.toml")
    content = cargo_toml.read_text(encoding="utf-8")

    package_match = re.search(
        r"(?ms)^\[package\]\n(?P<body>.*?)(?=^\[|\Z)",
        content,
    )
    if package_match is None:
        print("missing [package] section in Cargo.toml", file=sys.stderr)
        return 1

    package_body = package_match.group("body")
    version_match = re.search(r'(?m)^version = "(?P<version>[^"]+)"$', package_body)
    if version_match is None:
        print("missing package version in Cargo.toml", file=sys.stderr)
        return 1

    current_version = version_match.group("version")
    next_version = bump(current_version, release_type)
    updated_body = package_body.replace(
        f'version = "{current_version}"',
        f'version = "{next_version}"',
        1,
    )
    start, end = package_match.span("body")
    updated_content = content[:start] + updated_body + content[end:]
    cargo_toml.write_text(updated_content, encoding="utf-8")

    print(next_version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
