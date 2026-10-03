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
echo 'rustc 1.99.0 fixture'
STUB
cat > "$fixture/bin/git" <<'STUB'
#!/bin/sh
case $1 in rev-parse) echo fixture-revision ;; esac
STUB
cat > "$fixture/bin/cargo" <<'STUB'
#!/bin/sh
case "$*" in
    *--version*) echo fixture-cargo; exit 0 ;;
    'run '*) echo /usr/bin/true; exit 0 ;;
    *--no-report*)
        case "$*" in
            *--list*)
                if [ "${EMPTY_COVERAGE_SELECTION:-}" != yes ]; then echo 'fixture: test'; fi
                exit 0 ;;
        esac
        if [ "${FAIL_COVERAGE_TEST:-}" = yes ]; then exit 42; fi
        mkdir -p "$CARGO_LLVM_COV_TARGET_DIR"
        echo profile > "$CARGO_LLVM_COV_TARGET_DIR/child.profraw"
        echo 'test result: ok. 1 passed; 0 failed; 0 ignored'
        exit 0 ;;
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
chmod +x "$fixture/bin/"*
export PATH="$fixture/bin:$PATH"
# Fake tools are chosen only in this temporary repository.
cd "$fixture"
scripts/coverage.sh > first.log
first=$(readlink target/coverage)
test -f "$first/previous-report/legacy"
test "$(cat "$first/status")" = complete
test -s "$first/profiles.txt"
grep -q fixture-revision "$first/provenance.txt"
if FAIL_COVERAGE_TEST=yes scripts/coverage.sh > failed.log 2>&1; then
    echo 'failed test run incorrectly succeeded' >&2; exit 1
fi
test "$(readlink target/coverage)" = "$first"
if EMPTY_COVERAGE_SELECTION=yes scripts/coverage.sh > empty.log 2>&1; then exit 1; fi
test "$(readlink target/coverage)" = "$first"
scripts/coverage.sh > second.log
second=$(readlink target/coverage)
test "$first" != "$second"
test -f "$first/html/index.html"
test -f "$second/html/index.html"
if scripts/coverage.sh --consensus all > invalid.log 2>&1; then exit 1; fi
if scripts/coverage.sh --branch > nightly.log 2>&1; then exit 1; fi
echo 'Coverage publication, failed-run preservation, history and mode guards passed.'
