#!/usr/bin/env python3
"""Bumps the workspace version in Cargo.toml and Cargo.lock.

    scripts/bump-version.py patch|minor|major|X.Y.Z

Prints the new version. Every crate inherits `[workspace.package] version`,
so this is the only number to change; the lock file gets the same version
for the workspace's own packages (the ones without a `source`).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    arg = sys.argv[1]
    toml_path = ROOT / "Cargo.toml"
    toml = toml_path.read_text()
    section = re.search(r"^\[workspace\.package\]\n(?:(?!^\[).*\n)*", toml, re.M)
    found = section and re.search(r'^version = "(\d+)\.(\d+)\.(\d+)"$', section.group(0), re.M)
    if not found:
        sys.exit('no version = "X.Y.Z" in [workspace.package]')
    major, minor, patch = map(int, found.groups())
    if arg == "major":
        new = f"{major + 1}.0.0"
    elif arg == "minor":
        new = f"{major}.{minor + 1}.0"
    elif arg == "patch":
        new = f"{major}.{minor}.{patch + 1}"
    elif re.fullmatch(r"\d+\.\d+\.\d+", arg):
        new = arg
    else:
        sys.exit(f"unknown bump: {arg}")

    start = section.start() + found.start()
    end = section.start() + found.end()
    toml_path.write_text(toml[:start] + f'version = "{new}"' + toml[end:])

    lock_path = ROOT / "Cargo.lock"
    blocks = lock_path.read_text().split("\n[[package]]\n")
    for i, block in enumerate(blocks):
        if re.search(r'^name = "heronite(-[a-z-]+)?"$', block, re.M) and "\nsource = " not in block:
            blocks[i] = re.sub(r'^version = ".*"$', f'version = "{new}"', block, count=1, flags=re.M)
    lock_path.write_text("\n[[package]]\n".join(blocks))
    print(new)


if __name__ == "__main__":
    main()
