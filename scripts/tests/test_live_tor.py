import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location('live', Path(__file__).parents[1] / 'check_live_tor.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


class LiveIdentityTests(unittest.TestCase):
    def setUp(self):
        self.expected = {label: dict(user='u', password=label, kind=kind, target=target)
                         for label, kind, target in [('old', 'evm', None), ('new', 'evm', None),
                                                     ('treasury', 'treasury', None),
                                                     ('catalog', 'discovery', 'provider.example:443')]}
        self.events = []
        for n, label in enumerate(self.expected, 1):
            self.events += [f'650 STREAM {n} NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD={label}',
                            f'650 STREAM {n} SUCCEEDED {n} 192.0.2.1:443']

    def test_rotation_identities_and_redaction(self):
        result = live.isolation_evidence(self.expected, self.events)
        self.assertEqual(len(result), 4)
        self.assertEqual(result['old']['successful_streams'], 1)
        self.assertNotIn('password', str(result))
        self.assertNotIn('provider.example', str(result))

    def test_missing_retired_wallet_and_cross_identity_reuse(self):
        for events in ([x for x in self.events if not x.startswith('650 STREAM 1 ')],
                       [x.replace('2 SUCCEEDED 2', '2 SUCCEEDED 1') for x in self.events]):
            with self.assertRaises(ValueError): live.isolation_evidence(self.expected, events)

    def test_unknown_credentials_changed_credentials_and_local_dns(self):
        bad = [self.events + ['650 STREAM 99 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=unknown'],
               self.events + ['650 STREAM 99 NEW 0 provider.example:443 PURPOSE=USER'],
               self.events + ['650 STREAM 1 CLOSED 1 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=new'],
               [x.replace('NEW 0 provider.example', 'NEW 0 192.0.2.1') for x in self.events],
               [x.replace('NEW 0 provider.example', 'NEW 0 [::1]') for x in self.events],
               [x.replace('4 NEW 0 provider.example', '4 NEW 0 different.example') for x in self.events]]
        for events in bad:
            with self.assertRaises(ValueError): live.isolation_evidence(self.expected, events)

    def test_unobserved_discovery_is_explicit_and_reuse_within_identity_allowed(self):
        events = self.events[:6] + ['650 STREAM 8 NEW 0 other.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=old',
                                  '650 STREAM 8 SUCCEEDED 8 192.0.2.1:443']
        result = live.isolation_evidence(self.expected, events)
        self.assertEqual(result['catalog']['status'], 'not_observed')
        self.assertEqual(result['old']['circuits'], 2)


if __name__ == '__main__': unittest.main()
