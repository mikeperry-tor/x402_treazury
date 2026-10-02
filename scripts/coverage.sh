#!/bin/sh
# Local stable-Rust coverage of the default application, including Zcash.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ "$#" -ne 0 ]; then
    echo 'Usage: scripts/coverage.sh (default features, all targets)' >&2
    exit 2
fi
# Homebrew Rust does not provide rustup's llvm-tools-preview component.
# Explicit overrides win; other installations use cargo-llvm-cov discovery.
if command -v brew >/dev/null 2>&1; then
    llvm_prefix=$(brew --prefix llvm 2>/dev/null || true)
    if [ -x "$llvm_prefix/bin/llvm-cov" ]; then
        LLVM_COV=${LLVM_COV:-$llvm_prefix/bin/llvm-cov}
        LLVM_PROFDATA=${LLVM_PROFDATA:-$llvm_prefix/bin/llvm-profdata}
        export LLVM_COV LLVM_PROFDATA
    fi
fi
PROTOC=$(cargo run --quiet --locked --manifest-path compat/protoc/Cargo.toml)
export PROTOC
# Keep reports and profiles in ignored target output. Never enable proving or
# Docker features implicitly. Child CLI tests forward only LLVM_PROFILE_FILE.
cargo llvm-cov --locked --all-targets --no-report -- --test-threads=1
mkdir -p target/coverage
# Application source only; test harnesses, examples and vendored code are not
# part of this denominator. Inline unit-test helpers may still be included.
exclude='/(tests|examples|vendor|compat)/|/src/discovery/tests\.rs$'
cargo llvm-cov report --ignore-filename-regex "$exclude" --html --output-dir target/coverage
cargo llvm-cov report --ignore-filename-regex "$exclude" --json --summary-only --output-path target/coverage/summary.json
cargo llvm-cov report --ignore-filename-regex "$exclude" > target/coverage/summary.txt
cat target/coverage/summary.txt
