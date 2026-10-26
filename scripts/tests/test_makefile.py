"""Check Make's build authority, argument boundaries and ordered entrypoints offline."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class MakefileTests(unittest.TestCase):
    # Make exports command-line variables. Exercise the documented interpreter
    # override on every fixture setup without letting it select real child tools.
    @patch.dict(os.environ, {'PYTHON': sys.executable})
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / 'repo with spaces'
        self.root.mkdir()
        (self.root / 'bin').mkdir()
        shutil.copy(ROOT / 'Makefile', self.root)
        shutil.copy(ROOT / 'rust-toolchain.toml', self.root)
        self.log = self.root / 'calls.jsonl'
        version = next(line.split('"')[1] for line in
                       (ROOT / 'rust-toolchain.toml').read_text().splitlines()
                       if line.startswith('channel = '))
        mock = f'''#!{sys.executable}
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
if sys.argv[1:] == ['--version']:
    version = os.environ.get('FAKE_' + name.upper() + '_VERSION', {version!r})
    print(name + ' ' + version + ' (fixture)')
else:
    with open(os.environ['MAKE_TEST_LOG'], 'a') as log:
        log.write(json.dumps([name, sys.argv[1:], os.environ.get('CARGO_INCREMENTAL'), os.environ.get('PROTOC')]) + '\\n')
    if name == 'cargo' and sys.argv[1] == 'run' and 'compat/protoc/Cargo.toml' in sys.argv:
        if os.environ.get('FAIL_PROTOC'):
            sys.exit(42)
        print('/fixture/protoc with spaces')
    if os.environ.get('FAIL_ARGUMENT') in sys.argv[1:]:
        sys.exit(43)
'''
        for name in ('cargo', 'rustc', 'python3'):
            path = self.root / 'bin' / name
            path.write_text(mock)
            path.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.root / "bin"}:{os.environ["PATH"]}',
                        MAKE_TEST_LOG=str(self.log), CARGO_INCREMENTAL='inherited')
        for key in ('MAKEFLAGS', 'MFLAGS', 'MAKELEVEL', 'CARGO_ARGS', 'TEST_ARGS',
                    'PROFILE', 'COMPLEXITY_BASE_REF', 'TEST', 'PYTHON'):
            self.env.pop(key, None)

    def make(self, *args, success=True, **env):
        self.log.unlink(missing_ok=True)
        result = subprocess.run(['make', '--no-print-directory', '-C', str(self.root), *args],
                                env={**self.env, **env}, capture_output=True, text=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def test_profiles_and_argument_forwarding(self):
        cases = [
            ([], 'build', ['--profile', 'release'], '0'),
            (['build'], 'build', ['--profile', 'release'], '0'),
            (['build', 'PROFILE=dev'], 'build', ['--profile', 'dev'], '0'),
            (['build-dev'], 'build', ['--profile', 'dev'], '1'),
            (['build-dev', 'PROFILE=release'], 'build', ['--profile', 'dev'], '1'),
            (['build-dev', 'CARGO_ARGS=--target-dir "path with spaces" --offline'],
             'build', ['--profile', 'dev', '--target-dir', 'path with spaces', '--offline'], '1'),
            (['test'], 'test', ['--all-targets', '--', '--test-threads=1'], 'inherited'),
            (['test', 'CARGO_ARGS=--lib', 'TEST_ARGS=--exact "a b"'],
             'test', ['--lib', '--', '--exact', 'a b'], 'inherited'),
            (['cargo-check'], 'check', [], 'inherited'),
            (['clippy'], 'clippy', ['--all-targets', '--', '-D', 'warnings'], 'inherited'),
            (['run', 'CARGO_ARGS=--example diagnostic -- "a b"'],
             'run', ['--example', 'diagnostic', '--', 'a b'], 'inherited'),
        ]
        for args, command, forwarded, incremental in cases:
            with self.subTest(args=args):
                _, calls = self.make(*args)
                self.assertEqual(len(calls), 2)
                self.assertEqual(calls[0][1], ['run', '--quiet', '--locked', '--manifest-path', 'compat/protoc/Cargo.toml'])
                self.assertEqual(calls[0][2], incremental)
                self.assertEqual(calls[1], ['cargo', [command, '--locked', '--manifest-path',
                                                    'Cargo.toml', *forwarded], incremental,
                                            '/fixture/protoc with spaces'])

    def test_build_arguments_are_expanded_only_once(self):
        marker = self.root / 'argument-expansions'
        # The dollar signs pass through Make to a recipe-shell substitution.
        self.make('build', 'CARGO_ARGS=--target-dir "$$(printf x >> \"argument-expansions\"; printf \"path with spaces\")"')
        self.assertEqual(marker.read_text(), 'x')

    def test_run_environment_expansion_preserves_argument_boundaries(self):
        _, calls = self.make('run', 'CARGO_ARGS=--example diagnostic -- "$$MAKE_TEST_VALUE"',
                             MAKE_TEST_VALUE='value with spaces')
        self.assertEqual(calls[-1][1][-1], 'value with spaces')

    def test_conflicting_build_profiles_are_rejected_before_protoc(self):
        for target in ('build', 'build-dev'):
            for flag in ('--release', '-r', '--profile release', '--profile=release'):
                with self.subTest(target=target, flag=flag):
                    result, calls = self.make(target, f'CARGO_ARGS={flag}', success=False)
                    self.assertIn('Select the build profile', result.stderr)
                    self.assertEqual(calls, [])

    def test_protoc_failure_cannot_fall_back_or_build(self):
        _, calls = self.make('build', success=False, FAIL_PROTOC='yes', PROTOC='/inherited/protoc')
        self.assertEqual(len(calls), 1)
        self.assertIn('compat/protoc/Cargo.toml', calls[0][1])

    def test_toolchain_mismatch_stops_before_protoc(self):
        for name in ('RUSTC', 'CARGO'):
            result, calls = self.make('build', success=False, **{f'FAKE_{name}_VERSION': '0.0.0'})
            self.assertIn('Toolchain mismatch', result.stderr)
            self.assertEqual(calls, [])

    def test_help_and_dry_runs_do_not_execute_cargo(self):
        for args in (['help'], ['-n', 'build'], ['-n', 'check']):
            _, calls = self.make(*args, FAKE_RUSTC_VERSION='0.0.0')
            self.assertEqual(calls, [])

    def test_multiple_goals_remain_sequential_under_parallel_make(self):
        _, calls = self.make('-j8', 'build-dev', 'test')
        # No second protoc/build may interleave with the preceding command.
        self.assertEqual([call[1][0] for call in calls], ['run', 'build', 'run', 'test'])
        self.assertEqual([call[2] for call in calls], ['1', '1', 'inherited', 'inherited'])

    def check_stages(self):
        script = self.root / 'scripts/tests/coverage.sh'
        script.parent.mkdir(parents=True, exist_ok=True)
        script.write_text('#!/bin/sh\nexec python3 coverage-regression\n')
        script.chmod(0o755)
        def application(command, *args):
            return ('cargo', [command, '--locked', '--manifest-path', 'Cargo.toml', *args])
        return {
            'check-static': [
                ('cargo', ['fmt', '--check']),
                ('python3', ['coverage-regression']),
                ('python3', ['-m', 'unittest', 'discover', '-s', 'scripts/tests', '-p', 'test_*.py']),
                ('cargo', ['test', '--locked', '--manifest-path', 'tools/complexity/Cargo.toml', '--target-dir', 'target/tools/complexity']),
                ('python3', ['scripts/complexity.py', '--base', 'main']),
                ('python3', ['vendor/verify.py']),
                ('python3', ['vendor/verify_zingo.py']),
            ],
            'check-clippy': [application('clippy', '--all-targets', '--features', 'zcash-regtest', '--', '-D', 'warnings')],
            'check-compat': [('python3', ['scripts/check_compat.py'])],
            'check-default': [
                application('test', '-p', 'pepper-sync', '--lib', 'wallet::traits::diagnostic_tests::scan_merge_conflict_preserves_pool_context', '--', '--exact'),
                application('test', '--all-targets', '--', '--test-threads=1'),
            ],
            **{stage: [application('test', '--features', 'zcash-testutils', '--lib', case, '--', '--test-threads=1')]
               for stage, case in (
                   ('check-proving', 'treasury::send::proving_tests'),
                   ('check-lifecycle', 'treasury::lifecycle_tests'),
                   ('check-codec', 'synthetic_address_codec_is_scoped_to_its_test_client'))},
        }

    @staticmethod
    def check_commands(calls):
        return [(call[0], call[1]) for call in calls
                if 'compat/protoc/Cargo.toml' not in call[1]]

    def test_check_preserves_selections_and_orders_static_lints_compat_before_runtime(self):
        stages = self.check_stages()
        (self.root / 'check').touch()
        _, calls = self.make('-j8', 'check', 'CARGO_ARGS=--wrong', 'TEST_ARGS=--wrong')
        self.assertEqual(self.check_commands(calls), [command for stage in stages.values() for command in stage])

    def test_named_check_stages_are_independent_and_ignore_focused_run_overrides(self):
        for stage, expected in self.check_stages().items():
            with self.subTest(stage=stage):
                # A same-named file must never turn validation into a cached pass.
                (self.root / stage).touch()
                _, calls = self.make('-j8', stage, 'CARGO_ARGS=--wrong', 'TEST_ARGS=--wrong')
                self.assertEqual(self.check_commands(calls), expected)
                _, calls = self.make(stage, success=False, FAKE_CARGO_VERSION='0.0.0')
                self.assertEqual(calls, [])

    def test_resume_runs_only_requested_remaining_stages(self):
        stages = self.check_stages()
        for _ in range(2):
            _, calls = self.make('-j8', 'check-lifecycle', 'check-codec')
            self.assertEqual(self.check_commands(calls), stages['check-lifecycle'] + stages['check-codec'])

    def test_check_stops_at_failed_stage_even_with_keep_going(self):
        expected = [command for stage in self.check_stages().values() for command in stage]
        for flags in ([], ['-j8'], ['-k']):
            for failure in ('vendor/verify.py', 'clippy', 'scripts/check_compat.py',
                            'wallet::traits::diagnostic_tests::scan_merge_conflict_preserves_pool_context',
                            'treasury::send::proving_tests', 'treasury::lifecycle_tests',
                            'synthetic_address_codec_is_scoped_to_its_test_client'):
                with self.subTest(flags=flags, failure=failure):
                    _, calls = self.make(*flags, 'check', success=False, FAIL_ARGUMENT=failure)
                    stop = next(i for i, (_, args) in enumerate(expected) if failure in args) + 1
                    self.assertEqual(self.check_commands(calls), expected[:stop])

    def test_check_forwards_interpreter_and_complexity_baseline_to_stages(self):
        self.check_stages()
        shutil.copy(self.root / 'bin/python3', self.root / 'bin/analysis-python')
        _, calls = self.make('check', 'PYTHON=analysis-python', 'COMPLEXITY_BASE_REF=review-base')
        self.assertIn(('analysis-python', ['scripts/complexity.py', '--base', 'review-base']),
                      self.check_commands(calls))
        self.assertIn(('analysis-python', ['scripts/check_compat.py']), self.check_commands(calls))

    def test_coverage_modes_remain_explicit(self):
        script = self.root / 'scripts/coverage.sh'
        script.parent.mkdir(parents=True)
        script.write_text('#!/bin/sh\nexec python3 coverage-helper "$@"\n')
        script.chmod(0o755)
        for target, args in [('coverage', []), ('coverage-branch', ['--branch']),
                             ('coverage-proving', ['--proving']),
                             ('coverage-consensus', ['--consensus', 'named-case'])]:
            _, calls = self.make(target, 'TEST=named-case')
            self.assertEqual(calls[0][1], ['coverage-helper', *args])
        _, calls = self.make('coverage-consensus', success=False)
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
