"""Ensure patched snapshot hashes cannot hide undeclared upstream changes."""
import difflib
import hashlib
import importlib.util
import json
import runpy
from pathlib import Path, PureWindowsPath
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


VERIFIER = Path(__file__).resolve().parents[2] / "vendor" / "verify_pepper.py"


class PepperVerificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        shutil.copyfile(VERIFIER, self.root / "verify_pepper.py")
        self.originals = {
            "Cargo.toml": '[dependencies]\nexample = { workspace = true }\n',
            "src/lib.rs": "pub fn value() -> u8 { 1 }\n",
        }
        suffixes = {name: "".join(f"{prefix} retained line {n}\n" for n in range(8))
                    for name, prefix in [("Cargo.toml", "#"), ("src/lib.rs", "//")]}
        self.originals = {name: text + suffixes[name] for name, text in self.originals.items()}
        current = dict(self.originals)
        patches = []
        for label, name, text in [
            ("manifest", "Cargo.toml", '[dependencies]\nexample = "1"\n'),
            ("progress", "src/lib.rs", "pub fn value() -> u8 { 2 }\n"),
            ("diagnostics", "src/lib.rs", "pub fn value() -> u8 { 3 }\n"),
        ]:
            text += suffixes[name]
            patch = f"pepper-sync-{label}.patch"
            patches.append(patch)
            self.root.joinpath(patch).write_text("".join(difflib.unified_diff(
                current[name].splitlines(keepends=True), text.splitlines(keepends=True),
                fromfile=f"a/pepper-sync/{name}", tofile=f"b/pepper-sync/{name}",
            )))
            current[name] = text
        for name, text in current.items():
            path = self.root / "pepper-sync" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.manifest = {
            "revision": "test-upstream",
            "patches": patches,
            "files": {},
            "original_files": {name: self.digest(text.encode())
                               for name, text in self.originals.items()},
        }
        self.refresh_snapshot()

    @staticmethod
    def digest(data):
        return hashlib.sha256(data).hexdigest()

    def refresh_snapshot(self):
        self.manifest["files"] = {
            path.relative_to(self.root).as_posix(): self.digest(path.read_bytes())
            for path in self.root.rglob("*")
            if path.is_file() and (path.suffix == ".patch" or "pepper-sync" in path.parts)
        }
        self.root.joinpath("pepper-sync-sources.json").write_text(json.dumps(self.manifest))

    def verify(self):
        return subprocess.run([sys.executable, str(self.root / "verify_pepper.py")],
                              capture_output=True, text=True)

    def test_declared_patch_chain_restores_upstream(self):
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("2 upstream files", result.stdout)

    def test_patch_bytes_and_working_directory_preserve_real_reverse_order(self):
        real_run = subprocess.run
        seen = []

        def reverse(arguments, **options):
            self.assertEqual(arguments, ["patch", "--binary", "--batch", "--fuzz=0", "-R", "-p2"])
            self.assertEqual(options["cwd"].name, "pepper-sync")
            self.assertNotEqual(options["cwd"], self.root / "pepper-sync")
            self.assertTrue(options["check"])
            self.assertTrue(options["capture_output"])
            self.assertNotIn("text", options)
            self.assertNotIn("encoding", options)
            expected = self.manifest["patches"][-len(seen) - 1]
            self.assertEqual(options["input"], self.root.joinpath(expected).read_bytes())
            seen.append(expected)
            return real_run(arguments, **options)

        with mock.patch.object(subprocess, "run", side_effect=reverse):
            runpy.run_path(str(self.root / "verify_pepper.py"), run_name="__main__")
        self.assertEqual(seen, list(reversed(self.manifest["patches"])))
        self.assertEqual((self.root / "pepper-sync/src/lib.rs").read_bytes(),
                         ("pub fn value() -> u8 { 3 }\n" +
                          "".join(f"// retained line {n}\n" for n in range(8))).encode())

    def test_crlf_reconstruction_is_fatal_even_when_normalizing_would_match(self):
        real_run = subprocess.run
        final_patch = self.root.joinpath(self.manifest["patches"][0]).read_bytes()
        expected = self.originals["Cargo.toml"].encode()
        changed = expected.replace(b"\n", b"\r\n")

        def reverse(arguments, **options):
            result = real_run(arguments, **options)
            if options["input"] == final_patch:
                restored = options["cwd"] / "Cargo.toml"
                self.assertEqual(restored.read_bytes(), expected)
                restored.write_bytes(changed)
            return result

        with mock.patch.object(subprocess, "run", side_effect=reverse):
            with self.assertRaises(SystemExit) as raised:
                runpy.run_path(str(self.root / "verify_pepper.py"), run_name="__main__")
        diagnostic = str(raised.exception)
        self.assertIn("expected_sha256=" + self.digest(expected), diagnostic)
        self.assertIn("actual_sha256=" + self.digest(changed), diagnostic)
        self.assertIn("crlf_to_lf_matches_expected=True (diagnostic only)", diagnostic)
        self.assertNotEqual(self.digest(expected), self.digest(changed))

    def test_failed_patch_preserves_captured_streams_and_exit_code(self):
        failure = subprocess.CalledProcessError(
            3, ["patch"], output=b"original patch output\n",
            stderr=b"original native failure \xff\n")
        with mock.patch.object(subprocess, "run", side_effect=failure):
            with self.assertRaises(SystemExit) as raised:
                runpy.run_path(str(self.root / "verify_pepper.py"), run_name="__main__")
        diagnostic = str(raised.exception)
        self.assertIn(self.manifest["patches"][-1], diagnostic)
        self.assertIn("exit 3", diagnostic)
        self.assertIn("stdout:\noriginal patch output", diagnostic)
        self.assertIn("stderr:\noriginal native failure \ufffd", diagnostic)

    def test_real_invalid_patch_failure_is_visible_after_snapshot_hash_accepts_it(self):
        self.root.joinpath(self.manifest["patches"][-1]).write_bytes(b"not a patch\n")
        self.refresh_snapshot()
        result = self.verify()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot reverse", result.stderr)
        self.assertIn("stderr:", result.stderr)
        self.assertNotIn("CalledProcessError", result.stderr)

    def test_updated_snapshot_hash_cannot_hide_source_or_manifest_changes(self):
        for name in self.originals:
            with self.subTest(name=name):
                path = self.root / "pepper-sync" / name
                patched = path.read_bytes()
                # Outside the patch hunk, so reversal succeeds before hash rejection.
                path.write_bytes(patched + b"\n# undeclared content\n")
                self.refresh_snapshot()
                result = self.verify()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("differs from upstream plus declared patches", result.stderr)
                self.assertIn("crlf_to_lf_matches_expected=False (diagnostic only)", result.stderr)
                path.write_bytes(patched)
                self.refresh_snapshot()

    def test_updated_snapshot_cannot_hide_added_file(self):
        valid = self.verify()
        self.assertEqual(valid.returncode, 0, valid.stderr)
        self.root.joinpath("pepper-sync/extra.rs").write_text("// undeclared\n")
        self.refresh_snapshot()
        result = self.verify()
        self.assertNotEqual(result.returncode, 0)

    def test_modified_patch_is_rejected(self):
        self.root.joinpath(self.manifest["patches"][0]).write_text("changed patch\n")
        result = self.verify()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("vendor verification failed", result.stderr)


