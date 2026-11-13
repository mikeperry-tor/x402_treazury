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
# The maintained inventory above may include files created by declared patches.
# Reversal must still restore the exact upstream inventory and bytes below.
with tempfile.TemporaryDirectory() as directory:
    restored = Path(directory) / "pepper-sync"
    shutil.copytree(source, restored)
    for name in reversed(manifest["patches"]):
        try:
            subprocess.run(["patch", "--binary", "--batch", "--fuzz=0", "-R", "-p2"],
                           cwd=restored, input=(root / name).read_bytes(),
                           check=True, capture_output=True)
        except subprocess.CalledProcessError as error:
            raise SystemExit(
                f"Pepper: cannot reverse {name} (exit {error.returncode})\n"
                f"stdout:\n{error.stdout.decode('utf-8', errors='replace')}"
                f"stderr:\n{error.stderr.decode('utf-8', errors='replace')}"
            ) from None
    actual = {path.relative_to(restored).as_posix() for path in restored.rglob("*") if path.is_file()}
    if actual != set(expected):
        raise SystemExit(f"Restored Pepper inventory differs: {actual ^ set(expected)}")
    for name, digest in expected.items():
        restored_bytes = (restored / name).read_bytes()
        actual_digest = hashlib.sha256(restored_bytes).hexdigest()
        if actual_digest != digest:
            crlf_match = hashlib.sha256(restored_bytes.replace(b"\r\n", b"\n")).hexdigest() == digest
            raise SystemExit(
                f"pepper-sync/{name}: differs from upstream plus declared patches; "
                f"expected_sha256={digest} actual_sha256={actual_digest} "
                f"crlf_to_lf_matches_expected={crlf_match} (diagnostic only)"
            )
print(f"Verified Pepper: {len(expected)} upstream files plus declared patches at {manifest['revision']}")
