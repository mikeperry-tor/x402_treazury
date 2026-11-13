"""Exercise monitor ordering and isolation with small, socket-free processes."""

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "diagnose-treasury-sync.py"
SPEC = importlib.util.spec_from_file_location("treasury_diagnostics", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class TreasuryDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "output"
        self.executable = self.root / "test-fixture"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.tool(self.bin / "gdb", "import sys\nprint('synthetic stack')\nsys.exit(0)\n")
        # Use the actual process-control code on both development hosts. Native
        # /proc and GDB acceptance remains a separate Linux check.
        self.runner = self.root / "run.py"
        self.runner.write_text(
            "import importlib.util, os, pathlib, sys\n"
            f"spec = importlib.util.spec_from_file_location('monitor', {str(SCRIPT)!r})\n"
            "module = importlib.util.module_from_spec(spec)\nspec.loader.exec_module(module)\n"
            "module.group_members = lambda group: {group: {'state': 'S'}}\n"
            "if os.environ.get('CAPTURE_FAILS') == '1':\n"
            " def fail(*args): raise OSError('synthetic capture failure')\n"
            " module.capture = fail\n"
            "sys.exit(module.monitor(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]),\n"
            " after=float(os.environ.get('CAPTURE_AFTER', '0.25')),\n"
            " capture_budget=float(os.environ.get('CAPTURE_BUDGET', '0.3')), grace=0.1,\n"
            " lifecycle=os.environ.get('LIFECYCLE') == '1'))\n"
        )

    def tool(self, path, body):
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)

    def start(self, body, **env):
        self.tool(self.executable, body)
        child = subprocess.Popen(
            [sys.executable, str(self.runner), str(self.executable), str(self.output)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env={**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"], **env},
        )
        self.addCleanup(self.cleanup, child)
        return child

    @staticmethod
    def cleanup(child):
        if child.poll() is None:
            child.kill()
        child.communicate(timeout=5)

    def result(self, child, expected):
        output, _ = child.communicate(timeout=8)
        self.assertEqual(child.returncode, expected, output.decode())
        status = json.loads((self.output / "status.json").read_text())
        self.assertEqual(status["exit_status"], expected)
        self.assertEqual(status["test_threads"], 8)
        return status

    def wait_for(self, path):
        deadline = time.monotonic() + 3
        while not path.exists():
            if time.monotonic() >= deadline:
                self.fail(f"Fixture did not create {path.name}")
            time.sleep(0.01)

    def assert_not_running(self, pid):
        # A killed descendant may remain a zombie until its parent/init reaps it.
        state = subprocess.run(
            ["ps", "-p", str(pid), "-o", "stat="], capture_output=True, text=True, check=False
        ).stdout.strip()
        self.assertTrue(not state or state.startswith("Z"), f"PID {pid} still running: {state}")

    def test_success_and_failure_preserve_output_and_status(self):
        for code in (0, 17):
            with self.subTest(code=code):
                child = self.start(
                    "import os, sys\n"
                    "assert sys.argv[1:] == ['--test-threads=8', '--nocapture']\n"
                    "assert os.environ['X402_TEST_SYNC_DIAGNOSTICS'] == '1'\n"
                    f"print('fixture output', flush=True)\nsys.exit({code})\n",
                    CAPTURE_AFTER="3",
                )
                status = self.result(child, code)
                self.assertEqual(status["reason"], "completed")
                self.assertIn("fixture output", (self.output / "test.log").read_text())
                self.assertFalse((self.output / "processes.json").exists())

    def test_timeout_captures_before_terminating_group_and_spares_other_processes(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        self.addCleanup(self.cleanup, unrelated)
        marker = self.root / "child.pid"
        child = self.start(
            "import pathlib, subprocess, sys, time\n"
            "descendant = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
            f"pathlib.Path({str(marker)!r}).write_text(str(descendant.pid))\n"
            "print('entered stall', flush=True)\ntime.sleep(30)\n"
        )
        self.wait_for(marker)
        status = self.result(child, 124)
        self.assertEqual(status["reason"], "diagnostic_limit")
        self.assertIn(str(status["test_pid"]), (self.output / "processes.json").read_text())
        self.assertIn("synthetic stack", (self.output / f"threads-{status['test_pid']}.txt").read_text())
        self.assert_not_running(int(marker.read_text()))
        self.assertIsNone(unrelated.poll())

    def test_cancellation_captures_before_termination(self):
        ready = self.root / "ready"
        self.tool(
            self.bin / "gdb",
            "import pathlib, sys, os, signal\n"
            "pid = int(sys.argv[-1])\nos.kill(pid, 0)\n"
            f"pathlib.Path({str(self.root / 'captured-alive')!r}).write_text(str(pid))\n"
            f"pathlib.Path({str(self.root / 'debugger-args')!r}).write_text('\\n'.join(sys.argv[1:]))\n",
        )
        child = self.start(
            f"import pathlib, time\npathlib.Path({str(ready)!r}).touch()\ntime.sleep(30)\n",
            CAPTURE_AFTER="30",
        )
        self.wait_for(ready)
        child.send_signal(signal.SIGINT)
        status = self.result(child, 130)
        self.assertEqual(status["reason"], "cancelled")
        self.assertTrue((self.root / "captured-alive").exists())
        args = (self.root / "debugger-args").read_text().splitlines()
        self.assertIn("set print frame-arguments none", args)
        self.assertIn("set auto-load off", args)
        self.assertIn("set debuginfod enabled off", args)
        self.assertIn("thread apply all bt", args)
        self.assertNotIn("thread apply all bt full", args)
        self.assert_not_running(status["test_pid"])

    def test_failed_and_stalled_debuggers_do_not_prevent_cleanup(self):
        for debugger in ("import sys; sys.exit(9)", "import time; time.sleep(30)"):
            with self.subTest(debugger=debugger):
                self.tool(self.bin / "gdb", debugger + "\n")
                child = self.start("import time; time.sleep(30)\n")
                status = self.result(child, 124)
                self.assert_not_running(status["test_pid"])
                text = (self.output / f"threads-{status['test_pid']}.txt").read_text()
                self.assertTrue("status: 9" in text or "budget exhausted" in text, text)

    def test_cancel_during_capture_kills_debugger_and_its_descendants(self):
        ready = self.root / "debugger-pids"
        self.tool(
            self.bin / "gdb",
            "import os, pathlib, subprocess, sys, time\n"
            "descendant = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
            f"pathlib.Path({str(ready)!r}).write_text(f'{{os.getpid()}} {{descendant.pid}}')\n"
            "time.sleep(30)\n",
        )
        child = self.start("import time; time.sleep(30)\n", CAPTURE_BUDGET="30")
        self.wait_for(ready)
        child.send_signal(signal.SIGINT)
        status = self.result(child, 130)
        self.assertEqual(status["reason"], "cancelled")
        self.assert_not_running(status["test_pid"])
        for pid in ready.read_text().split():
            self.assert_not_running(int(pid))

    def test_cargo_selection_rejects_missing_and_ambiguous_executables(self):
        build = self.root / "build.jsonl"
        artifact = {
            "reason": "compiler-artifact",
            "target": {"name": "treasury_sync", "kind": ["test"]},
            "profile": {"test": True},
            "executable": str(self.executable),
        }
        self.tool(self.executable, "pass\n")
        build.write_text("Make recipe\n" + json.dumps(artifact) + "\n")
        self.assertEqual(MODULE.test_executable(build), self.executable)
        build.write_text("Make recipe\n")
        with self.assertRaisesRegex(ValueError, "found 0"):
            MODULE.test_executable(build)
        build.write_text(json.dumps(artifact) + "\n" + json.dumps({**artifact, "executable": "/other"}))
        with self.assertRaisesRegex(ValueError, "found 2"):
            MODULE.test_executable(build)
        build.write_text(json.dumps({**artifact, "executable": str(self.root / "missing")}))
        with self.assertRaisesRegex(ValueError, "missing"):
            MODULE.test_executable(build)

    def test_proc_snapshot_selects_owned_group_and_tolerates_exited_threads(self):
        proc = self.root / "proc"
        for pid, group in ((10, 10), (11, 10), (12, 12)):
            entry = proc / str(pid)
            entry.mkdir(parents=True)
            fields = ["S", "1", str(group)] + ["0"] * 19
            fields[11], fields[12], fields[21] = "250", "50", "3"
            (entry / "stat").write_text(f"{pid} (fixture (worker)) {' '.join(fields)}\n")
            task = entry / "task" / str(pid)
            task.mkdir(parents=True)
            (task / "wchan").write_text("futex_wait\n")
        (proc / "10/task/exited").mkdir()
        (proc / "999").mkdir()  # Process disappeared before stat could be read.
        with mock.patch.object(MODULE, "Path", return_value=proc):
            members = MODULE.group_members(10)
        self.assertEqual(set(members), {10, 11})
        self.assertEqual(members[10]["name"], "fixture (worker)")
        self.assertEqual(members[10]["threads"], {"10": "futex_wait"})
        self.assertEqual(members[10]["cpu_user_seconds"], 250 / os.sysconf("SC_CLK_TCK"))
        self.assertEqual(members[10]["cpu_system_seconds"], 50 / os.sysconf("SC_CLK_TCK"))
        self.assertEqual(members[10]["resident_bytes"], 3 * os.sysconf("SC_PAGE_SIZE"))

    def test_lifecycle_selects_library_artifact_instead_of_integration(self):
        library = self.root / "library-fixture"
        self.tool(library, "pass\n")
        self.tool(self.executable, "pass\n")
        build = self.root / "build.jsonl"
        artifact = {
            "reason": "compiler-artifact", "profile": {"test": True},
            "target": {"name": "treasury_sync", "kind": ["test"]},
            "executable": str(self.executable),
        }
        build.write_text(json.dumps(artifact) + "\n" + json.dumps({
            **artifact, "target": {"name": "x402_treazury", "kind": ["lib"]},
            "executable": str(library),
        }))
        self.assertEqual(MODULE.test_executable(build), self.executable)
        self.assertEqual(MODULE.test_executable(build, lifecycle=True), library)

    def test_periodic_lifecycle_capture_preserves_completion_during_failed_debugger(self):
        permit = self.root / "finish-during-capture"
        self.tool(
            self.bin / "gdb",
            f"import pathlib, subprocess, sys, time\npathlib.Path({str(permit)!r}).touch()\n"
            "while True:\n"
            " state = subprocess.run(['ps', '-p', sys.argv[-1], '-o', 'stat='], capture_output=True, text=True).stdout.strip()\n"
            " if not state or state.startswith('Z'): break\n"
            " time.sleep(0.01)\n"
            "sys.exit(9)\n",
        )
        child = self.start(
            "import pathlib, sys, time\n"
            "assert sys.argv[1:] == ['treasury::lifecycle_tests', '--test-threads=8', '--nocapture']\n"
            f"while not pathlib.Path({str(permit)!r}).exists(): time.sleep(0.01)\n"
            "sys.exit(17)\n",
            LIFECYCLE="1", CAPTURE_AFTER="0.1", CAPTURE_BUDGET="2",
        )
        status = self.result(child, 17)
        self.assertEqual(status["reason"], "completed")
        self.assertEqual(status["periodic_snapshots"], 1)
        self.assertTrue((self.output / "snapshot-1/processes.json").exists())

    def test_lifecycle_continues_after_multiple_capture_errors(self):
        finished = self.root / "finished"
        child = self.start(
            f"import pathlib, time\ntime.sleep(0.7)\npathlib.Path({str(finished)!r}).touch()\n",
            LIFECYCLE="1", CAPTURE_AFTER="0.1", CAPTURE_FAILS="1",
        )
        status = self.result(child, 0)
        self.assertEqual(status["reason"], "completed")
        self.assertGreaterEqual(status["periodic_snapshots"], 2)
        self.assertTrue(finished.exists())
        self.assertTrue((self.output / "snapshot-2/capture-error.txt").exists())

    def test_cancel_during_periodic_capture_does_not_attach_twice(self):
        ready = self.root / "capture-started"
        self.tool(
            self.bin / "gdb",
            "import pathlib, time\n"
            f"with pathlib.Path({str(ready)!r}).open('a') as marker: marker.write('attach\\n')\n"
            "time.sleep(30)\n",
        )
        child = self.start(
            "import time; time.sleep(30)\n", LIFECYCLE="1", CAPTURE_BUDGET="30",
        )
        self.wait_for(ready)
        child.send_signal(signal.SIGINT)
        status = self.result(child, 130)
        self.assertEqual(status["reason"], "cancelled")
        self.assertEqual(status["periodic_snapshots"], 1)
        self.assertEqual(ready.read_text().splitlines(), ["attach"])
        self.assertFalse((self.output / "processes.json").exists())
        self.assert_not_running(status["test_pid"])


if __name__ == "__main__":
    unittest.main()
