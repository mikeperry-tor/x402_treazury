#!/bin/sh
# Exercise report publication/failure behavior without compiling or networking.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/bin" "$fixture/target/coverage"
cp "$project_dir/scripts/coverage.sh" "$fixture/scripts/"
echo fixture > "$fixture/Cargo.lock"
echo legacy > "$fixture/target/coverage/legacy"
cat > "$fixture/bin/rustc" <<'STUB'
#!/bin/sh
if [ "${CONDITION_FIXTURE:-}" = yes ]; then
    case "$*" in
        *'--print sysroot'*) echo "$FIXTURE_ROOT" ;;
        *-vV*) printf '%s\n' 'rustc 1.101.0-nightly fixture' 'commit-hash: 92b5b8c87c61b20adc9001b41baf587634a74734' 'host: fixture' ;;
        *) echo 'rustc 1.101.0-nightly fixture' ;;
    esac
else
    echo 'rustc 1.99.0 fixture'
fi
STUB
cat > "$fixture/bin/git" <<'STUB'
#!/bin/sh
case $1 in
    rev-parse) echo fixture-revision ;;
    ls-files) printf 'Cargo.lock\0' ;;
esac
STUB
cat > "$fixture/bin/cargo" <<'STUB'
#!/bin/sh
case "$*" in
    'llvm-cov --version') echo 'cargo-llvm-cov 0.9.1'; exit 0 ;;
    *--version*) echo fixture-cargo; exit 0 ;;
    'llvm-cov clean '*)
        # Cargo refuses existing target roots it did not initialize.
        if [ -d "$CARGO_LLVM_COV_TARGET_DIR" ] && [ ! -f "$CARGO_LLVM_COV_TARGET_DIR/CACHEDIR.TAG" ]; then
            echo 'untagged target root' >&2; exit 41
        fi
        rm -f "$CARGO_LLVM_COV_TARGET_DIR/child.profraw"; exit 0 ;;
    'run '*)
        while [ "$#" -gt 0 ]; do
            if [ "$1" = --target-dir ]; then shift; mkdir -p "$1"; break; fi
            shift
        done
        echo /usr/bin/true; exit 0 ;;
    *--no-report*)
        case "$*" in
            *'--lib treasury::'*|*'--lib synthetic_address_codec_is_scoped_to_its_test_client'*)
                if [ "$2" != test ]; then echo 'unrecognized subcommand filter' >&2; exit 40; fi ;;
        esac
        mkdir -p "$CARGO_LLVM_COV_TARGET_DIR"
        touch "$CARGO_LLVM_COV_TARGET_DIR/CACHEDIR.TAG"
        case "$*" in
            *--list*)
                case "$*" in
                    *treasury::lifecycle_tests*) if [ "${EMPTY_LIFECYCLE:-}" = yes ]; then exit 0; fi ;;
                esac
                if [ "${EMPTY_COVERAGE_SELECTION:-}" != yes ]; then echo 'fixture: test'; fi
                exit 0 ;;
        esac
        if [ "${FAIL_COVERAGE_TEST:-}" = yes ]; then exit 42; fi
        case "$*" in
            *treasury::lifecycle_tests*)
                if [ "${FAIL_LIFECYCLE:-}" = yes ]; then exit 43; fi ;;
        esac
        printf '%s\n' "$*" >> "$FIXTURE_ROOT/executions.txt"
        mkdir -p "$CARGO_LLVM_COV_TARGET_DIR"
        echo profile > "$CARGO_LLVM_COV_TARGET_DIR/child.profraw"
        echo 'test result: ok. 1 passed; 0 failed; 0 ignored'
        exit 0 ;;
esac
case "$*" in
    'llvm-cov report '*)
        if [ "${FAIL_EXPORT:-}" = yes ]; then exit 44; fi
        case "$*" in
            *--lcov*) if [ "${FAIL_LCOV_EXPORT:-}" = yes ]; then exit 45; fi ;;
        esac
        if [ "${SOURCE_DRIFT:-}" = yes ]; then echo changed >> Cargo.lock; fi ;;
esac
while [ "$#" -gt 0 ]; do
    case $1 in
        --output-path) shift; echo '{}' > "$1"; exit 0 ;;
        --output-dir) shift; mkdir -p "$1/html"; echo report > "$1/html/index.html"; exit 0 ;;
    esac
    shift
done
echo 'TOTAL fixture summary'
STUB
printf '%s\n' '#!/bin/sh' 'echo fixture-wrapper' > "$fixture/bin/cargo-llvm-cov"
cat > "$fixture/bin/rustup" <<'STUB'
#!/bin/sh
for arg do last=$arg; done
printf '%s/bin/%s\n' "$FIXTURE_ROOT" "$last"
STUB
mkdir -p "$fixture/lib/rustlib/fixture/bin"
for tool in llvm-cov llvm-profdata; do
    printf '%s\n' '#!/bin/sh' "echo 'LLVM version 23.1.4'" > "$fixture/lib/rustlib/fixture/bin/$tool"
    chmod +x "$fixture/lib/rustlib/fixture/bin/$tool"
