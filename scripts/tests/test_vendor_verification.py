"""Ensure patched snapshot hashes cannot hide undeclared upstream changes."""
import difflib
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


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
                path.write_bytes(patched)
                self.refresh_snapshot()

    def test_updated_snapshot_cannot_hide_added_file(self):
        self.root.joinpath("pepper-sync/extra.rs").write_text("// undeclared\n")
        self.refresh_snapshot()
        result = self.verify()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("upstream inventory differs", result.stderr)

    def test_modified_patch_is_rejected(self):
        self.root.joinpath(self.manifest["patches"][0]).write_text("changed patch\n")
        result = self.verify()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("vendor verification failed", result.stderr)
