"""Exercise CI stage logging without Cargo, sockets, or network access."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "ci-stage.sh"
STAGES = (
    "check-static",
    "check-clippy",
    "check-compat",
    "check-default-remaining",
    "check-proving",
    "check-lifecycle",
    "check-codec",
)


class CiStageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.script = scripts / "ci-stage.sh"
        shutil.copyfile(SCRIPT, self.script)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.fake_tool(
            "make",
            '#!/bin/sh\nprintf "%s\\n" "$@" > "$CALLS"\n'
            'printf "stage stdout\\n"\nprintf "stage stderr\\n" >&2\n'
            'exit "${MAKE_STATUS:-0}"\n',
        )
        self.calls = self.root / "calls"
        self.summary = self.root / "summary"
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "CALLS": str(self.calls),
            "GITHUB_STEP_SUMMARY": str(self.summary),
            "MAKE_STATUS": "0",
        }

    def fake_tool(self, name, contents):
        tool = self.bin / name
        tool.write_text(contents)
        tool.chmod(0o755)

    def run_stage(self, *args, **env):
        return subprocess.run(
            ["bash", str(self.script), *args],
            cwd=self.bin,
            env={**self.env, **env},
            capture_output=True,
            text=True,
            check=False,
        )

    def assert_result(self, stage, status):
        fields = dict(
            line.split("=", 1)
            for line in (self.root / f"target/ci-checks/{stage}.status")
            .read_text()
            .splitlines()
        )
        self.assertGreaterEqual(int(fields["elapsed_seconds"]), 0)
        self.assertEqual(fields["exit_status"], str(status))
        summary = self.summary.read_text()
        self.assertIn(f"### {stage}\n", summary)
        self.assertIn(f"Elapsed: {fields['elapsed_seconds']} seconds", summary)
        self.assertIn(f"exit status: {status}.", summary)

    def test_each_stage_logs_combined_output_and_appends_summary(self):
        self.summary.write_text("Existing summary\n")
        for stage in STAGES:
            with self.subTest(stage=stage):
                result = self.run_stage(stage)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls.read_text(), stage + "\n")
                log = (self.root / f"target/ci-checks/{stage}.log").read_text()
                self.assertEqual(log, "stage stdout\nstage stderr\n")
                self.assertEqual(result.stdout, log)
                self.assert_result(stage, 0)
        summary = self.summary.read_text()
        self.assertTrue(summary.startswith("Existing summary\n"))
        self.assertEqual(summary.count("### check-"), len(STAGES))

    def test_make_failure_retains_output_status_and_summary(self):
        result = self.run_stage("check-clippy", MAKE_STATUS="23")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertIn("stage stderr", (self.root / "target/ci-checks/check-clippy.log").read_text())
        self.assert_result("check-clippy", 23)

    def test_log_failure_is_not_reported_as_success(self):
        self.fake_tool("tee", '#!/bin/sh\ncat > "$1"\nexit 17\n')
        result = self.run_stage("check-static")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assert_result("check-static", 17)

    def test_make_failure_takes_precedence_over_log_failure(self):
        self.fake_tool("tee", '#!/bin/sh\ncat > "$1"\nexit 17\n')
        result = self.run_stage("check-static", MAKE_STATUS="23")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assert_result("check-static", 23)

    def test_summary_is_optional(self):
        result = self.run_stage("check-codec", GITHUB_STEP_SUMMARY="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "target/ci-checks/check-codec.status").exists())
        self.assertFalse(self.summary.exists())

    def test_invalid_stage_or_argument_count_never_executes_make(self):
        for args in ((), ("check-default",), ("../check-static",), ("check-static", "check-clippy")):
            with self.subTest(args=args):
                result = self.run_stage(*args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(self.calls.exists())
                self.assertFalse((self.root / "target").exists())
                self.assertFalse(self.summary.exists())


if __name__ == "__main__":
    unittest.main()
