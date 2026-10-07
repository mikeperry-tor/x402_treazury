"""Exercise wrapper profile selection without compiling or downloading anything."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class ZcashWrapperTests(unittest.TestCase):
    def test_command_profiles_and_forwarding(self):
        root = Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory)
            (fixture / 'scripts').mkdir()
            (fixture / 'bin').mkdir()
            for name in ('zcash.sh', 'check_toolchain.sh'):
                shutil.copy(root / 'scripts' / name, fixture / 'scripts' / name)
            shutil.copy(root / 'rust-toolchain.toml', fixture / 'rust-toolchain.toml')
            version = next(line.split('"')[1] for line in
                           (root / 'rust-toolchain.toml').read_text().splitlines()
                           if line.startswith('channel = '))
            mock = f'''#!{sys.executable}
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
if sys.argv[1:] == ['--version']:
    print(name + ' {version} (fixture)')
else:
    with open(os.environ['WRAPPER_LOG'], 'a') as log:
        log.write(json.dumps([sys.argv[1:], os.environ.get('CARGO_INCREMENTAL')]) + '\\n')
    if sys.argv[1] == 'run':
        print('/fixture/protoc')
'''
            for name in ('cargo', 'rustc'):
                path = fixture / 'bin' / name
                path.write_text(mock)
                path.chmod(0o755)
            log = fixture / 'calls.jsonl'
            env = dict(os.environ, PATH=f'{fixture / "bin"}:{os.environ["PATH"]}',
                       WRAPPER_LOG=str(log), CARGO_INCREMENTAL='inherited')
            cases = [
                ([], 'build', ['--release'], '0'),
                (['build'], 'build', ['--release'], '0'),
                (['build', '--release'], 'build', ['--release'], '0'),
                (['build', '-r'], 'build', ['-r'], '0'),
                (['build', '--profile', 'dev'], 'build', ['--profile', 'dev'], '0'),
                (['build', '--profile=dev'], 'build', ['--profile=dev'], '0'),
                (['build', '--developer'], 'build', [], '1'),
                (['build', '--target-dir', 'path with spaces', '--developer', '--offline'],
                 'build', ['--target-dir', 'path with spaces', '--offline'], '1'),
                (['test', '--', '--developer'], 'test', ['--', '--developer'], 'inherited'),
                (['test', '--lib', '--', '--exact', 'a b'],
                 'test', ['--lib', '--', '--exact', 'a b'], 'inherited'),
                (['check'], 'check', [], 'inherited'),
                (['clippy', '--', '-D', 'warnings'],
                 'clippy', ['--', '-D', 'warnings'], 'inherited'),
            ]
            for args, command, forwarded, incremental in cases:
                with self.subTest(args=args):
                    log.unlink(missing_ok=True)
                    result = subprocess.run(['sh', str(fixture / 'scripts/zcash.sh'), *args],
                                            env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    self.assertEqual(len(calls), 2)
                    self.assertEqual(calls[0][1], incremental)
                    self.assertEqual(calls[1], [[command, '--locked', '--manifest-path',
                                               str(fixture / 'Cargo.toml'), *forwarded], incremental])
            for args in (['build', '--developer', '--release'],
                         ['build', '--profile=dev', '--developer'],
                         ['test', '--developer']):
                with self.subTest(rejected=args):
                    log.unlink(missing_ok=True)
                    result = subprocess.run(['sh', str(fixture / 'scripts/zcash.sh'), *args],
                                            env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 2)
                    self.assertIn('--developer', result.stderr)
                    self.assertFalse(log.exists())


if __name__ == '__main__':
    unittest.main()
