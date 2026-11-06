#!/usr/bin/env python3
"""Reverse reviewed dependency patches and verify every published source byte."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def verify(manifest_name="transport-sources.json"):
    root = Path(__file__).resolve().parent
    manifest = json.loads((root / manifest_name).read_text())
    for name, entry in manifest.items():
        for relative, notice in entry.get("notices", {}).items():
            if hashlib.sha256((root / relative).read_bytes()).hexdigest() != notice["sha256"]:
                raise SystemExit(f"{name}: license notice differs: {relative}")
        source = root / name
        expected = entry["original_files"]
        actual = {p.relative_to(source).as_posix() for p in source.rglob("*") if p.is_file()}
        if actual != set(expected):
            raise SystemExit(f"{name}: unexpected/missing vendor files: {actual ^ set(expected)}")
        patch = root / entry.get("patch", f"{name}-transport.patch")
        if hashlib.sha256(patch.read_bytes()).hexdigest() != entry["patch_sha256"]:
            raise SystemExit(f"{name}: patch differs from review")
        with tempfile.TemporaryDirectory() as directory:
            restored = Path(directory) / name
            shutil.copytree(source, restored)
            try:
                subprocess.run(["patch", "--binary", "--batch", "--fuzz=0", "-R", "-p1"],
                               cwd=restored, input=patch.read_bytes(),
                               check=True, capture_output=True)
            except subprocess.CalledProcessError as error:
                raise SystemExit(
                    f"{name}: cannot reverse {patch.name} (exit {error.returncode})\n"
                    f"stdout:\n{error.stdout.decode('utf-8', errors='replace')}"
                    f"stderr:\n{error.stderr.decode('utf-8', errors='replace')}"
                ) from None
            for relative, digest in expected.items():
                restored_bytes = (restored / relative).read_bytes()
                actual_digest = hashlib.sha256(restored_bytes).hexdigest()
                if actual_digest != digest:
                    crlf_match = hashlib.sha256(restored_bytes.replace(b"\r\n", b"\n")).hexdigest() == digest
                    raise SystemExit(
                        f"{name}/{relative}: differs from published source plus reviewed patch; "
                        f"expected_sha256={digest} actual_sha256={actual_digest} "
                        f"crlf_to_lf_matches_expected={crlf_match} (diagnostic only)"
                    )
        print(f"Verified {name} {entry['version']}: published source plus reviewed patch")


if __name__ == "__main__":
    verify()
