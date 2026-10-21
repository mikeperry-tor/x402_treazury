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
            with patch.dict(os.environ, {"EVM_PRIVATE_KEY": "secret", "RUSTFLAGS": "--bad", "LLVM_PROFILE_FILE": "unwanted", "HTTP_PROXY": "unwanted", "XDG_DATA_HOME": "/untrusted/shared-data"}):
                env = r.build_environment(work, work / "source", cache, ["/usr/bin"], "1", {"sdk_path": "/sdk", "deployment_target": "14.0"})
            for key in ("EVM_PRIVATE_KEY", "RUSTFLAGS", "LLVM_PROFILE_FILE", "HTTP_PROXY"):
                self.assertNotIn(key, env)
            self.assertEqual(env["CARGO_NET_OFFLINE"], "true")
            self.assertEqual(env["GIT_CEILING_DIRECTORIES"], str(work))
            self.assertEqual(env["XDG_DATA_HOME"], str(work / "home/.local/share"))
            self.assertTrue((work / "cargo-home/registry").is_symlink())
            self.assertFalse((work / "cargo-home/config.toml").exists())
            self.assertIn("--remap-path-prefix=", env["CARGO_ENCODED_RUSTFLAGS"])

    def test_macos_parameter_cache_follows_absolute_xdg_override(self):
        with patch.dict(os.environ, {"HOME": "/fixture/home"}, clear=True):
            self.assertEqual(r.parameter_directory(), Path("/fixture/home/.local/share/ZcashParams"))
            for override in ("", "relative/cache", "/fixture/data"):
                with patch.dict(os.environ, {"XDG_DATA_HOME": override}):
                    expected = "/fixture/data/ZcashParams" if override.startswith("/") else "/fixture/home/.local/share/ZcashParams"
                    self.assertEqual(r.parameter_directory(), Path(expected))

    def test_parameter_copies_are_independent_in_each_build_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            original = root / "original"
            original.mkdir()
            param = original / "fixture.params"
            param.write_bytes(b"good")
            expected = {param.name: (4, r.digest(param, "blake2b"))}
            copies = []
            with patch.object(r, "PARAMETERS", expected):
                r.verify_parameters(original)
                for label in ("first", "second-longer-path"):
                    work = root / label
                    work.mkdir()
                    env = r.build_environment(work, work / "source", root / "cache", ["/usr/bin"], "1", {"sdk_path": "/sdk", "deployment_target": "14.0"})
                    r.stage_parameters(original, env)
                    copy = Path(env["XDG_DATA_HOME"]) / "ZcashParams" / param.name
                    self.assertTrue(copy.is_relative_to(Path(env["HOME"])))
                    self.assertFalse(copy.is_symlink())
                    self.assertNotEqual(copy.stat().st_ino, param.stat().st_ino)
                    copies.append(copy)
                copies[0].write_bytes(b"evil")
                self.assertEqual(copies[1].read_bytes(), b"good")
                self.assertEqual(param.read_bytes(), b"good")

    def test_staging_rejects_missing_or_corrupt_copies(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            original = root / "original"
            original.mkdir()
            param = original / "fixture.params"
            param.write_bytes(b"good")
            expected = {param.name: (4, r.digest(param, "blake2b"))}
            with patch.object(r, "PARAMETERS", expected):
                for label, content in (("corrupt", b"evil"), ("missing", None)):
                    def broken_copy(source, destination):
                        if content is not None:
                            destination.write_bytes(content)
                    with patch.object(r.shutil, "copyfile", side_effect=broken_copy):
                        with self.assertRaisesRegex(RuntimeError, "Missing or invalid"):
                            r.stage_parameters(original, {"XDG_DATA_HOME": str(root / label)})
                param.unlink()
                with self.assertRaises(FileNotFoundError):
                    r.stage_parameters(original, {"XDG_DATA_HOME": str(root / "missing-original")})

    def test_failed_preflight_writes_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(r.subprocess, "run"), patch.object(r, "output", return_value=" M Cargo.toml"):
                with self.assertRaisesRegex(RuntimeError, "Tracked changes"):
                    r.qualify(Path(tmp))
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
