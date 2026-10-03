#!/bin/sh
# rustup reads rust-toolchain.toml; enforce it for Homebrew/system Rust too.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
expected=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$project_dir/rust-toolchain.toml")
[ -n "$expected" ] || { echo 'Missing pinned Rust toolchain' >&2; exit 1; }
for tool in rustc cargo; do
    actual=$("$tool" --version)
    case "$actual" in
        "$tool $expected "*) ;;
        *) echo "Toolchain mismatch: expected $tool $expected; found $actual. Select the pinned toolchain before building." >&2; exit 1 ;;
    esac
done
