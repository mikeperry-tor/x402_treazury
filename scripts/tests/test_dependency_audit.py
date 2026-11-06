"""Offline advisory replay failures, nonregistry coverage and evidence identity."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'dependency_audit', Path(__file__).parents[1] / 'dependency_audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def report(count=0, settings=None):
    return json.dumps({'vulnerabilities': {'count': count, 'found': bool(count),
                                          'list': [{}] * count},
                       'settings': settings if settings is not None else {'informational_warnings': ['notice', 'unmaintained', 'unsound']}, 'warnings': {}})


class DependencyAuditTests(unittest.TestCase):
    def test_audit_distinguishes_findings_from_tool_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'report'
            for code, count, status in ((0, 0, 'clean'), (1, 1, 'findings')):
                with patch.object(audit, 'run', return_value=subprocess.CompletedProcess(
                        [], code, report(count), 'diagnostic')) as child:
                    result = audit.audit('cargo-audit', Path('/db'), Path('/lock'), output)
                self.assertEqual(result['status'], status)
                self.assertEqual(output.with_suffix('.stderr.txt').read_text(), 'diagnostic')
                args = child.call_args.args[0]
                for flag in ('--no-fetch', '--no-yanked', '--stale', '--json'):
                    self.assertIn(flag, args)
            for code, data in ((2, report()), (0, 'not JSON'), (0, report(1)),
                               (0, report(settings={'ignore': ['RUSTSEC-2026-0387']})),
                               (0, report(settings={'target_arch': ['x86_64']})),
                               (0, report(settings={'informational_warnings': []})),
                               (0, report(settings={'informational_warnings': ['notice']}))):
                with self.subTest(code=code, data=data), patch.object(
                        audit, 'run', return_value=subprocess.CompletedProcess([], code, data, '')):
                    with self.assertRaises(ValueError):
                        audit.audit('cargo-audit', Path('/db'), Path('/lock'), output)

    def test_informational_findings_cannot_be_reported_clean(self):
        data = json.loads(report())
        data['warnings'] = {'unmaintained': [{'advisory': {'id': 'RUSTSEC-2020-0001'}}]}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'report'
            for code in [0, 1]:
                with patch.object(audit, 'run', return_value=subprocess.CompletedProcess(
                        [], code, json.dumps(data), '')):
                    if code == 0:
                        with self.assertRaisesRegex(ValueError, 'successful exit despite'):
                            audit.audit('cargo-audit', Path('/db'), Path('/lock'), output)
                    else:
                        result = audit.audit('cargo-audit', Path('/db'), Path('/lock'), output)
                        self.assertEqual(result['status'], 'findings')
                        self.assertEqual(result['warnings'], data['warnings'])

    def test_inventory_preserves_versions_and_maps_nonregistry_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'tools/protoc').mkdir(parents=True)
            (root / 'Cargo.lock').write_text('''version = 4
[[package]]
name = "local-lib"
version = "0.5.5"
[[package]]
name = "git-lib"
version = "1.2.3"
source = "git+https://example.invalid/lib?rev=abc#abc"
[[package]]
name = "registry-lib"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
''')
            (root / 'tools/protoc/Cargo.lock').write_text('version = 4\npackage = []\n')
            sources = {}
            for name, version in [('local-lib', '0.5.5'), ('git-lib', '1.2.3')]:
                source = root / name
                source.mkdir()
                (source / 'Cargo.toml').write_text(
                    f'[package]\nname = "{name}"\nversion = "{version}"\n')
                sources[name] = source
            untracked = '.cargo-ok'
            def git(path, *args):
                if args[0] == 'status':
                    return ''
                if args[0] == 'ls-files':
                    return untracked
                if args == ('rev-parse', '--show-toplevel'):
                    return str(path)
                return 'a' * 40 if path == root / 'db' else 'abc'
            binary = root / 'cargo-audit'
            binary.write_bytes(b'operator audit tool')
            output = root / 'output'
            with patch.object(audit, 'git', side_effect=git), patch.object(
                    audit.shutil, 'which', return_value=str(binary)), patch.object(
                    audit, 'run', return_value=subprocess.CompletedProcess([], 0, 'cargo-audit 1', '')), \
                    patch.object(audit, 'source_hashes', return_value={'sha256': 'source'}), patch.object(
                    audit, 'audit', return_value={'exit_code': 0}) as child:
                status = audit.assess(root, root / 'db', 'a' * 40, sources, output, 'cargo-audit')
                untracked = '.cargo-ok\nsrc/unexpected.rs'
                with self.assertRaisesRegex(ValueError, 'untracked files'):
                    audit.assess(root, root / 'db', 'a' * 40, sources, output, 'cargo-audit')
                self.assertEqual(child.call_count, 3, 'reject extra source before audit')
                untracked = '.cargo-ok'
            with patch.object(audit, 'git', side_effect=git), patch.object(
                    audit.shutil, 'which', return_value=str(binary)), patch.object(
                    audit, 'run', return_value=subprocess.CompletedProcess([], 0, 'cargo-audit 1', '')), \
                    patch.object(audit, 'audit') as missing_source_audit:
                with self.assertRaisesRegex(ValueError, 'missing --source local-lib'):
                    audit.assess(root, root / 'db', 'a' * 40, {}, output, 'cargo-audit')
                missing_source_audit.assert_not_called()
            self.assertEqual(status, 0)
            self.assertEqual(child.call_count, 3)
            summary = json.loads((output / 'summary.json').read_text())
            identities = summary['nonregistry_sources']
            self.assertEqual([row['name'] for row in identities], ['local-lib', 'git-lib'])
            self.assertIsNone(identities[0]['original_source'])
            self.assertTrue(identities[1]['original_source'].startswith('git+'))
            self.assertEqual(summary['lockfiles'][0]['sha256'], audit.digest((root / 'Cargo.lock').read_bytes()))
            normalized = audit.tomllib.loads((output / 'nonregistry-inventory.lock').read_text())
            self.assertEqual([(p['name'], p['version']) for p in normalized['package']],
                             [('git-lib', '1.2.3'), ('local-lib', '0.5.5')])
            self.assertTrue(all(p['source'] == audit.REGISTRY for p in normalized['package']))

    def test_source_hash_changes_with_bytes_and_omits_ignored_files(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'lib.rs').write_text('first')
            (path / 'secret').write_text('must not read')
            (path / '.cargo-ok').write_text('Cargo cache metadata')
            with patch.object(audit, 'run', return_value=subprocess.CompletedProcess([], 0, '.cargo-ok\0lib.rs\0', '')):
                before = audit.source_hashes(path)
                (path / 'lib.rs').write_text('second')
                after = audit.source_hashes(path)
            self.assertNotEqual(before['sha256'], after['sha256'])
            self.assertEqual(list(after['files']), ['lib.rs'])


if __name__ == '__main__':
    unittest.main()
