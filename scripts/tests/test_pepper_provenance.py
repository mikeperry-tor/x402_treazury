"""Exercise maintained and restored inventories with a small reversible patch."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


VERIFY = Path(__file__).resolve().parents[2] / "vendor/verify_pepper.py"


class PepperProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "pepper-sync/src").mkdir(parents=True)
        shutil.copyfile(VERIFY, self.root / "verify_pepper.py")
        self.base = self.root / "pepper-sync/src/base.rs"
        self.base.write_text("// original upstream source\n")
        (self.root / "pepper-sync/src/added.rs").write_text("// local helper\n")
        (self.root / "progress.patch").write_text(
            "--- /dev/null\n+++ b/pepper-sync/src/added.rs\n"
            "@@ -0,0 +1 @@\n+// local helper\n"
        )
        self.manifest = {
            "revision": "synthetic-upstream",
            "patches": ["progress.patch"],
            "files": {},
            "original_files": {"src/base.rs": self.digest(self.base)},
        }
        self.update_maintained_hashes()

    @staticmethod
    def digest(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def update_maintained_hashes(self):
        self.manifest["files"] = {
            name: self.digest(self.root / name)
            for name in ["pepper-sync/src/base.rs", "pepper-sync/src/added.rs", "progress.patch"]
        }

    def verify(self):
        (self.root / "pepper-sync-sources.json").write_text(json.dumps(self.manifest))
        return subprocess.run(
            [sys.executable, str(self.root / "verify_pepper.py")],
            capture_output=True, text=True, check=False,
        )

    def test_declared_file_creation_restores_exact_upstream(self):
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "pepper-sync/src/added.rs").exists())
        self.assertEqual(self.digest(self.base), self.manifest["original_files"]["src/base.rs"])

    def test_undeclared_addition_is_rejected(self):
        (self.root / "pepper-sync/src/unlisted.rs").write_text("// unlisted\n")
        self.assertNotEqual(self.verify().returncode, 0)

    def test_added_file_remaining_after_reversal_is_rejected(self):
        self.manifest["patches"] = []
        self.assertNotEqual(self.verify().returncode, 0)

    def test_upstream_bytes_must_match_after_reversal(self):
        self.base.write_text("// modified without a declared patch\n")
        self.update_maintained_hashes()
        self.assertNotEqual(self.verify().returncode, 0)


if __name__ == "__main__":
    unittest.main()
