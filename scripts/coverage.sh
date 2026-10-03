#!/bin/sh
# Reproducible local coverage; no optional consensus/live tests run implicitly.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
usage() {
    echo 'Usage: scripts/coverage.sh [--branch | --proving | --consensus TEST]'
}
mode=default
case ${1:-} in
    '') ;;
    --branch) mode=branch; shift ;;
    --proving) mode=proving; shift ;;
    --consensus)
        shift
        case ${1:-} in
            deposit_settlement_and_reorg_recovery|high_index_refunds_and_separate_shielding|expired_ambiguous_deposit_releases_only_after_chain_proof|indexer_non_inclusion_response|tor_consensus_lifecycle)
                mode=consensus-$1; consensus_test=$1; shift ;;
            *) usage >&2; echo 'Select one named treasury consensus test; broad filters are forbidden.' >&2; exit 2 ;;
        esac ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then usage >&2; exit 2; fi
if [ "$mode" = branch ]; then
    case $(rustc --version) in
        *nightly*) ;;
        *) echo 'Branch coverage requires nightly Rust: RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh --branch' >&2; exit 2 ;;
    esac
fi
# Respect explicit tools. A rustup nightly must use its own matching LLVM tools.
if rustc --version --verbose | grep -q '(Homebrew)' && command -v brew >/dev/null 2>&1; then
    llvm_prefix=$(brew --prefix llvm 2>/dev/null || true)
    if [ -x "$llvm_prefix/bin/llvm-cov" ]; then
        LLVM_COV=${LLVM_COV:-$llvm_prefix/bin/llvm-cov}
        LLVM_PROFDATA=${LLVM_PROFDATA:-$llvm_prefix/bin/llvm-profdata}
        export LLVM_COV LLVM_PROFDATA
    fi
fi
# A mode owns its profiles; never mix toolchains/features or inherit a shared target.
CARGO_LLVM_COV_TARGET_DIR=$project_dir/target/llvm-cov-$mode
export CARGO_LLVM_COV_TARGET_DIR
mkdir -p target/coverage-runs
run_dir=$(mktemp -d "$project_dir/target/coverage-runs/$mode-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")
exclude='/(tests|examples|vendor|compat)/|/src/discovery/tests\.rs$'
echo "Coverage run: $run_dir"
echo running > "$run_dir/status"
finish() {
    result=$?
    if [ "$result" -ne 0 ]; then
        echo "failed (exit $result)" > "$run_dir/status"
        echo "Coverage incomplete; previous report preserved. See $run_dir" >&2
    fi
}
trap finish EXIT
{
    echo "mode=$mode"
    echo "source_revision=$(git rev-parse HEAD)"
    echo "lockfile_sha256=$(shasum -a 256 Cargo.lock | cut -d ' ' -f 1)"
    echo "ignore_filename_regex=$exclude"
    echo "target_dir=$CARGO_LLVM_COV_TARGET_DIR"
    echo '--- source status (including untracked paths; no file contents) ---'
    git status --porcelain
    echo '--- compiler ---'
    rustc -vV
    cargo --version
    cargo llvm-cov --version
    echo '--- LLVM tool overrides (unset means cargo-llvm-cov discovery) ---'
    echo "LLVM_COV=${LLVM_COV:-unset}"
    echo "LLVM_PROFDATA=${LLVM_PROFDATA:-unset}"
    if [ -n "${LLVM_COV:-}" ]; then "$LLVM_COV" --version; fi
    if [ -n "${LLVM_PROFDATA:-}" ]; then "$LLVM_PROFDATA" --version; fi
    echo 'Inline unit-test helpers may contribute to the application denominator.'
    echo 'CLI/crash child results are asserted by parents; see test.log and profiles.txt.'
    echo 'A missing crash-child profile does not mean the parent recovery assertion did not run.'
    echo 'Optional proving/consensus modules are absent unless selected; live tests are not selected.'
} > "$run_dir/provenance.txt"
# Commands are fixed by this script; no credentials are accepted as arguments.
run() {
    printf '%s' 'command:' >> "$run_dir/commands.txt"
    printf ' %s' "$@" >> "$run_dir/commands.txt"
    printf '\n' >> "$run_dir/commands.txt"
    "$@"
}
PROTOC=$(run cargo run --quiet --locked --manifest-path compat/protoc/Cargo.toml)
export PROTOC
case $mode in
    default) set -- --all-targets ;;
    branch) set -- --all-targets --branch ;;
    proving) set -- --features zcash-testutils --lib treasury::send::proving_tests::funded_preparation_proves_and_restores_identical_pending_bytes ;;
    consensus-*) set -- --features zcash-regtest --lib "treasury::regtest::$consensus_test" ;;
esac
# Remove previous raw profiles before accumulating this run only.
run cargo llvm-cov clean --workspace
# Listing records selected and ignored cases without running ignored helpers.
run cargo llvm-cov --locked "$@" --no-report -- --list > "$run_dir/tests.txt" 2>&1
if ! grep -Eq '^[^ ]+: test$' "$run_dir/tests.txt"; then
    echo 'No tests matched the coverage selection; refusing an empty qualification.' >&2
    exit 2
fi
case $mode in
    consensus-*) set -- "$@" --no-report -- --ignored --exact --nocapture --test-threads=1 ;;
    proving) set -- "$@" --no-report -- --exact --nocapture --test-threads=1 ;;
    *) set -- "$@" --no-report -- --test-threads=1 ;;
esac
run cargo llvm-cov --locked "$@" > "$run_dir/test.log" 2>&1
# Export all profiles collected from test and CLI executables. Process exits/panics
# need not flush profiles; test outcomes remain separate from coverage evidence.
find "$CARGO_LLVM_COV_TARGET_DIR" -name '*.profraw' -type f -print > "$run_dir/profiles.txt"
run cargo llvm-cov report --json --output-path "$run_dir/full.json"
run cargo llvm-cov report --json --summary-only --output-path "$run_dir/unfiltered-summary.json"
run cargo llvm-cov report --ignore-filename-regex "$exclude" --html --output-dir "$run_dir"
run cargo llvm-cov report --ignore-filename-regex "$exclude" --json --summary-only --output-path "$run_dir/summary.json"
run cargo llvm-cov report --ignore-filename-regex "$exclude" > "$run_dir/summary.txt"
echo complete > "$run_dir/status"
# Preserve the legacy directory and every older successful run before publishing.
report_dir=target/coverage
if [ "$mode" != default ]; then report_dir=target/coverage-$mode; fi
if [ -e "$report_dir" ] && [ ! -L "$report_dir" ]; then
    mv "$report_dir" "$run_dir/previous-report"
fi
ln -sfn "$run_dir" "$report_dir"
cat "$run_dir/summary.txt"
echo "Report: $report_dir/html/index.html"
