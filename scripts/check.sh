#!/bin/sh
# Validate the default Zcash build. Reduced-build checks are explicit opt-in only.
set -eu
with_zcash=true
case ${1:-} in
    '') ;;
    --zcash) with_zcash=true; shift ;;
    --no-default-features) with_zcash=false; shift ;;
    -h|--help) echo 'Usage: scripts/check.sh [--no-default-features]'; exit 0 ;;
    *) echo 'Usage: scripts/check.sh [--no-default-features]' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'Usage: scripts/check.sh [--no-default-features]' >&2
    exit 2
fi
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
sh scripts/check_toolchain.sh
cargo fmt --check
scripts/tests/coverage.sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
python3 vendor/verify.py
python3 vendor/verify_zingo.py
if [ "$with_zcash" = true ]; then
    scripts/zcash.sh test --all-targets -- --test-threads=1
    scripts/zcash.sh clippy --all-targets --features zcash-regtest -- -D warnings
    python3 scripts/check_compat.py
else
    cargo test --locked --no-default-features --all-targets
    cargo clippy --locked --no-default-features --all-targets -- -D warnings
fi
