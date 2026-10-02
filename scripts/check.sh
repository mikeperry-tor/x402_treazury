#!/bin/sh
# Run feature suites sequentially: their CLI tests share target/debug/treazure.
set -eu
with_zcash=false
case ${1:-} in
    '') ;;
    --zcash) with_zcash=true; shift ;;
    -h|--help) echo 'Usage: scripts/check.sh [--zcash]'; exit 0 ;;
    *) echo 'Usage: scripts/check.sh [--zcash]' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'Usage: scripts/check.sh [--zcash]' >&2
    exit 2
fi
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
cargo fmt --check
python3 vendor/verify.py
python3 vendor/verify_zingo.py
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
if [ "$with_zcash" = true ]; then
    scripts/zcash.sh test --all-targets -- --test-threads=1
    scripts/zcash.sh clippy --all-targets --features zcash-regtest -- -D warnings
    python3 scripts/check_compat.py
fi
