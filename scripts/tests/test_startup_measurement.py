import importlib.util
from pathlib import Path
import tempfile
import tomllib
import unittest

spec = importlib.util.spec_from_file_location('measurement', Path(__file__).parents[1]/'measure_startup_tor.py')
measurement = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measurement)


class StartupMeasurementTests(unittest.TestCase):
    def test_requires_retained_learning_and_does_not_export_guard_state(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'state'
            for count in [0, 99]:
                path.write_text(f'TotalBuildTimes {count}\nGuard secret\n')
                with self.assertRaises(ValueError):
                    measurement.build_times(path)
            path.write_text('TotalBuildTimes 100\nCircuitBuildAbandonedCount 3\nGuard secret\n')
            self.assertEqual(measurement.build_times(path), {'TotalBuildTimes':'100', 'CircuitBuildAbandonedCount':'3'})

    def test_live_learning_overrides_stale_state_and_detects_reset(self):
        from types import SimpleNamespace
        class Control:
            events = []
            def command(self, command):
                assert command == 'GETINFO version'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'state'
            path.write_text('TotalBuildTimes 394\n')
            args = SimpleNamespace(tor_state=path, control=Control())
            with self.assertRaises(ValueError):
                measurement.learning_snapshot(args)
            args.control.events = ['650 BUILDTIMEOUT_SET COMPUTED TOTAL_TIMES=120 TIMEOUT_MS=5000']
            self.assertTrue(measurement.learning_snapshot(args)['qualified'])
            args.control.events.append('650 BUILDTIMEOUT_SET RESET TOTAL_TIMES=0')
            self.assertFalse(measurement.learning_snapshot(args, False)['qualified'])
            with self.assertRaises(ValueError):
                measurement.learning_snapshot(args)

    def test_issue_filter_records_all_tagged_exclusions_and_refuses_empty_cohort(self):
        import json
        from types import SimpleNamespace
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            clean, slow, other = [out/name for name in ['clean.toml', 'slow.toml', 'other.toml']]
            clean.write_text("spec = 'local.json'\nreliability_tags = []\n")
            slow.write_text("reliability_tags = ['slow_pricing']\n")
            other.write_text("reliability_tags = ['questionable_result_relevance']\n")
            args = SimpleNamespace(output=out, label='test', exclude_issue_tagged=True)
            with patch('builtins.print') as log:
                self.assertEqual(measurement.select_providers(args, [clean, slow, other]), [clean])
                self.assertIn('provider_issue_filter', log.call_args.args[0])
            exclusions = json.loads((out/'test-excluded-issues.json').read_text())
            self.assertEqual([x['reliability_tags'] for x in exclusions], [['slow_pricing'], ['questionable_result_relevance']])
            with patch('builtins.print'), self.assertRaisesRegex(ValueError, 'All providers excluded'):
                measurement.select_providers(args, [slow])
            args.exclude_issue_tagged = False
            self.assertEqual(measurement.select_providers(args, [slow]), [slow])

    def test_config_preserves_provider_limits_and_uses_only_tor(self):
        providers = [Path('/example/agent402.toml'), Path('/example/straits/provider.toml')]
        config = tomllib.loads(measurement.config(providers, 32, 9150, 'test_namespace'))
        self.assertEqual(config['startup']['catalog_concurrency'], 32)
        self.assertEqual(config['network']['mode'], 'tor')
        self.assertEqual(config['network']['socks_endpoint'], '127.0.0.1:9150')
        self.assertEqual(config['servers']['measure']['sources'], ['agent402', 'straits'])
        self.assertEqual(config['sources']['straits'], {'extends':'/example/straits/provider.toml'})
        self.assertNotIn('treasury', config)
        self.assertNotIn('funding', config)


class NewnymTests(unittest.IsolatedAsyncioTestCase):
    async def test_explicit_signal_and_completion_event_before_each_sample(self):
        from types import SimpleNamespace
        from unittest.mock import AsyncMock, patch
        import json
        class Control:
            def __init__(self):
                self.events = []
                self.commands = []
            def command(self, command):
                self.commands.append(command)
                self.events.append('650 SIGNAL NEWNYM')
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out/'provenance.json').write_text(json.dumps({'socks_port':9150, 'namespace':'fixed'}))
            (out/'cohort.json').write_text(json.dumps({'providers/exa.toml':{'returncode':0}}))
            args = SimpleNamespace(output=out, control=Control(), newnym=False, binary16=Path('unused'), label='test')
            with self.assertRaises(ValueError):
                await measurement.catalog_comparison(args)
            self.assertEqual(args.control.commands, [])
            args.newnym = True
            limits = []
            async def run(args, label, binary, path, catalog_only):
                config = tomllib.loads(path.read_text())
                limits.append(config['startup']['catalog_concurrency'])
                self.assertEqual(config['network']['isolation_namespace'], 'fixed')
                self.assertTrue(catalog_only)
                return {'qualified':True}
            with patch.object(measurement, 'run', run), patch.object(measurement.asyncio, 'sleep', AsyncMock()) as sleep, patch('builtins.print'):
                await measurement.catalog_comparison(args)
            self.assertEqual(limits, [3,16,32,3])
            self.assertEqual(args.control.commands, ['SIGNAL NEWNYM']*4)
            self.assertEqual(sleep.await_count, 4)
            args.control = SimpleNamespace(events=[], command=lambda _: [])
            with patch.object(measurement, 'run', AsyncMock()) as run, patch.object(measurement.asyncio, 'sleep', AsyncMock()), patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError, 'No observed NEWNYM'):
                    await measurement.catalog_comparison(args)
                run.assert_not_awaited()


    async def test_successful_cohort_can_exclude_timeout_retries(self):
        from types import SimpleNamespace
        from unittest.mock import AsyncMock, patch
        import json
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out/'provenance.json').write_text(json.dumps({'socks_port':9150, 'namespace':'fixed'}))
            failure = out/'failure.stderr'
            failure.write_text('connection timed out')
            (out/'cohort.json').write_text(json.dumps({
                'providers/exa.toml': {'returncode':0},
                'providers/regimeshift.toml': {'returncode':1, 'failure_log':str(failure)},
            }))
            events = []
            control = SimpleNamespace(events=events, command=lambda _: events.append('650 SIGNAL NEWNYM'))
            args = SimpleNamespace(output=out, control=control, newnym=True,
                binary16=Path('unused'), label='test', successful_cohort_only=True, catalog_limits=[16])
            async def run(args, label, binary, path, catalog_only):
                config = tomllib.loads(path.read_text())
                self.assertEqual(list(config['sources']), ['exa'])
                self.assertEqual(config['startup']['catalog_concurrency'], 16)
                return {'qualified':True}
            with patch.object(measurement, 'run', run), patch.object(measurement.asyncio, 'sleep', AsyncMock()), patch('builtins.print'):
                await measurement.catalog_comparison(args)
            self.assertEqual(json.loads((out/'catalog-comparison-cohort.json').read_text()), ['providers/exa.toml'])
            self.assertEqual(len(json.loads((out/'test-catalog-results.json').read_text())), 1)


if __name__ == '__main__':
    unittest.main()
