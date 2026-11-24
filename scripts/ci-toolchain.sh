#!/bin/sh
set -eu

version=$(make --no-print-directory toolchain-version)
rustup toolchain install "$version" --profile minimal --component rustfmt --component clippy "$@"
RUSTUP_TOOLCHAIN=$version make toolchain
printf 'RUSTUP_TOOLCHAIN=%s\n' "$version" >> "$GITHUB_ENV"
