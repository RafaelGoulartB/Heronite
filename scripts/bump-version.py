#!/usr/bin/env python3
"""Bumps the workspace version in Cargo.toml and Cargo.lock.

    scripts/bump-version.py patch|minor|major|X.Y.Z

Prints the new version. Every crate inherits `[workspace.package] version`,
so this is the only number to change; the lock file gets the same version
for the workspace's own packages (the ones without a `source`). The new
version also goes on top of the AppStream data's releases (with a link to the
release notes), which software centres (and Flathub) show, and in the COPR's
spec (Version, with a %changelog entry).
"""
import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC_AUTHOR = "RafaGoulartB <rafagoulartb@gmail.com>"


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
        if re.search(r'^name = "heronite(-[a-z0-9-]+)?"$', block, re.M) and "\nsource = " not in block:
            blocks[i] = re.sub(r'^version = ".*"$', f'version = "{new}"', block, count=1, flags=re.M)
    lock_path.write_text("\n[[package]]\n".join(blocks))

    metainfo_path = ROOT / "packaging/linux/app.heronite.Heronite.metainfo.xml"
    metainfo = metainfo_path.read_text()
    if f'<release version="{new}"' not in metainfo:
        today = datetime.date.today().isoformat()
        notes = f"https://github.com/RafaelGoulartB/Heronite/releases/tag/v{new}"
        entry = rf'\1  <release version="{new}" date="{today}">\n\1    <url type="details">{notes}</url>\n\1  </release>\n'
        metainfo, n = re.subn(r"^(\s*)<releases>\n", rf"\g<0>{entry}", metainfo, count=1, flags=re.M)
        if n != 1:
            sys.exit(f"no <releases> in {metainfo_path}")
        metainfo_path.write_text(metainfo)

    spec_path = ROOT / "packaging/copr/heronite.spec"
    spec = spec_path.read_text()
    spec, n = re.subn(r"^(Version:\s+)\S+$", rf"\g<1>{new}", spec, count=1, flags=re.M)
    if n != 1:
        sys.exit(f"no Version: in {spec_path}")
    spec = re.sub(r"^(Release:\s+)\d+", r"\g<1>1", spec, count=1, flags=re.M)
    if not re.search(rf"^\* .* - {re.escape(new)}-1$", spec, re.M):
        # English names whatever the locale: rpm only reads these.
        today = datetime.date.today()
        day = "Mon Tue Wed Thu Fri Sat Sun".split()[today.weekday()]
        month = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()[today.month - 1]
        entry = f"* {day} {month} {today.day:02d} {today.year} {SPEC_AUTHOR} - {new}-1\n- Update to {new}\n\n"
        spec, n = re.subn(r"^%changelog\n", rf"\g<0>{entry}", spec, count=1, flags=re.M)
        if n != 1:
            sys.exit(f"no %changelog in {spec_path}")
    spec_path.write_text(spec)
    print(new)


if __name__ == "__main__":
    main()
