#!/bin/sh
# Build/test using Cargo-managed protoc; Zcash is enabled by default.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sh "$project_dir/scripts/check_toolchain.sh"
PROTOC=$(cargo run --quiet --locked --manifest-path "$project_dir/compat/protoc/Cargo.toml")
export PROTOC
command=${1:-build}
if [ "$#" -gt 0 ]; then shift; fi
exec cargo "$command" --locked --manifest-path "$project_dir/Cargo.toml" "$@"
