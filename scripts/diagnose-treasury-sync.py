#!/usr/bin/env python3
"""Bounded, opt-in Linux experiment; never used by ordinary application checks."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time


def test_executable(build_log):
    candidates = set()
    for line in build_log.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue  # Make may also print its recipe.
        if (
            item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == "treasury_sync"
            and item.get("target", {}).get("kind") == ["test"]
            and item.get("profile", {}).get("test") is True
            and item.get("executable")
        ):
            candidates.add(item["executable"])
    if len(candidates) != 1:
        raise ValueError(f"Expected one treasury_sync test executable; found {len(candidates)}")
    executable = Path(candidates.pop())
    if not executable.is_file():
        raise ValueError("The compiled treasury_sync executable is missing")
    return executable


def group_members(group):
    """Inspect only the experiment's process group, without command lines or env."""
    members = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            raw = (entry / "stat").read_text()
            name, _, rest = raw.partition("(")[2].rpartition(")")
            fields = rest.split()
            if int(fields[2]) != group:
                continue
            threads = {}
            for task in (entry / "task").iterdir():
                try:
                    threads[task.name] = (task / "wchan").read_text().strip()
                except OSError:
                    continue
            members[int(entry.name)] = {
                "name": name,
                "state": fields[0],
                "parent": int(fields[1]),
                "group": group,
                "threads": threads,
            }
        except (OSError, ValueError, IndexError):
            continue  # An owned process/thread can exit while being inspected.
    return members


def kill_group(group, sig):
    try:
        os.killpg(group, sig)
    except ProcessLookupError:
        pass


def capture(group, output, budget, interrupted):
    deadline = time.monotonic() + budget
    members = group_members(group)
    (output / "processes.json").write_text(json.dumps(members, indent=2) + "\n")
    # Root first: it owns the async runtime even when no child CLI is running.
    shortened = False
    for pid in sorted(members, key=lambda pid: (pid != group, pid)):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        with (output / f"threads-{pid}.txt").open("wb") as log:
            try:
                debugger = subprocess.Popen(
                    [
                        "sudo", "-n", "gdb", "-q", "-nx", "-batch",
                        "-iex", "set auto-load off",
                        "-iex", "set debuginfod enabled off",
                        "-ex", "set pagination off",
                        "-ex", "set print frame-arguments none",
                        "-ex", "thread apply all bt",
                        "-ex", "detach", "-p", str(pid),
                    ],
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
            except OSError as error:
                log.write(f"Debugger unavailable: {error}\n".encode())
                continue
            while True:
                if interrupted and not shortened:
                    deadline = min(deadline, time.monotonic() + 4)
                    shortened = True
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    kill_group(debugger.pid, signal.SIGKILL)
                    debugger.wait()
                    log.write(b"\nDebugger capture budget exhausted.\n")
                    break
                try:
                    status = debugger.wait(timeout=min(remaining, 0.05))
                    log.write(f"\nDebugger exit status: {status}\n".encode())
                    break
                except subprocess.TimeoutExpired:
                    continue


def monitor(executable, output, after=300, capture_budget=30, grace=1):
    output.mkdir(parents=True, exist_ok=True)
    interrupted = []
    previous = {}

    def stop(signum, _frame):
        if not interrupted:
            interrupted.append(signum)

    for sig in (signal.SIGINT, signal.SIGTERM):
        previous[sig] = signal.signal(sig, stop)
    process = None
    started = time.monotonic()
    try:
        with (output / "test.log").open("wb") as log:
            process = subprocess.Popen(
                [str(executable), "--test-threads=8", "--nocapture"],
                env={**os.environ, "X402_TEST_SYNC_DIAGNOSTICS": "1"},
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )

            def copy_output():
                for line in iter(process.stdout.readline, b""):
                    log.write(line)
                    log.flush()
                    try:
                        sys.stdout.buffer.write(line)
                        sys.stdout.buffer.flush()
                    except BrokenPipeError:
                        pass  # The artifact log remains useful if console output closes.

            reader = threading.Thread(target=copy_output, daemon=True)
            reader.start()
            reason = "completed"
            while process.poll() is None:
                if interrupted:
                    reason = "cancelled"
                    break
                if time.monotonic() - started >= after:
                    reason = "diagnostic_limit"
                    break
                time.sleep(0.05)
            if reason != "completed":
                print(f"Capturing treasury fixture diagnostics: {reason}", flush=True)
                # GitHub normally gives SIGINT only 7.5 seconds before SIGTERM.
                budget = min(capture_budget, 4) if interrupted else capture_budget
                try:
                    capture(process.pid, output, budget, interrupted)
                except Exception as error:
                    (output / "capture-error.txt").write_text(f"{type(error).__name__}: {error}\n")
                if interrupted:
                    reason = "cancelled"
                status = 128 + interrupted[0] if interrupted else 124
            else:
                status = process.returncode
                if status < 0:
                    status = 128 - status
            (output / "status.json").write_text(json.dumps({
                "reason": reason,
                "exit_status": status,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "test_pid": process.pid,
                "test_threads": 8,
            }, indent=2) + "\n")
            # Also clean up descendants after a failed/panicking test exits.
            kill_group(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=grace)
            except subprocess.TimeoutExpired:
                pass
            kill_group(process.pid, signal.SIGKILL)
            process.wait()
            reader.join(timeout=grace)
            return status
    finally:
        if process is not None:
            kill_group(process.pid, signal.SIGKILL)
            process.wait()
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build_log", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("Native stack capture requires the focused Linux runner")
    return monitor(test_executable(args.build_log), args.output)


if __name__ == "__main__":
    sys.exit(main())