done
FIXTURE_ROOT=$fixture
export FIXTURE_ROOT
chmod +x "$fixture/bin/"*
export PATH="$fixture/bin:$PATH"
# Fake tools are chosen only in this temporary repository.
cd "$fixture"
scripts/coverage.sh > first.log
first=$(readlink target/coverage)
test -f "$first/previous-report/legacy"
test "$(cat "$first/status")" = complete
test -s "$first/profiles.txt"
test -s "$first/lcov.info"
grep -q fixture-revision "$first/provenance.txt"
# The application denominator excludes operational tools in their relocated homes.
exclude=$(sed -n 's/^ignore_filename_regex=//p' "$first/provenance.txt")
grep -Fq -- "--ignore-filename-regex $exclude --lcov --output-path $first/lcov.info" "$first/commands.txt"
for source in /repo/tools/providers/snapshot_spec.rs /repo/tools/diagnostics/quote_near.rs /repo/tests/live/runner/driver.rs; do
    printf '%s\n' "$source" | grep -Eq "$exclude"
done
if printf '%s\n' /repo/src/payment.rs | grep -Eq "$exclude"; then
    echo 'application source incorrectly excluded from coverage' >&2; exit 1
fi
if FAIL_COVERAGE_TEST=yes scripts/coverage.sh > failed.log 2>&1; then
    echo 'failed test run incorrectly succeeded' >&2; exit 1
fi
test "$(readlink target/coverage)" = "$first"
if FAIL_LCOV_EXPORT=yes scripts/coverage.sh > lcov-failure.log 2>&1; then
    echo 'failed LCOV export incorrectly succeeded' >&2; exit 1
fi
test "$(readlink target/coverage)" = "$first"
test -s "$first/lcov.info"
if EMPTY_COVERAGE_SELECTION=yes scripts/coverage.sh > empty.log 2>&1; then exit 1; fi
test "$(readlink target/coverage)" = "$first"
scripts/coverage.sh > second.log
second=$(readlink target/coverage)
test "$first" != "$second"
test -f "$first/html/index.html"
while IFS= read -r profile; do test -s "$profile"; done < "$first/profiles.txt"
test -f "$second/html/index.html"
if scripts/coverage.sh --consensus all > invalid.log 2>&1; then exit 1; fi
if scripts/coverage.sh --branch > nightly.log 2>&1; then exit 1; fi
executions_before_financial=$(wc -l < executions.txt | tr -d ' ')
CONDITION_FIXTURE=yes scripts/coverage.sh --condition-financial > financial.log
financial=$(readlink target/coverage-condition-financial)
test "$(wc -l < executions.txt | tr -d ' ')" = "$((executions_before_financial + 3))"
for filter in treasury::send::proving_tests treasury::lifecycle_tests synthetic_address_codec_is_scoped_to_its_test_client; do
    test "$(grep -c "$filter" executions.txt)" = 1
    test "$(grep -c "command: cargo llvm-cov test .* $filter " "$financial/commands.txt")" = 2
done
for selection in proving lifecycle codec; do
    test -s "$financial/tests-$selection.txt"
done
test "$(grep -c 'command: cargo llvm-cov clean' "$financial/commands.txt")" = 1
test -f "$financial/all-sources.json"
test "$(grep -c -- '--no-default-ignore-filename-regex' "$financial/commands.txt")" = 1
grep -- '--no-default-ignore-filename-regex' "$financial/commands.txt" | grep -q '/all-sources.json$'
if grep -q -- '--no-default-ignore-filename-regex' "$first/commands.txt"; then exit 1; fi
for filter in treasury::send::proving_tests synthetic_address_codec_is_scoped_to_its_test_client; do
    grep "$filter" executions.txt | grep -q -- '--test-threads=1'
done
grep treasury::lifecycle_tests executions.txt | grep -q -- '--test-threads=8'
head -n 2 executions.txt | grep -q -- '--test-threads=1'
if grep -q -- '--branch' "$financial/commands.txt"; then exit 1; fi
grep -q -- '-Zcoverage-options=condition' "$financial/provenance.txt"
if CONDITION_FIXTURE=yes FAIL_LIFECYCLE=yes scripts/coverage.sh --condition-financial > late-failure.log 2>&1; then exit 1; fi
test "$(readlink target/coverage-condition-financial)" = "$financial"
if CONDITION_FIXTURE=yes FAIL_EXPORT=yes scripts/coverage.sh --condition-financial > export-failure.log 2>&1; then exit 1; fi
test "$(readlink target/coverage-condition-financial)" = "$financial"
if CONDITION_FIXTURE=yes EMPTY_LIFECYCLE=yes scripts/coverage.sh --condition-financial > empty-lifecycle.log 2>&1; then exit 1; fi
test "$(readlink target/coverage-condition-financial)" = "$financial"
if CONDITION_FIXTURE=yes SOURCE_DRIFT=yes scripts/coverage.sh --condition-financial > source-drift.log 2>&1; then exit 1; fi
test "$(readlink target/coverage-condition-financial)" = "$financial"
CONDITION_FIXTURE=yes scripts/coverage.sh --condition > condition.log
condition=$(readlink target/coverage-condition)
grep -q -- '--all-targets' "$condition/commands.txt"
test -s "$condition/profiles.txt"
test -f "$condition/all-sources.json"
grep -- '--all-targets' "$condition/commands.txt" | grep -q -- '--test-threads=8'
echo 'Coverage publication, failed-run preservation, history and mode guards passed.'
