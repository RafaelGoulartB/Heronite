#!/usr/bin/env python3
"""Compute the affected Cargo workspace packages for a pull request.

On pushes to main, workflow_dispatch, or changes to workspace-wide files
(Cargo.toml, Cargo.lock, .cargo/, vendored GPUI patches, CI scripts), falls
back to `--workspace`. On pull requests touching a subset of crates, prints
the minimal `-p <pkg>` flags covering every changed crate plus all of its
transitive reverse dependents in the workspace.

When any Rust source file under `app/src/` or `crates/*/src/` changes, the
test arguments also include `heronite-i18n` (without expanding its reverse
dependents) so `crates/i18n/tests/coverage.rs` still verifies `tr!` keys on
every pull request.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Changes to any of these paths force a full `--workspace` run.
GLOBAL_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "rustfmt.toml",
    ".github/workflows/ci.yml",
    "scripts/ci-deps.sh",
    "scripts/ci-affected.py",
}
GLOBAL_PREFIXES = (
    ".cargo/",
    "vendor/gpui-pre-linux/",
    "vendor/gpui-pre-windows/",
)

# Files outside crate directories that specific crates embed or test directly.
EXTRA_PATH_PACKAGES: list[tuple[str, tuple[str, ...]]] = [
    ("data/", ("heronite-launch",)),
    ("extensions/", ("heronite-metadata-ext", "heronite")),
    ("scripts/extensions-index.py", ("heronite-metadata-ext",)),
    ("packaging/windows/heronite.ico", ("heronite", "heronite-epic-login")),
    ("assets/screenshots/", ()),
    ("assets/", ("heronite",)),
]

# Non-Rust paths that do not affect `cargo clippy` or `cargo test`.
# Anything not matched here, by a crate, or by EXTRA_PATH_PACKAGES safely
# falls back to `--workspace`.
NON_RUST_FILES = {
    "LICENSE",
    "cliff.toml",
    ".git-blame-ignore-revs",
    ".gitattributes",
    ".gitignore",
}
NON_RUST_PREFIXES = (
    "docs/",
    "licenses/",
    "packaging/",
    ".copr/",
    ".github/",
    "scripts/",
)


def load_workspace() -> tuple[dict[str, Path], dict[str, set[str]]]:
    """Return (pkg_name -> rel_dir, pkg_name -> direct workspace reverse deps)."""
    root_toml = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    members_raw = root_toml.get("workspace", {}).get("members", [])

    pkg_dirs: dict[str, Path] = {}
    pkg_manifests: dict[str, dict] = {}
    for pattern in members_raw:
        matches = sorted(ROOT.glob(pattern))
        for member_dir in matches:
            manifest_path = member_dir / "Cargo.toml"
            if not manifest_path.is_file():
                continue
            data = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
            name = data.get("package", {}).get("name")
            if not name:
                continue
            pkg_dirs[name] = member_dir.relative_to(ROOT)
            pkg_manifests[name] = data

    rev_deps: dict[str, set[str]] = {name: set() for name in pkg_dirs}

    def collect_deps(table: dict) -> set[str]:
        found: set[str] = set()
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            deps = table.get(section, {})
            if not isinstance(deps, dict):
                continue
            for dep_key, spec in deps.items():
                pkg_name = dep_key
                if isinstance(spec, dict) and isinstance(spec.get("package"), str):
                    pkg_name = spec["package"]
                if pkg_name in pkg_dirs:
                    found.add(pkg_name)
        return found

    for name, manifest in pkg_manifests.items():
        deps = collect_deps(manifest)
        for target_table in manifest.get("target", {}).values():
            if isinstance(target_table, dict):
                deps |= collect_deps(target_table)
        for dep in deps:
            rev_deps[dep].add(name)

    return pkg_dirs, rev_deps


def transitive_reverse_deps(seeds: set[str], rev_deps: dict[str, set[str]]) -> set[str]:
    affected = set(seeds)
    stack = list(seeds)
    while stack:
        cur = stack.pop()
        for dependent in rev_deps.get(cur, ()):
            if dependent not in affected:
                affected.add(dependent)
                stack.append(dependent)
    return affected


def git_changed_files(base: str, head: str) -> list[str] | None:
    try:
        out = subprocess.check_output(
            ["git", "diff", "--name-only", base, head],
            cwd=ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return None
    return [line.strip() for line in out.splitlines() if line.strip()]


def classify_files(
    files: list[str],
    pkg_dirs: dict[str, Path],
    rev_deps: dict[str, set[str]],
) -> dict[str, object]:
    # Sort member directories longest-first so nested members match cleanly.
    dir_to_pkg = sorted(
        ((p.as_posix().rstrip("/") + "/", name) for name, p in pkg_dirs.items()),
        key=lambda item: len(item[0]),
        reverse=True,
    )

    seeds: set[str] = set()
    # Packages to check/test without expanding their reverse dependents (e.g.
    # locale JSON edits only change runtime translation tables checked by
    # `heronite-i18n`'s own coverage tests, never the crate's Rust API).
    direct_only: set[str] = set()
    rust_src_touched = False

    for raw_path in files:
        path = raw_path.replace("\\", "/").removeprefix("./")
        if not path:
            continue

        if path in GLOBAL_FILES or any(path.startswith(p) for p in GLOBAL_PREFIXES):
            return {
                "scope": "workspace",
                "run_rust": True,
                "clippy_args": "--workspace",
                "test_args": "--workspace",
                "packages": sorted(pkg_dirs),
                "reason": f"global file changed: {path}",
            }

        if path.startswith("crates/i18n/locales/"):
            direct_only.add("heronite-i18n")
            continue

        matched_pkg: str | None = None
        for prefix, pkg in dir_to_pkg:
            if path.startswith(prefix):
                matched_pkg = pkg
                break
        if matched_pkg is not None:
            seeds.add(matched_pkg)
            if path.endswith(".rs") and (path.startswith("app/src/") or "/src/" in path):
                rust_src_touched = True
            continue

        extra_matched = False
        for prefix, pkgs in EXTRA_PATH_PACKAGES:
            if path == prefix or path.startswith(prefix):
                seeds.update(pkgs)
                extra_matched = True
                break
        if extra_matched:
            continue

        if (
            fnmatch.fnmatch(path, "*.md")
            or path in NON_RUST_FILES
            or any(path.startswith(p) for p in NON_RUST_PREFIXES)
        ):
            continue

        # Unknown top-level path: fail safe to full workspace.
        return {
            "scope": "workspace",
            "run_rust": True,
            "clippy_args": "--workspace",
            "test_args": "--workspace",
            "packages": sorted(pkg_dirs),
            "reason": f"unmapped path changed: {path}",
        }

    if not seeds and not direct_only:
        return {
            "scope": "none",
            "run_rust": False,
            "clippy_args": "",
            "test_args": "",
            "packages": [],
            "reason": "no Rust crates affected",
        }

    affected = transitive_reverse_deps(seeds, rev_deps) | direct_only
    if len(affected) >= len(pkg_dirs):
        return {
            "scope": "workspace",
            "run_rust": True,
            "clippy_args": "--workspace",
            "test_args": "--workspace",
            "packages": sorted(pkg_dirs),
            "reason": f"all {len(pkg_dirs)} crates affected",
        }

    clippy_pkgs = sorted(affected)
    test_pkgs = set(affected)
    if rust_src_touched and "heronite-i18n" in pkg_dirs:
        test_pkgs.add("heronite-i18n")
    test_pkgs_sorted = sorted(test_pkgs)

    total_seeds = len(seeds | direct_only)
    return {
        "scope": "packages",
        "run_rust": True,
        "clippy_args": " ".join(f"-p {p}" for p in clippy_pkgs),
        "test_args": " ".join(f"-p {p}" for p in test_pkgs_sorted),
        "packages": test_pkgs_sorted,
        "reason": f"{total_seeds} changed -> {len(clippy_pkgs)} affected crate(s)",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "pull_request"))
    parser.add_argument("--base", default="HEAD^1")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--files", nargs="*", help="Explicit list of changed files (for testing)")
    parser.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT", ""))
    args = parser.parse_args()

    pkg_dirs, rev_deps = load_workspace()

    if args.files is not None:
        result = classify_files(args.files, pkg_dirs, rev_deps)
    elif args.event != "pull_request":
        result = {
            "scope": "workspace",
            "run_rust": True,
            "clippy_args": "--workspace",
            "test_args": "--workspace",
            "packages": sorted(pkg_dirs),
            "reason": f"full run for event={args.event}",
        }
    else:
        changed = git_changed_files(args.base, args.head)
        if changed is None:
            result = {
                "scope": "workspace",
                "run_rust": True,
                "clippy_args": "--workspace",
                "test_args": "--workspace",
                "packages": sorted(pkg_dirs),
                "reason": "could not compute git diff; falling back to --workspace",
            }
        else:
            result = classify_files(changed, pkg_dirs, rev_deps)

    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as fh:
            fh.write(f"scope={result['scope']}\n")
            fh.write(f"run_rust={'true' if result['run_rust'] else 'false'}\n")
            fh.write(f"clippy_args={result['clippy_args']}\n")
            fh.write(f"test_args={result['test_args']}\n")
            fh.write(f"reason={result['reason']}\n")

    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
