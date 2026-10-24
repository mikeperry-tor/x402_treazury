import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from contextlib import ExitStack
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
            command = ["make", "--no-print-directory", "-C", str(r.ROOT), "toolchain"]
            subprocess.run(command, env=env, check=True)
            (Path(tmp) / "rustc").write_text("#!/bin/sh\necho 'rustc 1.98.0 (fixture)'\n")
            failure = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(failure.returncode, 0)
            self.assertIn("Toolchain mismatch", failure.stderr)


class ZingolibReleaseInputsTests(unittest.TestCase):
    def metadata(self, directory, graph="root"):
        return {"packages": [
            {"name": "x402_treazury" if graph == "root" else "x402-zingolib-compat",
             "manifest_path": str(directory.parent / graph / "Cargo.toml")},
            {"name": "zingolib", "id": "git+https://example.invalid/sdk#zingolib@7.0.0",
             "source": "git+https://example.invalid/sdk?rev=fixture#fixture",
             "version": "7.0.0", "manifest_path": str(directory / "Cargo.toml")},
        ]}

    def git(self, directory, env, *arguments):
        return subprocess.check_output(["git", *arguments], cwd=directory, env=env,
                                       stderr=subprocess.STDOUT, text=True).strip()

    def repository(self, directory, env):
        directory.mkdir()
        self.git(directory, env, "init", "-q")
        (directory / "Cargo.toml").write_text('[package]\nname = "zingolib"\nversion = "7.0.0"\n')
        self.git(directory, env, "add", "Cargo.toml")
        self.git(directory, env, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=", "commit", "-qm", "fixture")

    def environment(self, root):
        return {"PATH": os.environ["PATH"], "HOME": str(root), "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CEILING_DIRECTORIES": str(root)}

    def test_root_and_compatibility_metadata_find_resolved_source_without_vendor_paths(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(r, "SANDBOX", []):
            root = Path(tmp)
            env = self.environment(root)
            sdk = root / "cached-upstream"
            self.repository(sdk, env)
            revision = self.git(sdk, env, "rev-parse", "HEAD")
            for graph in ("root", "compatibility"):
                with self.subTest(graph=graph):
                    inputs = r.zingolib_inputs(self.metadata(sdk, graph), env)
                    self.assertEqual(inputs["manifest_path"], str(sdk / "Cargo.toml"))
                    self.assertEqual(inputs["version"], "7.0.0")
                    self.assertEqual(inputs["source"], self.metadata(sdk)["packages"][1]["source"])
                    self.assertEqual(inputs["git"]["revision"]["stdout"], revision)
                    self.assertEqual(inputs["git"]["tags"]["stdout"], "")
                    self.assertEqual(inputs["git"]["diff_index"]["returncode"], 0)

    def test_matching_tags_and_dirty_status_are_observed_without_predicting_descriptor(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(r, "SANDBOX", []):
            root = Path(tmp)
            env = self.environment(root)
            sdk = root / "upstream"
            self.repository(sdk, env)
            for tag in ("zingolib_v7.0.0", "zingolib_v7.2.0", "other_v99"):
                self.git(sdk, env, "tag", tag)
            clean = r.zingolib_inputs(self.metadata(sdk), env)
            self.assertEqual(clean["git"]["tags"]["stdout"].splitlines(),
                             ["zingolib_v7.2.0", "zingolib_v7.0.0"])
            (sdk / "untracked").write_text("untracked files do not make upstream diff-index dirty")
            self.assertEqual(r.zingolib_inputs(self.metadata(sdk), env)["git"]["diff_index"]["returncode"], 0)
            with (sdk / "Cargo.toml").open("a") as stream:
                stream.write("# tracked modification\n")
            dirty = r.zingolib_inputs(self.metadata(sdk), env)
            self.assertEqual(dirty["git"]["diff_index"]["returncode"], 1)
            self.assertEqual(dirty["git"]["revision"], clean["git"]["revision"])
            self.assertNotIn("descriptor", dirty)

    def test_missing_git_repository_preserves_failures_instead_of_claiming_clean(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(r, "SANDBOX", []):
            root = Path(tmp)
            sdk = root / "archive"
            sdk.mkdir()
            inputs = r.zingolib_inputs(self.metadata(sdk), self.environment(root))
            for observed in inputs["git"].values():
                self.assertNotEqual(observed["returncode"], 0)
                self.assertEqual(observed["stdout"], "")
                self.assertTrue(observed["stderr"])
            self.assertNotIn("descriptor", inputs)

    def test_discovery_rejects_missing_or_ambiguous_sdk_packages(self):
        package = self.metadata(Path("/fixture/upstream"))["packages"][1]
        for packages in ([], [package, package]):
            with self.subTest(packages=len(packages)):
                with self.assertRaisesRegex(RuntimeError, "one resolved zingolib"):
                    r.zingolib_inputs({"packages": packages}, {})

    def test_generated_descriptor_is_actual_build_output_and_requires_unique_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, "found 0"):
                r.generated_zingolib_descriptor(target)
            path = target / "triple/release/build/zingolib-first/out/git_description.txt"
            path.parent.mkdir(parents=True)
            path.write_text("zl_7.2.0_dirty")
            self.assertEqual(r.generated_zingolib_descriptor(target),
                             {"path": str(path), "text": "zl_7.2.0_dirty"})
            other = target / "triple/release/build/zingolib-second/out/git_description.txt"
            other.parent.mkdir(parents=True)
            other.write_text("different")
            with self.assertRaisesRegex(RuntimeError, "found 2"):
                r.generated_zingolib_descriptor(target)

    def test_metadata_capture_keeps_diagnostics_out_of_json_and_uses_isolated_environment(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(r, "SANDBOX", []):
            root = Path(tmp)
            command = [r.sys.executable, "-c",
                       'import json, os, sys; print("fixture warning", file=sys.stderr); '
                       'print(json.dumps({"home": os.environ["HOME"], "packages": []}))']
            captured = r.capture_logged(command, root, {"HOME": "/isolated"}, root / "log")
            self.assertEqual(json.loads(captured), {"home": "/isolated", "packages": []})
            self.assertIn("fixture warning", (root / "log").read_text())
            self.assertIn('"packages": []', (root / "log").read_text())

    def test_build_failure_preserves_observed_inputs_and_generated_output_when_available(self):
        for phase in ("compile", "smoke"):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as tmp, ExitStack() as patches:
                root = Path(tmp)
                (root / "scripts").mkdir()
                (root / "scripts/release-environment.json").write_text('{"host": "fixture"}')
                run_directory = root / "evidence"
                run_directory.mkdir()
                inputs = {"source": "git+fixture", "git": {"diff_index": {"returncode": 0}}}
                descriptor = {"path": "fixture/out/git_description.txt", "text": "actual-fixture-output"}

                def logged(command, *args):
                    if (phase == "compile" and "--release" in command) or (phase == "smoke" and "--help" in command):
                        raise RuntimeError("fixture " + phase + " failure")

                replacements = {
                    "ROOT": root,
                    "check_environment": lambda profile: {},
                    "output": lambda command, **kwargs: "" if "status" in command else "fixture",
                    "digest": lambda *args: "fixture-hash",
                    "parameter_directory": lambda: root / "params",
                    "verify_parameters": lambda directory: {},
                    "stage_parameters": lambda *args: None,
                    "build_environment": lambda *args: {"CARGO_TARGET_DIR": str(root / "target")},
                    "run_logged": logged,
                    "capture_logged": lambda *args: '{"packages": []}',
                    "zingolib_inputs": lambda *args: dict(inputs),
                    "generated_zingolib_descriptor": lambda *args: descriptor,
                }
                for name, value in replacements.items():
                    patches.enter_context(patch.object(r, name, value))
                patches.enter_context(patch.object(r.subprocess, "run"))
                patches.enter_context(patch.object(r.shutil, "which", return_value="/fixture/bin/tool"))
                with self.assertRaisesRegex(RuntimeError, "fixture " + phase + " failure"):
                    r.qualify(run_directory)
                report = json.loads((run_directory / "report.json").read_text())
                self.assertEqual(report["status"], "failed")
                self.assertEqual(report["builds"][0]["label"], "first")
                self.assertEqual(report["builds"][0]["status"], "failed")
                self.assertIn(phase, report["builds"][0]["error"])
                self.assertEqual(report["builds"][0]["zingolib"]["source"], inputs["source"])
                if phase == "smoke":
                    self.assertEqual(report["builds"][0]["zingolib"]["descriptor"], descriptor)
                else:
                    self.assertNotIn("descriptor", report["builds"][0]["zingolib"])
