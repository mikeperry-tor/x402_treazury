#!/usr/bin/env python3
"""Reverse reviewed transport patches and verify every published source byte."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def verify():
    root = Path(__file__).resolve().parent
    manifest = json.loads((root / "transport-sources.json").read_text())
    for name, entry in manifest.items():
        source = root / name
        expected = entry["original_files"]
        actual = {str(p.relative_to(source)) for p in source.rglob("*") if p.is_file()}
        if actual != set(expected):
            raise SystemExit(f"{name}: unexpected/missing vendor files: {actual ^ set(expected)}")
        patch = root / f"{name}-transport.patch"
        if hashlib.sha256(patch.read_bytes()).hexdigest() != entry["patch_sha256"]:
            raise SystemExit(f"{name}: transport patch differs from review")
        with tempfile.TemporaryDirectory() as directory:
            restored = Path(directory) / name
            shutil.copytree(source, restored)
            subprocess.run(["patch", "--batch", "--fuzz=0", "-R", "-p1", "-d", str(restored),
                            "-i", str(patch)], check=True, capture_output=True)
            for relative, digest in expected.items():
                if hashlib.sha256((restored / relative).read_bytes()).hexdigest() != digest:
                    raise SystemExit(f"{name}/{relative}: differs from published source plus reviewed patch")
        print(f"Verified {name} {entry['version']}: published source plus reviewed transport patch")


if __name__ == "__main__":
    verify()
