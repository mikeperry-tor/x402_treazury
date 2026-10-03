#!/bin/sh
# Local coverage of the default application, including Zcash.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
branch=false
case ${1:-} in
    '') ;;
    --branch) branch=true; shift ;;
    -h|--help) echo 'Usage: scripts/coverage.sh [--branch] (branch requires nightly Rust)'; exit 0 ;;
    *) echo 'Usage: scripts/coverage.sh [--branch]' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'Usage: scripts/coverage.sh [--branch]' >&2
    exit 2
fi
report_dir=target/coverage
if [ "$branch" = true ]; then
    case $(rustc --version) in
        *nightly*) ;;
        *) echo 'Branch coverage requires nightly Rust. With rustup, run: RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh --branch' >&2; exit 2 ;;
    esac
    report_dir=target/coverage-branch
    # Separate profiles/artifacts also preserve the stable coverage run.
    CARGO_LLVM_COV_TARGET_DIR=$project_dir/target/llvm-cov-branch
    export CARGO_LLVM_COV_TARGET_DIR
    set -- --branch
fi
# Homebrew Rust does not provide rustup's llvm-tools-preview component.
# Explicit overrides win; other installations use cargo-llvm-cov discovery.
if rustc --version --verbose | grep -q '(Homebrew)' && command -v brew >/dev/null 2>&1; then
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
cargo llvm-cov --locked --all-targets --no-report "$@" -- --test-threads=1
mkdir -p "$report_dir"
# Application source only; test harnesses, examples and vendored code are not
# part of this denominator. Inline unit-test helpers may still be included.
exclude='/(tests|examples|vendor|compat)/|/src/discovery/tests\.rs$'
cargo llvm-cov report --ignore-filename-regex "$exclude" --html --output-dir "$report_dir"
cargo llvm-cov report --ignore-filename-regex "$exclude" --json --summary-only --output-path "$report_dir/summary.json"
cargo llvm-cov report --ignore-filename-regex "$exclude" > "$report_dir/summary.txt"
cat "$report_dir/summary.txt"
