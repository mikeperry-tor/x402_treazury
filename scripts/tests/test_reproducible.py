import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("reproducible", Path(__file__).parents[1] / "reproducible.py")
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


class ReproducibleTests(unittest.TestCase):
    def test_mismatch_preserves_both_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            first, second = Path(tmp) / "a", Path(tmp) / "b"
            first.write_bytes(b"same")
            second.write_bytes(b"same")
            self.assertEqual(r.compare(first, second), r.digest(first))
            second.write_bytes(b"different")
            with self.assertRaisesRegex(RuntimeError, "binaries differ"):
                r.compare(first, second)
            self.assertEqual(first.read_bytes(), b"same")
            self.assertEqual(second.read_bytes(), b"different")

    def test_parameters_reject_missing_and_corrupt_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "fixture.params"
            path.write_bytes(b"good")
            expected = {path.name: (4, r.digest(path, "blake2b"))}
            with patch.object(r, "PARAMETERS", expected):
                self.assertIn(path.name, r.verify_parameters(root))
                path.write_bytes(b"evil")
                with self.assertRaisesRegex(RuntimeError, "invalid"):
                    r.verify_parameters(root)
                path.unlink()
                with self.assertRaisesRegex(RuntimeError, "Missing"):
                    r.verify_parameters(root)

    def test_environment_drops_secrets_configuration_and_instrumentation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache, work = root / "cache", root / "build"
            (cache / "registry").mkdir(parents=True)
            (cache / "config.toml").write_text("untrusted config")
            work.mkdir()
            with patch.dict(os.environ, {"EVM_PRIVATE_KEY": "secret", "RUSTFLAGS": "--bad", "LLVM_PROFILE_FILE": "unwanted", "HTTP_PROXY": "unwanted"}):
                env = r.build_environment(work, work / "source", cache, ["/usr/bin"], "1", {"sdk_path": "/sdk", "deployment_target": "14.0"})
            for key in ("EVM_PRIVATE_KEY", "RUSTFLAGS", "LLVM_PROFILE_FILE", "HTTP_PROXY"):
                self.assertNotIn(key, env)
            self.assertEqual(env["CARGO_NET_OFFLINE"], "true")
            self.assertEqual(env["GIT_CEILING_DIRECTORIES"], str(work))
            self.assertTrue((work / "cargo-home/registry").is_symlink())
            self.assertFalse((work / "cargo-home/config.toml").exists())
            self.assertIn("--remap-path-prefix=", env["CARGO_ENCODED_RUSTFLAGS"])

    def test_failed_preflight_writes_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(r.subprocess, "run"), patch.object(r, "output", return_value=" M Cargo.toml"):
                with self.assertRaisesRegex(RuntimeError, "Tracked changes"):
                    r.qualify(Path(tmp), False)
            report = json.loads((Path(tmp) / "report.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("Tracked changes", report["error"])
            self.assertEqual(report["artifacts"], [])

    def test_environment_mismatch_fails_before_building(self):
        with patch.object(r, "output", side_effect=["rustc\nhost: unexpected\ncommit-hash: wrong", "cargo", "clang", "linker", "sdk"]):
            with self.assertRaisesRegex(RuntimeError, "host"):
                r.check_environment({"host": "expected"})

    def test_homebrew_style_toolchain_selection_is_enforced(self):
        expected = tomllib.loads((r.ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        with tempfile.TemporaryDirectory() as tmp:
            for tool in ("rustc", "cargo"):
                path = Path(tmp) / tool
                path.write_text(f"#!/bin/sh\necho '{tool} {expected} (fixture)'\n")
                path.chmod(0o755)
            env = {"PATH": f"{tmp}:/usr/bin:/bin"}
            command = ["sh", str(r.ROOT / "scripts/check_toolchain.sh")]
            subprocess.run(command, env=env, check=True)
            (Path(tmp) / "rustc").write_text("#!/bin/sh\necho 'rustc 1.98.0 (fixture)'\n")
            failure = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(failure.returncode, 0)
            self.assertIn("Toolchain mismatch", failure.stderr)