class TransportVerificationTests(unittest.TestCase):
    def test_windows_inventory_keys_preserve_exact_reconstruction_and_membership(self):
        verifier = VERIFIER.with_name("verify_transport.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            copied = root / "verify_transport.py"
            shutil.copyfile(verifier, copied)
            original = b"pub fn value() -> u8 { 1 }\n"
            patched = b"pub fn value() -> u8 { 2 }\n"
            source = root / "sample" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_bytes(patched)
            patch = "".join(difflib.unified_diff(
                original.decode().splitlines(keepends=True),
                patched.decode().splitlines(keepends=True),
                fromfile="a/src/lib.rs", tofile="b/src/lib.rs",
            )).encode()
            root.joinpath("sample.patch").write_bytes(patch)
            notice = b"public license notice\n"
            root.joinpath("LICENSE-SAMPLE").write_bytes(notice)
            root.joinpath("transport-sources.json").write_text(json.dumps({
                "sample": {"version": "test", "patch": "sample.patch",
                           "notices": {"LICENSE-SAMPLE": {"sha256": hashlib.sha256(notice).hexdigest()}},
                           "patch_sha256": hashlib.sha256(patch).hexdigest(),
                           "original_files": {"src/lib.rs": hashlib.sha256(original).hexdigest()}}
            }))
            spec = importlib.util.spec_from_file_location("transport_inventory_fixture", copied)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            # Model a native Windows relative path while keeping the real files,
            # actual reverse-patch command and original-byte hash checks intact.
            entry = mock.Mock()
            entry.is_file.return_value = True
            entry.relative_to.return_value = PureWindowsPath("src/lib.rs")
            with mock.patch.object(Path, "rglob", return_value=[entry]):
                real_run = subprocess.run
                with mock.patch.object(module.subprocess, "run", wraps=real_run) as reverse:
                    module.verify()
                root.joinpath("LICENSE-SAMPLE").write_bytes(b"changed license")
                with self.assertRaisesRegex(SystemExit, "license notice differs"):
                    module.verify()
                root.joinpath("LICENSE-SAMPLE").write_bytes(notice)
                arguments, options = reverse.call_args
                self.assertEqual(arguments[0], ["patch", "--binary", "--batch", "--fuzz=0", "-R", "-p1"])
                self.assertEqual(options["input"], patch)
                self.assertEqual(options["cwd"].name, "sample")
                self.assertNotEqual(options["cwd"], root / "sample")
                self.assertTrue(options["check"])
                self.assertTrue(options["capture_output"])
                self.assertNotIn("text", options)
                self.assertNotIn("encoding", options)
                extra = mock.Mock()
                extra.is_file.return_value = True
                extra.relative_to.return_value = PureWindowsPath("src/undeclared.rs")
                with mock.patch.object(Path, "rglob", return_value=[entry, extra]):
                    with self.assertRaisesRegex(SystemExit, "unexpected/missing vendor files"):
                        module.verify()
                # A changed snapshot still cannot defeat the original source hash.
                source.write_bytes(patched + b"// undeclared content\n")
                with self.assertRaisesRegex(SystemExit, "differs from published source"):
                    module.verify()
                source.write_bytes(patched)

                def crlf_reconstruction(arguments, **options):
                    result = real_run(arguments, **options)
                    restored = options["cwd"] / "src/lib.rs"
                    self.assertEqual(restored.read_bytes(), original)
                    restored.write_bytes(original.replace(b"\n", b"\r\n"))
                    return result

                with mock.patch.object(module.subprocess, "run", side_effect=crlf_reconstruction):
                    with self.assertRaisesRegex(SystemExit, "crlf_to_lf_matches_expected=True") as raised:
                        module.verify()
                self.assertIn("expected_sha256=" + hashlib.sha256(original).hexdigest(), str(raised.exception))
                self.assertIn("actual_sha256=" + hashlib.sha256(original.replace(b"\n", b"\r\n")).hexdigest(),
                              str(raised.exception))
                self.assertEqual(source.read_bytes(), patched)
                root.joinpath("sample.patch").write_bytes(b"not a patch\n")
                manifest = json.loads(root.joinpath("transport-sources.json").read_text())
                manifest["sample"]["patch_sha256"] = hashlib.sha256(b"not a patch\n").hexdigest()
                root.joinpath("transport-sources.json").write_text(json.dumps(manifest))
                with self.assertRaisesRegex(SystemExit, "stderr:") as raised:
                    module.verify()
                self.assertIn("sample.patch", str(raised.exception))
                self.assertIn("cannot reverse", str(raised.exception))
