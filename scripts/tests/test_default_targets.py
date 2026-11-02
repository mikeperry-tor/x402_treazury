"""Cargo supplies inventory/features; the root manifest supplies only bench flags."""
import copy
import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'default_targets.py'
spec = importlib.util.spec_from_file_location('default_targets', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DefaultTargetsTests(unittest.TestCase):
    def setUp(self):
        self.path = Path('/fixture with spaces é/Cargo.toml')
        self.targets = [self.target('lib', 'root', test=False), self.target('bin', 'app', test=False),
                        self.target('example', 'example', test=False), self.target('test', 'compat'),
                        self.target('test', 'future'), self.target('test', 'disabled', test=False),
                        self.target('test', 'bench_only', test=False),
                        self.target('bench', 'bench_default', test=False),
                        self.target('bench', 'disabled_bench', test=False),
                        self.target('bench', 'test_only_bench'),
                        self.target('test', 'gated', required=['optional'])]
        self.metadata = {'packages': [{'id': 'root', 'manifest_path': str(self.path),
                                      'targets': self.targets},
                                     {'id': 'tool', 'manifest_path': '/tool/Cargo.toml',
                                      'targets': [self.target('test', 'excluded_tool')]}],
                         'workspace_default_members': ['root'],
                         'resolve': {'nodes': [{'id': 'root', 'features': []}]}}
        self.manifest = {'test': [{'name': 'compat', 'bench': True},
                                  {'name': 'disabled', 'bench': False},
                                  {'name': 'bench_only', 'bench': True, 'harness': False}],
                         'bench': [{'name': 'disabled_bench', 'bench': False},
                                   {'name': 'test_only_bench', 'bench': False}]}

    def target(self, kind, name, test=True, required=()):
        return {'kind': [kind], 'name': name, 'src_path': str(self.path.parent / f'{name}.rs'),
                'test': test, 'required-features': list(required)}

    def select(self, compat=('compat',)):
        return module.remaining_targets(self.metadata, self.manifest, self.path, list(compat))

    def test_all_target_test_or_bench_union_excludes_compat_and_disabled_targets(self):
        self.assertEqual(self.select(), ['--lib', '--bins', '--examples',
                                        '--bench', 'bench_default', '--bench', 'test_only_bench',
                                        '--test', 'bench_only', '--test', 'future'])
        self.assertNotIn('--benches', self.select())
        self.assertNotIn('--tests', self.select())
        # Cargo's build script is built by Cargo, not run as a selected harness.
        self.targets.append(self.target('custom-build', 'build-script-build'))
        self.assertEqual(self.select().count('--lib'), 1)

    def test_resolved_features_control_named_targets_without_a_feature_parser(self):
        self.metadata['resolve']['nodes'][0]['features'] = ['optional']
        self.assertEqual(self.select()[-2:], ['--test', 'gated'])

    def test_no_library_and_multiple_library_crate_types(self):
        self.targets.pop(0)
        self.assertNotIn('--lib', self.select())
        self.targets.append({'kind': ['rlib', 'cdylib'], 'name': 'root', 'test': False})
        self.assertEqual(self.select()[0], '--lib')

    def test_manifest_path_matching_is_canonical_and_target_kinds_are_distinct(self):
        self.targets.append(self.target('test', 'space name', test=False))
        self.targets.append(self.target('bench', 'future', test=False))
        self.manifest['test'].append({'path': './sub/../space name.rs', 'bench': True})
        selected = self.select()
        self.assertIn('space name', selected)
        self.assertEqual(selected.count('future'), 2)

    def test_conflicting_ambiguous_or_nonboolean_manifest_overrides_fail(self):
        for row in [{'name': 'future', 'path': 'bench_only.rs'},
                    {'name': 'future', 'path': 'missing.rs'},
                    {'name': 'missing'}, {'name': 'future', 'bench': 'false'},
                    {'name': 'compat', 'bench': True}]:
            with self.subTest(row=row):
                saved = copy.deepcopy(self.manifest)
                self.manifest['test'].append(row)
                with self.assertRaises(ValueError):
                    self.select()
                self.manifest = saved

    def test_missing_duplicate_or_gated_compat_is_never_silently_skipped(self):
        for compat in [[], ['compat', 'compat'], ['missing'], ['gated']]:
            with self.subTest(compat=compat), self.assertRaises(ValueError):
                self.select(compat)
        self.targets.append(self.target('test', 'compat'))
        with self.assertRaises(ValueError):
            self.select()

    def test_changed_root_workspace_selection_or_missing_resolution_fails(self):
        for field, value in [('workspace_default_members', ['root', 'tool']),
                             ('workspace_default_members', []), ('resolve', None),
                             ('resolve', {'nodes': []}), ('packages', [])]:
            with self.subTest(field=field, value=value):
                saved = self.metadata[field]
                self.metadata[field] = value
                with self.assertRaises((ValueError, TypeError)):
                    self.select()
                self.metadata[field] = saved

    def test_empty_remainder_and_unknown_target_kind_fail(self):
        self.targets[:] = [self.target('test', 'compat')]
        self.manifest = {}
        with self.assertRaisesRegex(ValueError, 'no eligible targets'):
            self.select()
        self.targets.append(self.target('future-kind', 'unknown'))
        with self.assertRaisesRegex(ValueError, 'unsupported root target kind'):
            self.select()


if __name__ == '__main__':
    unittest.main()
