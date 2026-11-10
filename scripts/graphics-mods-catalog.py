#!/usr/bin/env python3
"""Builds data/graphics-mods.json, the catalog of graphics mods per game
that Heronite fetches at runtime (heronite_launch::gfxmods::catalog).

Run by a maintainer, never by the app or CI. Its output lands only through a
reviewed pull request:

    python3 scripts/graphics-mods-catalog.py            # rewrite the file
    python3 scripts/graphics-mods-catalog.py --check    # print the diff only

Sources:
- RenoDX's mod list, the wiki page's markdown
  (https://github.com/clshortfuse/renodx/wiki/mods). Only snapshot builds
  hosted on GitHub (github.com release assets, *.github.io) are taken;
  Nexus, Discord and other hosts are left out (they need a login or are not
  direct downloads). Rows that share one add-on become one entry with
  every title.

Steam app ids and executable names are hand edits (the wiki has neither).
They survive a rebuild: `steam` and `exe` of an existing entry are kept, and
so are entries marked `"manual": true`.
"""

import argparse
import difflib
import json
import re
import sys
import urllib.parse
import urllib.request
from pathlib import Path

SCHEMA = 1
WIKI = "https://raw.githubusercontent.com/wiki/clshortfuse/renodx/Mods.md"
OUT = Path(__file__).resolve().parent.parent / "data" / "graphics-mods.json"

ADDON = re.compile(r"\((https://[^)\s]+\.addon(?:64|32))\)")
STATUS_NOTE = re.compile(r'\[:(white_check_mark|construction):\]\(#\s*"([^"]*)"\)')
GENERIC = {
    "ue-extended": ("unreal", "Unreal Engine Extended"),
    "unityengine": ("unity", "Unity Engine"),
}


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "heronite-catalog-builder"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read().decode("utf-8")


def allowed(url):
    parts = urllib.parse.urlsplit(url)
    host = (parts.hostname or "").lower()
    plain = parts.scheme == "https" and parts.username is None and ":" not in parts.netloc and "\\" not in url
    return plain and (host == "github.com" or (host.endswith(".github.io") and host != "github.io"))


def slug(url):
    name = url.rsplit("/", 1)[1]
    return re.sub(r"\.addon(64|32)$", "", name.removeprefix("renodx-")).lower()


def cell_text(cell):
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", cell)
    return re.sub(r"\s+", " ", text).strip()


def status_of(cell):
    m = STATUS_NOTE.search(cell)
    if m:
        return ("working" if m.group(1) == "white_check_mark" else "testing"), m.group(2).strip()
    if ":white_check_mark:" in cell:
        return "working", ""
    if ":construction:" in cell:
        return "testing", ""
    return None, ""


def parse_wiki(md):
    """Per-game rows of the main table, and the generic engine mods."""
    games, generic = [], {}
    start = md.find("# List")
    end = md.find("## Multi-Game Mods")
    for line in md[start:end].splitlines():
        if not line.startswith("|") or line.startswith("| :") or line.startswith("| Name"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 4:
            continue
        name, links, status_cell = cell_text(cells[0]), cells[2], cells[3]
        status, note = status_of(status_cell)
        urls = [u for u in ADDON.findall(links) if allowed(u)]
        if not name or not status or not urls:
            continue
        mod = {"slug": slug(urls[0]), "status": status}
        for u in urls:
            mod["addon32" if u.endswith(".addon32") else "addon64"] = u
        if note:
            mod["note"] = note
        games.append({"name": name, "renodx": mod})
    for line in md[end:].splitlines():
        for u in ADDON.findall(line):
            key = GENERIC.get(slug(u))
            if key and allowed(u) and key[0] not in generic:
                generic[key[0]] = {"name": key[1], "slug": slug(u), "status": "working", "addon64": u}
    return games, generic


def build(previous):
    games, generic = parse_wiki(fetch(WIKI))
    old = {g["id"]: g for g in previous.get("games", [])}
    by_id = {}
    for g in games:
        gid = g["renodx"]["slug"]
        if gid in by_id:
            by_id[gid]["match"]["title"].append(g["name"])
            continue
        prev = old.pop(gid, {})
        match = {"title": [g["name"]]}
        for key in ("steam", "exe"):
            if prev.get("match", {}).get(key):
                match[key] = prev["match"][key]
        by_id[gid] = {"id": gid, "match": match, "renodx": g["renodx"]}
    out = list(by_id.values()) + [g for g in old.values() if g.get("manual")]
    out.sort(key=lambda g: g["id"])
    return {"schema": SCHEMA, "source": "https://github.com/clshortfuse/renodx/wiki/mods", "generic": generic, "games": out}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="print the diff, write nothing")
    args = ap.parse_args()
    previous = json.loads(OUT.read_text("utf-8")) if OUT.exists() else {}
    text = json.dumps(build(previous), indent=1, ensure_ascii=False) + "\n"
    if args.check:
        old = OUT.read_text("utf-8") if OUT.exists() else ""
        sys.stdout.writelines(difflib.unified_diff(old.splitlines(True), text.splitlines(True), "old", "new"))
        return
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(text, "utf-8", newline="\n")
    print(f"{OUT}: {len(json.loads(text)['games'])} games")


if __name__ == "__main__":
    main()
