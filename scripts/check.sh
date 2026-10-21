#!/bin/sh
# Validate the application, opt-in treasury tests, and compatibility graph.
set -eu
case ${1:-} in
    '') ;;
    -h|--help) echo 'Usage: scripts/check.sh'; exit 0 ;;
    *) echo 'Usage: scripts/check.sh' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'Usage: scripts/check.sh' >&2
    exit 2
fi
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
sh scripts/check_toolchain.sh
cargo fmt --check
scripts/tests/coverage.sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
cargo test --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity
python3 scripts/complexity.py --base "${COMPLEXITY_BASE_REF:-main}"
python3 vendor/verify.py
python3 vendor/verify_zingo.py
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh test --features zcash-testutils --lib treasury::send::proving_tests -- --test-threads=1
scripts/zcash.sh test --features zcash-testutils --lib treasury::lifecycle_tests -- --test-threads=1
scripts/zcash.sh test --features zcash-testutils --lib synthetic_address_codec_is_scoped_to_its_test_client -- --test-threads=1
scripts/zcash.sh clippy --all-targets --features zcash-regtest -- -D warnings
python3 scripts/check_compat.py
