#!/usr/bin/env python3
"""Read-only ARM64 install audit; not a substitute for per-game runtime tests."""

import argparse
import hashlib
from pathlib import Path
import subprocess


GAMES = ("hl2", "hl2mp", "hl1", "hl1mp", "episodic", "portal", "cstrike", "dod")


def output(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def symbols(path, defined):
    return {line.split()[-1] for line in output("nm", "-gU" if defined else "-u", str(path)).splitlines() if line.strip()}


def audit(runtime, games, cargo_launcher=None):
    launcher = runtime / "hl2_launcher"
    shared = sorted((runtime / "bin").glob("*.dylib"))
    modules = [runtime / game / "bin" / f"lib{kind}.dylib" for game in games for kind in ("client", "server")]
    required = [runtime / "bin" / f"lib{name}.dylib" for name in ("source_abi", "rust_engine_bridge", "launcher", "engine", "filesystem_stdio", "shaderapimetal")]
    for path in [launcher, *required, *modules]:
        if not path.is_file():
            raise ValueError(f"missing installed artifact: {path}")
    if cargo_launcher is not None and launcher.read_bytes() != cargo_launcher.read_bytes():
        raise ValueError("installed launcher differs from Cargo's source-launcher")

    exports = {}
    for path in [launcher, *shared, *modules]:
        if output("lipo", "-archs", str(path)).strip() != "arm64":
            raise ValueError(f"not a native arm64-only artifact: {path}")
        if path != launcher:
            exports[path] = symbols(path, True)
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        print(f"arm64\t{digest}\t{path.relative_to(runtime)}")
    for path in modules:
        if "_CreateInterface" not in exports[path]:
            raise ValueError(f"missing game interface factory: {path}")

    abi = exports[runtime / "bin" / "libsource_abi.dylib"]
    for name in ("source_host_run_app_system_group", "source_host_app_group_startup", "source_host_app_group_shutdown"):
        if "_" + name not in abi:
            raise ValueError(f"missing lifecycle ABI export: {name}")
    # Check transitional Rust imports against the shared install, not against
    # whichever build directory happens to be named by a Mach-O install ID.
    provided = set().union(*(exports[path] for path in shared))
    for path in [launcher, *shared, *modules]:
        missing = {name for name in symbols(path, False) if name.startswith("_source_")} - provided
        if missing:
            raise ValueError(f"unprovided Rust imports in {path}: {sorted(missing)}")
    print(f"PASS: {len(games)} client/server pairs, {len(shared)} shared libraries, Rust imports and lifecycle exports")
    print("Static package audit only: no content, renderer, gameplay or speech claim.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime", type=Path)
    parser.add_argument("--games", nargs="+", choices=GAMES, default=GAMES)
    parser.add_argument("--cargo-launcher", type=Path)
    args = parser.parse_args()
    try:
        audit(args.runtime.resolve(), args.games, args.cargo_launcher)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"FAIL: {error}\n")


if __name__ == "__main__":
    main()
