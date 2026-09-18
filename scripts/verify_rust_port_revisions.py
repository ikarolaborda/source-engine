#!/usr/bin/env python3
"""Verify the frozen source and submodule revisions for the Rust-port oracle."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "docs" / "rust-port" / "revisions.toml"
ASSIGNMENT = re.compile(r'^([A-Za-z0-9_-]+)\s*=\s*"([0-9a-f]{40})"\s*$')


def read_revisions(path: Path) -> tuple[str, dict[str, str]]:
    source_revision: str | None = None
    submodules: dict[str, str] = {}
    section = ""
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        match = ASSIGNMENT.match(line)
        if not match:
            continue
        name, revision = match.groups()
        if not section and name == "source_engine":
            source_revision = revision
        elif section == "submodules":
            submodules[name] = revision
    if source_revision is None:
        raise ValueError(f"missing source_engine revision in {path}")
    if not submodules:
        raise ValueError(f"missing [submodules] revisions in {path}")
    return source_revision, submodules


def git_revision(directory: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(directory), "rev-parse", "HEAD"], text=True
    ).strip()


def is_dirty(directory: Path) -> bool:
    return bool(
        subprocess.check_output(
            ["git", "-C", str(directory), "status", "--porcelain"], text=True
        ).strip()
    )


def contains_revision(directory: Path, revision: str) -> bool:
    """Report whether `revision` is reachable from the checked-out HEAD."""

    if (
        subprocess.call(
            ["git", "-C", str(directory), "cat-file", "-e", f"{revision}^{{commit}}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        != 0
    ):
        return False
    return (
        subprocess.call(
            ["git", "-C", str(directory), "merge-base", "--is-ancestor", revision, "HEAD"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        == 0
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument(
        "--require-clean",
        action="store_true",
        help="also reject tracked or untracked working-tree changes",
    )
    args = parser.parse_args()

    expected_root, expected_submodules = read_revisions(args.manifest)
    mismatches: list[str] = []

    # The port advances past the captured oracle, so the frozen source
    # revision only has to remain in the checked-out history.  Submodules stay
    # pinned exactly because the compatibility corpus was measured against
    # those exact trees.
    actual_root = git_revision(ROOT)
    if not contains_revision(ROOT, expected_root):
        mismatches.append(
            f"source_engine: frozen {expected_root} is not an ancestor of {actual_root}"
        )
    if args.require_clean and is_dirty(ROOT):
        mismatches.append("source_engine: working tree is dirty")

    for relative, expected in sorted(expected_submodules.items()):
        directory = ROOT / relative
        if not directory.is_dir():
            mismatches.append(f"{relative}: submodule directory is missing")
            continue
        actual = git_revision(directory)
        if actual != expected:
            mismatches.append(f"{relative}: expected {expected}, found {actual}")
        if args.require_clean and is_dirty(directory):
            mismatches.append(f"{relative}: working tree is dirty")

    if mismatches:
        for mismatch in mismatches:
            print(f"revision mismatch: {mismatch}", file=sys.stderr)
        return 1

    print(
        f"verified frozen source revision {expected_root} in the history of "
        f"{actual_root} and {len(expected_submodules)} frozen submodules"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
