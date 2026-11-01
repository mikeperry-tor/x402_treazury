#!/bin/sh
# Reproducible local coverage; no optional consensus/live tests run implicitly.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
usage() {
    echo 'Usage: scripts/coverage.sh [--condition | --condition-financial | --branch | --proving | --consensus TEST]'
}
mode=default
case ${1:-} in
    '') ;;
    --condition) mode=condition; shift ;;
    --condition-financial) mode=condition-financial; shift ;;
    --branch) mode=branch; shift ;;
    --proving) mode=proving; shift ;;
    --consensus)
        shift
        case ${1:-} in
            deposit_settlement_and_reorg_recovery|high_index_refunds_and_separate_shielding|expired_ambiguous_deposit_releases_only_after_chain_proof|indexer_non_inclusion_response|tor_consensus_lifecycle|recovery_cli_lifecycle)
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
        *) echo 'Branch coverage requires nightly Rust: RUSTUP_TOOLCHAIN=nightly make coverage-branch' >&2; exit 2 ;;
    esac
fi
# Analysis-only pin: resolve the installed toolchain, then verify its identity.
# Overrides must resolve to the same exact revision as the dated default.
case $mode in
    condition*)
        RUSTC=$(rustup which --toolchain "${CONDITION_TOOLCHAIN:-nightly-2026-10-31}" rustc)
        analysis_cargo=$(rustup which --toolchain "${CONDITION_TOOLCHAIN:-nightly-2026-10-31}" cargo)
        export RUSTC
        PATH=$(dirname "$analysis_cargo"):$PATH
        export PATH
        if ! "$RUSTC" -vV | grep -qx 'commit-hash: 92b5b8c87c61b20adc9001b41baf587634a74734'; then
            echo 'Condition analysis requires compiler 92b5b8c87c61b20adc9001b41baf587634a74734 (nightly 1.101.0).' >&2
            exit 2
        fi
        test "$(cargo llvm-cov --version)" = 'cargo-llvm-cov 0.9.1' || {
            echo 'Condition analysis requires cargo-llvm-cov 0.9.1.' >&2; exit 2;
        }
        if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
            echo 'Unset CARGO_ENCODED_RUSTFLAGS for pinned condition analysis.' >&2; exit 2
        fi
        RUSTFLAGS="${RUSTFLAGS:-} -Zcoverage-options=condition"
        export RUSTFLAGS
        analysis_sysroot=$("$RUSTC" --print sysroot)
        analysis_host=$("$RUSTC" -vV | sed -n 's/^host: //p')
        LLVM_COV=$analysis_sysroot/lib/rustlib/$analysis_host/bin/llvm-cov
        LLVM_PROFDATA=$analysis_sysroot/lib/rustlib/$analysis_host/bin/llvm-profdata
        export LLVM_COV LLVM_PROFDATA
        "$LLVM_COV" --version | grep -q 'LLVM version 23.1.4' || {
            echo 'Condition analysis requires matching LLVM 23.1.4 tools.' >&2; exit 2;
        }
        ;;
esac
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
exclude='/(tests|examples|tools|vendor)/|/src/discovery/tests\.rs$'
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
    echo 'profile=test'
    echo "RUSTC=${RUSTC:-rustc}"
    echo "RUSTFLAGS=${RUSTFLAGS:-}"
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
git ls-files -z |
    xargs -0 shasum -a 256 > "$run_dir/source-sha256.txt"
case $mode in
    condition*)
        shasum -a 256 "$RUSTC" "$analysis_cargo" "$(command -v cargo-llvm-cov)" "$LLVM_COV" "$LLVM_PROFDATA" > "$run_dir/tool-sha256.txt"
        ;;
