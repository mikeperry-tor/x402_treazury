"""Exercise the warning gate across real child-process output and exit boundaries."""
import json
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'check_warnings.py'


class WarningCheckTests(unittest.TestCase):
    def run_child(self, records, status=0):
        output = ''.join(json.dumps(record) + '\n' for record in records)
        child = ('import sys; '
                 f'sys.stdout.write({output!r}); '
                 'sys.stderr.write("cargo progress\\n"); '
                 f'sys.exit({status})')
        return subprocess.run([sys.executable, str(SCRIPT), sys.executable, '-c', child],
                              capture_output=True, text=True)

    @staticmethod
    def diagnostic(level, rendered):
        return {'reason': 'compiler-message', 'package_id': 'path+file:///vendor/rmcp#3.5.0',
                'message': {'level': level, 'message': 'unused method', 'rendered': rendered}}

    def test_dependency_warning_fails_even_after_successful_cached_build(self):
        warning = self.diagnostic('warning', 'warning: unused method\n')
        for fresh in (False, True):
            with self.subTest(fresh=fresh):
                result = self.run_child([warning, {'reason': 'compiler-artifact', 'fresh': fresh},
                                         {'reason': 'build-finished', 'success': True}])
                self.assertEqual(result.returncode, 1)
                self.assertIn('warning: unused method', result.stderr)
                self.assertIn('cargo progress', result.stderr)
                self.assertEqual(result.stdout, '')

    def test_notes_and_artifacts_succeed(self):
        result = self.run_child([self.diagnostic('note', 'note: context\n'),
                                 {'reason': 'compiler-artifact', 'fresh': True}])
        self.assertEqual(result.returncode, 0)
        self.assertIn('note: context', result.stderr)

    def test_cargo_failure_status_and_error_are_preserved(self):
        result = self.run_child([self.diagnostic('error', 'error: compilation failed\n'),
                                 self.diagnostic('warning', None)], status=42)
        self.assertEqual(result.returncode, 42)
        self.assertIn('error: compilation failed', result.stderr)
        self.assertIn('unused method', result.stderr)

    def test_unexpected_output_is_visible_and_fails(self):
        result = subprocess.run([sys.executable, str(SCRIPT), sys.executable, '-c',
                                 'print("unexpected output")'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('unexpected output', result.stderr)
