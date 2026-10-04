"""Offline checks of the real-Tor evidence verifier; no network or Tor needed."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("qualify_tor", Path(__file__).parents[1] / "qualify_tor.py")
qualification = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualification)


class CircuitEvidenceTests(unittest.TestCase):
    def test_listener_profile_preserves_outbound_restriction(self):
        profile = qualification.client_profile(9150, [8381, 8382, 8383])
        self.assertEqual(profile.count('network-outbound'), 1)
        self.assertIn('(deny network*)', profile)
        self.assertEqual(profile.count('network-inbound'), 3)
        self.assertEqual(profile.count('network-bind'), 3)
        self.assertNotIn('remote tcp "localhost:8381"', profile)
        for ports in ([9150], [8381, 8381], [0], [65536], ['8381'], [True]):
            with self.assertRaises(ValueError):
                qualification.client_profile(9150, ports)

    def setUp(self):
        labels = ["evm_a", "evm_b", "treasury", "discovery"]
        self.log = "\n".join("TOR_IDENTITY " + json.dumps(dict(label=label, user="user", password=label)) for label in labels)
        for label, protocol in [("evm_a", "https"), ("evm_b", "https"), ("evm_a", "https"),
                                ("discovery", "https"), ("treasury", "grpc"), ("evm_a", "grpc")]:
            self.log += "\nTOR_REQUEST " + json.dumps(dict(label=label, protocol=protocol))
        self.events = []
        for stream, label in enumerate(labels + ["evm_a"], 1):
            circuit = labels.index(label) + 10
            self.events.extend([
                f'650 STREAM {stream} NEW 0 zec.rocks:443 SOCKS_USERNAME="user" SOCKS_PASSWORD="{label}"',
                f'650 STREAM {stream} SENTCONNECT {circuit} zec.rocks:443',
                f'650 STREAM {stream} SUCCEEDED {circuit} 192.0.2.1:443',
            ])

    def test_complete_and_unrelated_events(self):
        events = ['650 CIRC 77 BUILT', '650 STREAM 88 NEW 0 elsewhere:80 SOCKS_USERNAME="unrelated" SOCKS_PASSWORD="token"', *self.events]
        evidence = qualification.circuit_evidence(self.log, events)
        self.assertEqual(evidence["evm_a"], {"circuits": ["10"], "streams": ["1", "5"]})

    def test_circuit_reuse_not_required(self):
        events = [line.replace("5 SENTCONNECT 10", "5 SENTCONNECT 99").replace("5 SUCCEEDED 10", "5 SUCCEEDED 99") for line in self.events]
        self.assertEqual(qualification.circuit_evidence(self.log, events)["evm_a"]["circuits"], ["10", "99"])

    def test_shared_circuit_rejected(self):
        with self.assertRaisesRegex(ValueError, "shared a Tor circuit"):
            qualification.circuit_evidence(self.log, [line.replace(" 11 ", " 10 ") for line in self.events])

    def test_missing_success_rejected(self):
        with self.assertRaisesRegex(ValueError, "missing successful"):
            qualification.circuit_evidence(self.log, [line for line in self.events if "2 SUCCEEDED" not in line])

    def test_local_resolution_rejected(self):
        with self.assertRaisesRegex(ValueError, "unexpected target"):
            qualification.circuit_evidence(self.log, [line.replace("zec.rocks", "192.0.2.1") for line in self.events])

    def test_changed_credentials_rejected(self):
        with self.assertRaisesRegex(ValueError, "credentials changed"):
            qualification.circuit_evidence(self.log, [*self.events, '650 STREAM 1 CLOSED 10 zec.rocks:443 SOCKS_USERNAME="user" SOCKS_PASSWORD="evm_b"'])

    def test_missing_request_rejected(self):
        with self.assertRaisesRegex(ValueError, "missing identity/request"):
            qualification.circuit_evidence(self.log.rsplit("\n", 1)[0], self.events)

    def test_missing_credentials_rejected(self):
        with self.assertRaisesRegex(ValueError, "missing successful"):
            qualification.circuit_evidence(self.log, [line.split(" SOCKS_USERNAME")[0] for line in self.events])


if __name__ == "__main__":
    unittest.main()