esac
# Commands are fixed by this script; no credentials are accepted as arguments.
run() {
    printf '%s' 'command:' >> "$run_dir/commands.txt"
    printf ' %s' "$@" >> "$run_dir/commands.txt"
    printf '\n' >> "$run_dir/commands.txt"
    "$@"
}
PROTOC=$(run cargo run --quiet --locked --manifest-path tools/protoc/Cargo.toml --target-dir "$project_dir/target/coverage-protoc-$mode")
export PROTOC
case $mode in
    default|condition) set -- --all-targets ;;
    condition-financial) set -- ;;
    branch) set -- --all-targets --branch ;;
    proving) set -- --features zcash-testutils --lib treasury::send::proving_tests::funded_preparation_proves_and_restores_identical_pending_bytes ;;
    consensus-*) set -- --features zcash-regtest --lib "treasury::regtest::$consensus_test" ;;
esac
# Remove previous raw profiles before accumulating this run only.
run cargo llvm-cov clean --workspace
collect() {
    selection=$1
    shift
    # Every independently filtered selection must contain tests.
    run cargo llvm-cov test --locked "$@" --no-report -- --list > "$run_dir/tests-$selection.txt" 2>&1
    cat "$run_dir/tests-$selection.txt" >> "$run_dir/tests.txt"
    if ! grep -Eq '^[^ ]+: test$' "$run_dir/tests-$selection.txt"; then
        echo "No tests matched coverage selection $selection; refusing an empty collection." >&2
        exit 2
    fi
    threads=1
    case $mode:$selection in
        condition:condition|condition-financial:lifecycle) threads=8 ;;
    esac
    case $mode in
        consensus-*) set -- "$@" --no-report -- --ignored --exact --nocapture --test-threads=1 ;;
        proving) set -- "$@" --no-report -- --exact --nocapture --test-threads=1 ;;
        *) set -- "$@" --no-report -- --test-threads="$threads" ;;
    esac
    run cargo llvm-cov test --locked "$@" >> "$run_dir/test.log" 2>&1
}
if [ "$mode" = condition-financial ]; then
    # Same feature/profile, separate filters; retain profiles across all three.
    collect proving --features zcash-testutils --lib treasury::send::proving_tests
    collect lifecycle --features zcash-testutils --lib treasury::lifecycle_tests
    collect codec --features zcash-testutils --lib synthetic_address_codec_is_scoped_to_its_test_client
else
    collect "$mode" "$@"
fi
# Export all profiles collected from test and CLI executables. Process exits/panics
# need not flush profiles; test outcomes remain separate from coverage evidence.
# Preserve raw evidence before the next same-mode clean removes working profiles.
find "$CARGO_LLVM_COV_TARGET_DIR" -name '*.profraw' -type f -print > "$run_dir/working-profiles.txt"
while IFS= read -r profile; do
    saved=$run_dir/profiles/${profile#"$CARGO_LLVM_COV_TARGET_DIR"/}
    mkdir -p "$(dirname "$saved")"
    cp "$profile" "$saved"
    printf '%s\n' "$saved" >> "$run_dir/profiles.txt"
done < "$run_dir/working-profiles.txt"
run cargo llvm-cov report --json --output-path "$run_dir/full.json"
run cargo llvm-cov report --json --summary-only --output-path "$run_dir/unfiltered-summary.json"
# Wrapper defaults hide tests and *_tests.rs, including the operational runner.
# Keep application exports stable and expose a separate same-profile raw view.
case $mode in
    condition*) run cargo llvm-cov report --no-default-ignore-filename-regex --json --output-path "$run_dir/all-sources.json" ;;
esac
run cargo llvm-cov report --ignore-filename-regex "$exclude" --html --output-dir "$run_dir"
run cargo llvm-cov report --ignore-filename-regex "$exclude" --json --summary-only --output-path "$run_dir/summary.json"
run cargo llvm-cov report --ignore-filename-regex "$exclude" > "$run_dir/summary.txt"
# Reject source drift before publication; the unique run retains all evidence.
shasum -a 256 -c "$run_dir/source-sha256.txt" > "$run_dir/source-verification.txt"
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
