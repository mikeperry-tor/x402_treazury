"""Verify the vendored sources differ only in the intended SHA-3 requirement."""
from pathlib import Path
import hashlib
import json

root = Path(__file__).resolve().parent
for name, provenance in json.loads((root / 'provenance.json').read_text()).items():
    directory = root / name
    expected = provenance['original_files']
    actual = {str(p.relative_to(directory)) for p in directory.rglob('*') if p.is_file()}
    if actual != set(expected):
        raise SystemExit(f'{name}: unexpected or missing files: {actual ^ set(expected)}')
    for relative, digest in expected.items():
        data = (directory / relative).read_bytes()
        if relative == 'Cargo.toml':
            before = b'[dependencies.sha3]\nversion = "=0.10.9"'
            if data.count(before) != 1:
                raise SystemExit(f'{name}: expected exactly one SHA-3 patch')
            data = data.replace(before, b'[dependencies.sha3]\nversion = "0.11.0"')
        if hashlib.sha256(data).hexdigest() != digest:
            raise SystemExit(f'{name}/{relative}: differs from recorded upstream bytes')
    print(f'{name} {provenance["version"]}: only the intended SHA-3 requirement changed')
