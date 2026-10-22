#!/usr/bin/env python3
"""Verify the retained Pepper snapshot, patches and copied-code license."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
manifest = json.loads((root / "zingo-sources.json").read_text())
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
    raise SystemExit("Zingo vendor verification failed: " + ", ".join(errors))
print(f"Verified {len(manifest['files'])} snapshot and patch files from {manifest['revision']}")
