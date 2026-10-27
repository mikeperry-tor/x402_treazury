#!/usr/bin/env python3
"""Verify Pepper's snapshot and restore its exact upstream tree in temporary storage."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parent
manifest = json.loads((root / "pepper-sync-sources.json").read_text())
errors = []
for name, expected in manifest["files"].items():
    path = root / name
    if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        errors.append(name)
for path in (root / "pepper-sync").rglob("*"):
    if not path.is_file():
        continue
    if path.relative_to(root).as_posix() not in manifest["files"]:
        errors.append(str(path.relative_to(root)))
if errors:
    raise SystemExit("Pepper vendor verification failed: " + ", ".join(errors))

source = root / "pepper-sync"
expected = manifest["original_files"]
actual = {path.relative_to(source).as_posix() for path in source.rglob("*") if path.is_file()}
if actual != set(expected):
    raise SystemExit(f"Pepper upstream inventory differs: {actual ^ set(expected)}")
with tempfile.TemporaryDirectory() as directory:
    restored = Path(directory) / "pepper-sync"
    shutil.copytree(source, restored)
    for name in reversed(manifest["patches"]):
        subprocess.run(["patch", "--batch", "--fuzz=0", "-R", "-p2", "-d", str(restored),
                        "-i", str(root / name)], check=True, capture_output=True)
    actual = {path.relative_to(restored).as_posix() for path in restored.rglob("*") if path.is_file()}
    if actual != set(expected):
        raise SystemExit(f"Restored Pepper inventory differs: {actual ^ set(expected)}")
    for name, digest in expected.items():
        if hashlib.sha256((restored / name).read_bytes()).hexdigest() != digest:
            raise SystemExit(f"pepper-sync/{name}: differs from upstream plus declared patches")
print(f"Verified Pepper: {len(expected)} upstream files plus declared patches at {manifest['revision']}")
