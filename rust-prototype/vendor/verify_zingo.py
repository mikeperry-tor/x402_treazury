#!/usr/bin/env python3
"""Verify the complete reviewed Zingo vendor snapshot, excluding generated params."""
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
for package in ("zingolib", "zingo-netutils"):
    for path in (root / package).rglob("*"):
        if not path.is_file() or path.suffix == ".params":
            continue
        if path.relative_to(root).as_posix() not in manifest["files"]:
            errors.append(str(path.relative_to(root)))
if errors:
    raise SystemExit("Zingo vendor verification failed: " + ", ".join(errors))
print(f"Verified {len(manifest['files'])} vendored files from {manifest['revision']} plus reviewed connector patch")
