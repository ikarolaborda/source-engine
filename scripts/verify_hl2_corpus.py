#!/usr/bin/env python3
"""Discover and fingerprint locally owned Half-Life 2 compatibility assets."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys


CRITICAL_FILES = (
    "hl2/gameinfo.txt",
    "hl2/steam.inf",
    "hl2/hl2_misc_dir.vpk",
    "hl2/hl2_pak_dir.vpk",
    "hl2/hl2_sound_misc_dir.vpk",
    "hl2/hl2_sound_vo_english_dir.vpk",
    "hl2/hl2_textures_dir.vpk",
    "hl2/maps/d1_trainstation_01.bsp",
    "hl2/resource/closecaption_english.dat",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def steam_libraries(home: Path) -> list[Path]:
    steam = home / "Library/Application Support/Steam"
    libraries = [steam]
    folders = steam / "steamapps/libraryfolders.vdf"
    if folders.is_file():
        text = folders.read_text(encoding="utf-8", errors="replace")
        for value in re.findall(r'"path"\s+"([^"]+)"', text):
            libraries.append(Path(value.replace("\\\\", "\\")))
    return libraries


def candidates(explicit: list[Path]) -> list[Path]:
    home = Path.home()
    found = list(explicit)
    configured = os.environ.get("SOURCE_HL2_ROOT")
    if configured:
        found.append(Path(configured))
    for library in steam_libraries(home):
        found.append(library / "steamapps/common/Half-Life 2")
    found.append(home / "Documents/Gaming/Half Life 2")

    unique: list[Path] = []
    seen: set[Path] = set()
    for path in found:
        path = path.expanduser().resolve()
        if path not in seen and (path / "hl2/gameinfo.txt").is_file():
            seen.add(path)
            unique.append(path)
    return unique


def app_manifest(root: Path) -> dict[str, str]:
    manifest = root.parent.parent / "appmanifest_220.acf"
    if not manifest.is_file():
        return {}
    text = manifest.read_text(encoding="utf-8", errors="replace")
    result = {"path": str(manifest)}
    for key in ("buildid", "LastUpdated", "BetaKey"):
        match = re.search(rf'"{re.escape(key)}"\s+"([^"]+)"', text)
        if match:
            result[key] = match.group(1)
    return result


def inspect(root: Path, full: bool) -> dict[str, object]:
    missing = [relative for relative in CRITICAL_FILES if not (root / relative).is_file()]
    files = [root / relative for relative in CRITICAL_FILES if (root / relative).is_file()]
    if full:
        files.extend(root.glob("**/*_dir.vpk"))
        files.extend(root.glob("**/*.bsp"))
        files = sorted(set(files))

    fingerprints = {}
    for path in files:
        relative = path.relative_to(root).as_posix()
        fingerprints[relative] = {
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
    return {
        "root": str(root),
        "steam_manifest": app_manifest(root),
        "bsp_count": sum(1 for _ in root.glob("**/*.bsp")),
        "vpk_count": sum(1 for _ in root.glob("**/*.vpk")),
        "missing_critical": missing,
        "files": fingerprints,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", action="append", default=[], type=Path)
    parser.add_argument("--full", action="store_true", help="hash all BSP and VPK directory files")
    args = parser.parse_args()

    roots = candidates(args.root)
    report = {
        "schema": 1,
        "corpora": [inspect(root, args.full) for root in roots],
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    if not roots:
        print("No Half-Life 2 corpus found", file=sys.stderr)
        return 1
    if all(item["missing_critical"] for item in report["corpora"]):
        print("No corpus contains every critical HL2 file", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

