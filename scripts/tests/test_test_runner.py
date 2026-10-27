"""Exercise platform selection, copied execution and failure/signal propagation."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class TestRunnerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = (Path(self.directory.name) / 'repo with spaces').resolve()
        scripts = self.root / 'scripts'
        scripts.mkdir(parents=True)
        for name in ('test-runner.sh', 'test-runner.toml'):
            shutil.copy(ROOT / 'scripts' / name, scripts / name)
        self.runner = scripts / 'test-runner.sh'
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'calls.jsonl'
        self.temp = self.root / 'private copies'
        self.temp.mkdir()
        mock = f'''#!{sys.executable}
import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
if name == 'uname':
    print(os.environ['TEST_PLATFORM'])
else:
    with open(os.environ['RUNNER_TEST_LOG'], 'a') as log:
        log.write(json.dumps([name, sys.argv[1:]]) + '\\n')
    if name == 'cp':
        if os.environ.get('FAIL_COPY'): sys.exit(41)
        assert sys.argv[1] == '-X'
        shutil.copy2(sys.argv[2], sys.argv[3])
    if name == 'codesign' and os.environ.get('FAIL_SIGN'): sys.exit(42)
'''
        for name in ('uname', 'cp', 'codesign', 'cargo'):
            p = self.bin / name
            p.write_text(mock)
            p.chmod(0o755)
        self.executable = self.root / 'fixture executable'
        self.executable.write_text(f'''#!{sys.executable}
import json, os, signal, sys
if os.environ.get('TEST_SIGNAL'): os.kill(os.getpid(), signal.SIGTERM)
print(json.dumps({{'executable': sys.argv[0], 'arguments': sys.argv[1:],
                  'cwd': os.getcwd(), 'input': sys.stdin.read(),
                  'environment': os.environ.get('TEST_VALUE')}}))
sys.exit(int(os.environ.get('TEST_EXIT', '0')))
''')
        self.executable.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}',
                        TEST_PLATFORM='Darwin', RUNNER_TEST_LOG=str(self.log),
                        TMPDIR=str(self.temp), TEST_VALUE='inherited value')

    def run_script(self, arguments, **environment):
        return subprocess.run([str(self.runner), *arguments], cwd=self.root,
                              env={**self.env, **environment}, input='wire input',
                              capture_output=True, text=True)

    def calls(self):
        return [json.loads(s) for s in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_macos_copy_preserves_original_arguments_io_and_environment(self):
        original = self.executable.read_bytes()
        r = self.run_script([str(self.executable), 'space argument', '$literal'], TEST_EXIT='17')
        self.assertEqual(r.returncode, 17, r.stderr)
        value = json.loads(r.stdout)
        copy = Path(value['executable'])
        self.assertNotEqual(copy, self.executable)
        self.assertEqual(copy.read_bytes(), original)
        self.assertEqual(self.executable.read_bytes(), original)
        self.assertEqual(copy.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual(value['arguments'], ['space argument', '$literal'])
        self.assertEqual(value['cwd'], str(self.root))
        self.assertEqual(value['input'], 'wire input')
        self.assertEqual(value['environment'], 'inherited value')
        self.assertEqual([c[0] for c in self.calls()], ['cp', 'codesign'])
        self.assertEqual(self.calls()[1][1], ['--force', '--sign', '-', str(copy)])

    def test_non_macos_executes_original_on_linux_and_windows_shells(self):
        for platform in ('Linux', 'MINGW64_NT-10.0', 'MSYS_NT-10.0'):
            with self.subTest(platform=platform):
                r = self.run_script([str(self.executable), 'argument'], TEST_PLATFORM=platform)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(json.loads(r.stdout)['executable'], str(self.executable))
                self.assertEqual(self.calls(), [])

    def test_copy_and_sign_failures_do_not_execute_original(self):
        for variable, status in [('FAIL_COPY', 41), ('FAIL_SIGN', 42)]:
            with self.subTest(variable=variable):
                self.log.unlink(missing_ok=True)
                r = self.run_script([str(self.executable)], **{variable: '1'})
                self.assertEqual(r.returncode, status)
                self.assertEqual(r.stdout, '')
                self.assertEqual(len(self.calls()), 1 if variable == 'FAIL_COPY' else 2)

    @unittest.skipUnless(os.name == 'posix', 'POSIX runner signal contract')
    def test_copied_process_retains_signal_status(self):
        r = self.run_script([str(self.executable)], TEST_SIGNAL='1')
        self.assertEqual(r.returncode, -signal.SIGTERM, r.stderr)

    def test_cargo_config_precedes_all_arguments_without_rewriting_targets(self):
        args = ['test', '--target', 'aarch64-unknown-linux-gnu', '--', '--exact', 'a b']
        r = self.run_script(['--cargo', 'cargo', *args])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), [['cargo', ['--config', str(self.root / 'scripts/test-runner.toml'), *args]]])

    def test_non_macos_cargo_arguments_are_unchanged(self):
        args = ['test', '--target', 'x86_64-apple-darwin', '--', '--exact', 'a b']
        r = self.run_script(['--cargo', 'cargo', *args], TEST_PLATFORM='Linux')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), [['cargo', args]])


if __name__ == '__main__':
    unittest.main()
