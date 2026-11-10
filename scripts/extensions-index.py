#!/usr/bin/env python3
"""Builds extensions/metadata/index.json, the app's own list of metadata
extensions (heronite_metadata_ext::lists::HERONITE_LIST), from the folders
under extensions/metadata/: each folder's source.toml says what the
extension is, and the list ("type": "metadata") names its two files
(source.toml, source.js) by address and SHA-256.
The format is described in docs/EXTENSIONS.md ("Lists of extensions").

Run after adding or changing an extension, and commit the result with it:

    python3 scripts/extensions-index.py            # rewrite the file
    python3 scripts/extensions-index.py --check    # fail when it is out of date
    python3 scripts/extensions-index.py --base https://example.org/ext/ --out /tmp/index.json

The addresses are <base><folder>/<file>; the base defaults to the raw files
of the repository's main branch. No third-party packages.
"""

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

INDEX = 1
TYPE = "metadata"
API = 1
ROOT = Path(__file__).resolve().parent.parent
FOLDER = ROOT / "extensions" / "metadata"
BASE = "https://raw.githubusercontent.com/RafaelGoulartB/Heronite/main/extensions/metadata/"
FILES = ("source.toml", "source.js")
MAX_SIZE = {"source.toml": 64 * 1024, "source.js": 512 * 1024}
ID = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")


def fail(msg):
    sys.exit(f"extensions-index: {msg}")


def entry(folder, base):
    manifest = tomllib.loads((folder / "source.toml").read_text(encoding="utf-8"))
    ext_id = manifest.get("id", "")
    if not ID.match(ext_id) or not 2 <= len(ext_id) <= 40:
        fail(f"{folder.name}: id {ext_id!r} is not valid")
    if ext_id != folder.name:
        fail(f"{folder.name}: its id is {ext_id!r}; name the folder after it")
    if manifest.get("api") != API:
        fail(f"{ext_id}: api must be {API}")
    homepage = manifest.get("homepage")
    if homepage is not None and not homepage.startswith("https://"):
        fail(f"{ext_id}: a list names https addresses only (homepage {homepage!r})")
    network = manifest.get("network", {})
    out = {
        "id": ext_id,
        "name": manifest["name"],
        "version": manifest["version"],
        "api": manifest["api"],
        "author": manifest["author"],
    }
    if homepage is not None:
        out["homepage"] = homepage
    out.update(
        {
            "kind": manifest["kind"],
            "adult": bool(manifest.get("adult", False)),
            "description": manifest["description"].strip(),
            "fields": manifest["provides"]["fields"],
            "applies": manifest.get("applies", {}),
            "hosts": network.get("hosts", []),
            "images": network.get("images", []),
            "settings": [{"label": s["label"], "required": bool(s.get("required", False))} for s in manifest.get("settings", [])],
        }
    )
    files = {}
    for name in FILES:
        data = (folder / name).read_bytes()
        if not 0 < len(data) <= MAX_SIZE[name]:
            fail(f"{ext_id}: {name} is {len(data)} bytes; it may be 1 to {MAX_SIZE[name]}")
        files[name] = {"url": f"{base}{folder.name}/{name}", "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
    out["files"] = files
    return out


def build(base):
    if not base.startswith("https://"):
        fail("the base address must be https")
    if not base.endswith("/"):
        base += "/"
    folders = sorted(p for p in FOLDER.iterdir() if p.is_dir() and (p / "source.toml").is_file())
    return {"index": INDEX, "type": TYPE, "name": "Heronite", "extensions": [entry(f, base) for f in folders]}


def text(index):
    return json.dumps(index, indent=2, ensure_ascii=False) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", default=BASE, help="where the folders are served (default: the repository's main branch)")
    parser.add_argument("--out", type=Path, default=FOLDER / "index.json")
    parser.add_argument("--check", action="store_true", help="only say whether the file is up to date")
    args = parser.parse_args()
    new = text(build(args.base))
    if args.check:
        old = args.out.read_text(encoding="utf-8") if args.out.exists() else ""
        if old != new:
            fail(f"{args.out} is out of date: run python3 scripts/extensions-index.py")
        return
    args.out.write_text(new, encoding="utf-8")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
