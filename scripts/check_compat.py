#!/usr/bin/env python3
"""Build/test patched payments + pinned zingolib with connector injection, without funds.
Downloads Cargo dependencies and public Sapling parameters on the first build.
"""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    subprocess.run([os.sys.executable, str(ROOT / 'vendor/verify_zingo.py')], check=True)
    compiler_manifest = ROOT / 'compat/protoc/Cargo.toml'
    protoc = subprocess.check_output(['cargo', 'run', '--quiet', '--locked', '--manifest-path', str(compiler_manifest)], text=True).strip()
    env = dict(os.environ, PROTOC=protoc)
    subprocess.run(['cargo', 'test', '--locked', '--manifest-path', str(ROOT / 'compat/zingolib/Cargo.toml')], env=env, check=True)

if __name__ == '__main__':
    main()
