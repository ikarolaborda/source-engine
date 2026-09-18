#!/usr/bin/env python3
"""Start a live gate in its own session and return immediately.

The live gates need the game's window, audio device and content, so they only
run on a developer machine, and the long ones run for an hour.  Waiting on one
holds up whatever else is being worked on, and a gate started with `&` from a
tool-driven shell dies when that shell's call ends, because it is still in the
caller's process group.  Double-forking through setsid puts the run in a
session of its own, so it survives, writes to a log, and can be polled later.

    scripts/gate_detached.py start soak --duration 3700
    scripts/gate_detached.py status soak
"""

from __future__ import annotations

import argparse
import errno
import os
import signal
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUNS = os.path.join(ROOT, "out-gate-runs")
DEFAULT_RUNTIME = os.path.join(ROOT, "out-rust-ci")
DEFAULT_CONTENT = os.path.expanduser(
    "~/Library/Application Support/Steam/steamapps/common/Half-Life 2"
)
# Long enough for the gate to reach its own timeout and report, rather than
# being cut off by this wrapper and blamed for a failure it did not have.
DURATIONS = {
    "smoke": 180,
    "save-demo": 180,
    "physics": 300,
    "hud-audio": 180,
    "intro-lipsync": 150,
    "ten-minute": 660,
    "soak": 3700,
}


def paths(scenario: str) -> tuple[str, str, str]:
    return (
        os.path.join(RUNS, f"{scenario}.log"),
        os.path.join(RUNS, f"{scenario}.pid"),
        os.path.join(RUNS, f"{scenario}.exit"),
    )


def running(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except OSError as error:
        return error.errno == errno.EPERM
    return True


def read_pid(pid_path: str) -> int | None:
    try:
        with open(pid_path) as handle:
            return int(handle.read().strip())
    except (OSError, ValueError):
        return None


def start(args: argparse.Namespace) -> int:
    log_path, pid_path, exit_path = paths(args.scenario)

    previous = read_pid(pid_path)
    if previous is not None and running(previous):
        print(f"{args.scenario} is already running as pid {previous}")
        return 1

    os.makedirs(RUNS, exist_ok=True)
    for stale in (log_path, exit_path):
        if os.path.exists(stale):
            os.replace(stale, stale + ".previous")

    duration = args.duration or DURATIONS[args.scenario]
    command = [
        os.path.join(ROOT, "scripts", "run_rust_hl2_smoke.sh"),
        args.runtime,
        args.content,
        str(duration),
        args.scenario,
    ]

    # The child reports its own exit through a file, because nothing is left
    # waiting on it once this process returns.
    script = (
        'set -o pipefail; "$@" > "$GATE_LOG" 2>&1; '
        'printf %s "$?" > "$GATE_EXIT"'
    )
    environment = dict(os.environ, GATE_LOG=log_path, GATE_EXIT=exit_path)

    child = subprocess.Popen(
        ["bash", "-c", script, "gate"] + command,
        cwd=ROOT,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )

    with open(pid_path, "w") as handle:
        handle.write(str(child.pid))

    print(f"started {args.scenario} as pid {child.pid} for up to {duration}s")
    print(f"log: {log_path}")
    return 0


def status(args: argparse.Namespace) -> int:
    log_path, pid_path, exit_path = paths(args.scenario)
    pid = read_pid(pid_path)

    if pid is not None and running(pid):
        age = int(time.time() - os.path.getmtime(pid_path))
        print(f"{args.scenario}: running as pid {pid} for {age}s")
        return 0

    if not os.path.exists(exit_path):
        print(f"{args.scenario}: not running, and no result recorded")
        return 1

    with open(exit_path) as handle:
        code = handle.read().strip()

    tail = ""
    if os.path.exists(log_path):
        with open(log_path, errors="replace") as handle:
            tail = "".join(handle.readlines()[-args.lines :])

    print(f"{args.scenario}: finished with exit {code}")
    if tail:
        print(tail, end="" if tail.endswith("\n") else "\n")
    return 0 if code == "0" else 1


def stop(args: argparse.Namespace) -> int:
    _, pid_path, _ = paths(args.scenario)
    pid = read_pid(pid_path)
    if pid is None or not running(pid):
        print(f"{args.scenario}: not running")
        return 1
    # The gate's own children hold the window and the audio device, so the
    # whole session goes, not just the shell that started it.
    os.killpg(os.getpgid(pid), signal.SIGTERM)
    print(f"{args.scenario}: signalled pid {pid}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    for name, handler in (("start", start), ("status", status), ("stop", stop)):
        command = commands.add_parser(name)
        command.add_argument("scenario", choices=sorted(DURATIONS))
        command.set_defaults(handler=handler)
        if name == "start":
            command.add_argument("--duration", type=int)
            command.add_argument("--runtime", default=DEFAULT_RUNTIME)
            command.add_argument("--content", default=DEFAULT_CONTENT)
        if name == "status":
            command.add_argument("--lines", type=int, default=5)

    args = parser.parse_args()
    return args.handler(args)


if __name__ == "__main__":
    sys.exit(main())
