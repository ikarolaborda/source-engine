#!/usr/bin/env python3
"""Read-only ARM64 install audit; not a substitute for per-game runtime tests."""

import argparse
import hashlib
from pathlib import Path
import subprocess


GAMES = ("hl2", "hl2mp", "hl1", "hl1mp", "episodic", "portal", "cstrike", "dod")
# Engine modules that are Rust alone. Waf never removes an installed file, so a
# C++ build of the same name left behind by an earlier install would load
# without complaint; each is checked for what it is, not only that it exists.
RUST_MODULES = ("scenefilecache", "soundemittersystem")
# A module may export one more symbol than its factory when the platform ABI
# forces a hand-written thunk that the linker will not hide.
RUST_MODULE_EXTRA_EXPORTS = {"soundemittersystem": {"_source_add_wave_name"}}
# Rust modules that are flat C rather than an interface, so there is no factory
# to look for. Eight subprojects link `steam_api`, which means its whole export
# set is the contract, and the set is small enough to hold here in full: a
# missing name is a link that will fail, a new one is a claim the C++ never made.
RUST_FLAT_MODULES = {
    "steam_api": frozenset({
        '_GetHSteamPipe', '_GetHSteamUser', '_SteamAPI_GetHSteamPipe',
        '_SteamAPI_GetHSteamUser', '_SteamAPI_GetSteamInstallPath', '_SteamAPI_Init',
        '_SteamAPI_InitSafe', '_SteamAPI_IsSteamRunning', '_SteamAPI_RegisterCallResult',
        '_SteamAPI_RegisterCallback', '_SteamAPI_ReleaseCurrentThreadMemory',
        '_SteamAPI_RestartAppIfNecessary', '_SteamAPI_RunCallbacks',
        '_SteamAPI_SetBreakpadAppID', '_SteamAPI_SetMiniDumpComment',
        '_SteamAPI_SetTryCatchCallbacks', '_SteamAPI_Shutdown',
        '_SteamAPI_UnregisterCallResult', '_SteamAPI_UnregisterCallback',
        '_SteamAPI_UseBreakpadCrashHandler', '_SteamAPI_WriteMiniDump', '_SteamApps',
        '_SteamClient', '_SteamFriends', '_SteamGameServer_GetHSteamPipe',
        '_SteamGameServer_GetHSteamUser', '_SteamGameServer_GetIPCCallCount',
        '_SteamGameServer_InitSafe', '_SteamGameServer_RunCallbacks',
        '_SteamGameServer_Shutdown', '_SteamHTTP', '_SteamInternal_ContextInit',
        '_SteamInternal_CreateInterface', '_SteamMatchmaking', '_SteamMatchmakingServers',
        '_SteamNetworking', '_SteamRemoteStorage', '_SteamScreenshots', '_SteamUser',
        '_SteamUserStats', '_SteamUtils', '_Steam_GetHSteamUserCurrent',
        '_Steam_RegisterInterfaceFuncs', '_Steam_RunCallbacks', '_g_pSteamClientGameServer',
    }),
}
CPP_RUNTIME = ("libc++", "libtier0", "libvstdlib")


def output(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def symbols(path, defined):
    return {line.split()[-1] for line in output("nm", "-gU" if defined else "-u", str(path)).splitlines() if line.strip()}


def audit(runtime, games, cargo_launcher=None):
    launcher = runtime / "hl2_launcher"
    shared = sorted((runtime / "bin").glob("*.dylib"))
    modules = [runtime / game / "bin" / f"lib{kind}.dylib" for game in games for kind in ("client", "server")]
    required = [runtime / "bin" / f"lib{name}.dylib" for name in ("source_abi", "rust_engine_bridge", "launcher", "engine", "filesystem_stdio", "shaderapimetal", *RUST_MODULES, *RUST_FLAT_MODULES)]
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

    expected = {name: {"_CreateInterface"} | RUST_MODULE_EXTRA_EXPORTS.get(name, set()) for name in RUST_MODULES}
    expected.update(RUST_FLAT_MODULES)
    for name, allowed in expected.items():
        path = runtime / "bin" / f"lib{name}.dylib"
        if exports[path] != set(allowed):
            raise ValueError(f"not the Rust {name} module, which exports {len(allowed)} symbols: {path}")
        # For the interface modules the export set alone tells the two apart,
        # since the C++ ones export more. A flat module's C++ original exports
        # exactly the same names, so what distinguishes them is that it was
        # compiled as C++ and links the C++ runtime, and this one does not.
        linked = [line.split()[0] for line in output("otool", "-L", str(path)).splitlines()[1:]]
        native = [library for library in linked if any(marker in library for marker in CPP_RUNTIME)]
        if native:
            raise ValueError(f"Rust {name} module links C++ libraries {native}: {path}")

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
