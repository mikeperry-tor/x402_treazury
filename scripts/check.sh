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
cargo test --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity
python3 scripts/complexity.py --base "${COMPLEXITY_BASE_REF:-main}"
python3 vendor/verify.py
python3 vendor/verify_zingo.py
if [ "$with_zcash" = true ]; then
    scripts/zcash.sh test --all-targets -- --test-threads=1
    scripts/zcash.sh test --features zcash-testutils --lib treasury::refunds::tests -- --test-threads=1
    scripts/zcash.sh test --features zcash-testutils --lib treasury::lifecycle_tests -- --test-threads=1
    scripts/zcash.sh test --features zcash-testutils --lib synthetic_address_codec_is_scoped_to_its_test_client -- --test-threads=1
    scripts/zcash.sh clippy --all-targets --features zcash-regtest -- -D warnings
    python3 scripts/check_compat.py
else
    cargo test --locked --no-default-features --all-targets
    cargo clippy --locked --no-default-features --all-targets -- -D warnings
fi
