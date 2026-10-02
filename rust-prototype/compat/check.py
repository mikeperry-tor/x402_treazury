#!/usr/bin/env python3
"""Build/test patched payments + the exact reference zingolib tag, without funds.
Downloads Cargo dependencies and public Sapling parameters on the first build.
"""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = 'c6381534f802b1022041beda4b01c106ad132329'

def main():
    checkout = ROOT / 'reference_repos/zingolib'
    actual = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if actual != EXPECTED:
        raise SystemExit(f'Expected zingolib_v6.0.0 ({EXPECTED}), got {actual}')
    if subprocess.check_output(['git', '-C', str(checkout), 'diff', 'HEAD', '--'], text=True):
        raise SystemExit('The reference checkout has tracked modifications; use an unmodified tag.')
    compiler_manifest = ROOT / 'rust-prototype/compat/protoc/Cargo.toml'
    protoc = subprocess.check_output(['cargo', 'run', '--quiet', '--locked', '--manifest-path', str(compiler_manifest)], text=True).strip()
    env = dict(os.environ, PROTOC=protoc)
    subprocess.run(['cargo', 'test', '--locked', '--manifest-path', str(ROOT / 'rust-prototype/compat/zingolib/Cargo.toml')], env=env, check=True)

if __name__ == '__main__':
    main()
