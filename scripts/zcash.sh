#!/bin/sh
# Build/test using Cargo-managed protoc; Zcash is enabled by default.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
command=${1:-build}
if [ "$#" -gt 0 ]; then shift; fi
developer=false
explicit_profile=false
build_help=false
# Rotate the original arguments to preserve quoting while removing our flag.
remaining=$#
while [ "$remaining" -gt 0 ]; do
    arg=$1
    shift
    remaining=$((remaining - 1))
    case "$arg" in
        --)
            set -- "$@" "$arg"
            while [ "$remaining" -gt 0 ]; do
                arg=$1; shift
                set -- "$@" "$arg"
                remaining=$((remaining - 1))
            done
            break ;;
        --developer) developer=true ;;
        --release|-r|--profile|--profile=*)
            explicit_profile=true
            set -- "$@" "$arg" ;;
        --help|-h) build_help=true; set -- "$@" "$arg" ;;
        *) set -- "$@" "$arg" ;;
    esac
done
if [ "$developer" = true ] && [ "$command" != build ]; then
    echo '--developer is supported only by scripts/zcash.sh build' >&2
    exit 2
fi
if [ "$command" = build ]; then
    if [ "$developer" = true ]; then
        if [ "$explicit_profile" = true ]; then
            echo '--developer cannot be combined with --release or --profile' >&2
            exit 2
        fi
        CARGO_INCREMENTAL=1
    else
        CARGO_INCREMENTAL=0
        if [ "$explicit_profile" = false ]; then set -- --release "$@"; fi
    fi
    export CARGO_INCREMENTAL
    if [ "$build_help" = true ]; then
        echo 'Wrapper default: release build, incremental disabled. --developer selects an incremental development build.' >&2
    fi
fi
sh "$project_dir/scripts/check_toolchain.sh"
PROTOC=$(cargo run --quiet --locked --manifest-path "$project_dir/compat/protoc/Cargo.toml")
export PROTOC
exec cargo "$command" --locked --manifest-path "$project_dir/Cargo.toml" "$@"
