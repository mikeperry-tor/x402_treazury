#!/bin/sh
# Build/test the optional embedded wallet using Cargo-managed protoc.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PROTOC=$(cargo run --quiet --locked --manifest-path "$project_dir/compat/protoc/Cargo.toml")
export PROTOC
command=${1:-build}
if [ "$#" -gt 0 ]; then shift; fi
exec cargo "$command" --locked --manifest-path "$project_dir/Cargo.toml" --features zcash "$@"
