#!/bin/sh
# Keep the workaround local to development tests. Ordinary run/build and release
# qualification must retain their original executable identity.
set -eu

platform=$(uname -s)
if [ "$1" = --cargo ]; then
    shift
    if [ "$platform" = Darwin ]; then
        cargo=$1
        shift
        directory=$(CDPATH= cd "$(dirname "$0")" && pwd)
        exec "$cargo" --config "$directory/test-runner.toml" "$@"
    fi
    exec "$@"
fi

if [ "$platform" != Darwin ]; then
    exec "$@"
fi

# Work around macOS pre-main _dyld_start hangs described in
# https://github.com/openai/codex/issues/23802 . This can also happen in normal
# terminal windows, not only Codex. The precise OS cause remains unconfirmed.
original=$1
shift
directory=$(mktemp -d "${TMPDIR:-/tmp}/x402-test.XXXXXX")
copy="$directory/$(basename "$original")"
cp -X "$original" "$copy"
codesign --force --sign - "$copy" >&2
# Retain the private copy: current_exe() helpers may execute it again. Never
# rewrite Cargo's artifact or remove a copy while its children can still use it.
printf 'Running macOS test copy: %s\n' "$copy" >&2
exec "$copy" "$@"
