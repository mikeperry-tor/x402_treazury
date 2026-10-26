# GNU Make 3.81+ and a POSIX shell. Run from the repository root (or make -C DIR).
.DEFAULT_GOAL := build
SHELL := /bin/sh
# Feature builds and CLI fixtures share target/debug/x402_treazury.
.NOTPARALLEL:

PYTHON ?= python3
PROFILE ?= release
TEST_ARGS ?= --test-threads=1
COMPLEXITY_BASE_REF ?= main

# Resolve protoc only when executing an application Cargo command. Keep failures
# in the assignment visible; an empty/failed helper must never use system protoc.
define application_cargo
set -eu; \
PROTOC=$$(cargo run --quiet --locked --manifest-path compat/protoc/Cargo.toml); \
export PROTOC; \
exec $(3) cargo $(1) --locked --manifest-path Cargo.toml $(2)
endef

# Make owns build profile selection. Do not allow extra flags to turn build-dev
# into a release build (or override PROFILE) while retaining its incremental mode.
define build_arguments
set -eu; set -- $(CARGO_ARGS); \
for arg do \
	case "$$arg" in \
		--) break ;; \
		--release|-r|--profile|--profile=*) \
			echo 'Select the build profile with PROFILE=NAME or make build-dev.' >&2; exit 2 ;; \
	esac; \
done;
endef

.PHONY: build build-dev test check cargo-check run clippy fmt fmt-check toolchain \
        check-static check-clippy check-compat check-default check-proving \
        check-lifecycle check-codec \
        coverage coverage-branch coverage-proving coverage-consensus help

build: toolchain
	@$(build_arguments) \
	export CARGO_INCREMENTAL=0; \
	$(call application_cargo,build,--profile '$(PROFILE)' "$$@")

build-dev: toolchain
	@$(build_arguments) \
	export CARGO_INCREMENTAL=1; \
	$(call application_cargo,build,--profile dev "$$@")

# CARGO_ARGS replaces the default selection, allowing focused --test/--lib runs.
test clippy: CARGO_ARGS ?= --all-targets

test: toolchain
	@$(call application_cargo,test,$(CARGO_ARGS) -- $(TEST_ARGS))

cargo-check: toolchain
	@$(call application_cargo,check,$(CARGO_ARGS))

run: toolchain
	@$(call application_cargo,run,$(CARGO_ARGS))

clippy: toolchain
	@$(call application_cargo,clippy,$(CARGO_ARGS) -- -D warnings)

fmt: toolchain
	cargo fmt

fmt-check: toolchain
	cargo fmt --check

toolchain:
	@set -eu; \
	expected=$$(sed -n 's/^channel = "\(.*\)"$$/\1/p' rust-toolchain.toml); \
	[ -n "$$expected" ] || { echo 'Missing pinned Rust toolchain' >&2; exit 1; }; \
	for tool in rustc cargo; do \
		actual=$$("$$tool" --version); \
		case "$$actual" in \
			"$$tool $$expected "*) ;; \
			*) echo "Toolchain mismatch: expected $$tool $$expected; found $$actual. Select the pinned toolchain before building." >&2; exit 1 ;; \
		esac; \
	done

# Each stage is independently callable for focused validation and continuation.
# Separate recursive calls preserve fail-fast behavior even with make -k.
# Fixed selections are independent of CARGO_ARGS and TEST_ARGS.
check:
	$(MAKE) check-static
	$(MAKE) check-clippy
	$(MAKE) check-compat
	$(MAKE) check-default
	$(MAKE) check-proving
	$(MAKE) check-lifecycle
	$(MAKE) check-codec

check-static: toolchain
	cargo fmt --check
	scripts/tests/coverage.sh
	$(PYTHON) -m unittest discover -s scripts/tests -p 'test_*.py'
	cargo test --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity
	$(PYTHON) scripts/complexity.py --base '$(COMPLEXITY_BASE_REF)'
	$(PYTHON) vendor/verify.py
	$(PYTHON) vendor/verify_zingo.py

check-clippy: toolchain
	@$(call application_cargo,check,--lib --bins --message-format=json,$(PYTHON) scripts/check_warnings.py)
	@$(call application_cargo,clippy,--all-targets --features zcash-regtest --message-format=json -- -D warnings,$(PYTHON) scripts/check_warnings.py)

check-compat: toolchain
	$(PYTHON) scripts/check_compat.py

check-default: toolchain
	@$(call application_cargo,test,-p pepper-sync --lib wallet::traits::diagnostic_tests::scan_merge_conflict_preserves_pool_context -- --exact)
	@$(call application_cargo,test,--all-targets -- --test-threads=1)

check-proving: toolchain
	@$(call application_cargo,test,--features zcash-testutils --lib treasury::send::proving_tests -- --test-threads=1)

check-lifecycle: toolchain
	@$(call application_cargo,test,--features zcash-testutils --lib treasury::lifecycle_tests -- --test-threads=1)

check-codec: toolchain
	@$(call application_cargo,test,--features zcash-testutils --lib synthetic_address_codec_is_scoped_to_its_test_client -- --test-threads=1)

# Complex report publication remains in an internal helper. Nightly branch
# coverage intentionally uses a different toolchain from the pinned application.
coverage:
	scripts/coverage.sh

coverage-branch:
	scripts/coverage.sh --branch

coverage-proving:
	scripts/coverage.sh --proving

coverage-consensus:
	@set -eu; test -n '$(TEST)' || { echo 'Usage: make coverage-consensus TEST=NAME' >&2; exit 2; }; \
	scripts/coverage.sh --consensus '$(TEST)'

help:
	@printf '%s\n' \
	  'make / make build       Release build, incremental disabled (PROFILE=NAME to change)' \
	  'make build-dev          Incremental development build' \
	  'make test               All default test targets, one test thread' \
	  'make check              All check stages in the order below' \
	  'make check-static       Format, tooling, complexity and provenance' \
	  'make check-clippy       Production compiler warnings and all-target zcash-regtest Clippy' \
	  'make check-compat       Combined dependency compatibility tests' \
	  'make check-default      SDK diagnostic and all default test targets' \
	  'make check-proving      Synthetic treasury proving tests' \
	  'make check-lifecycle    Synthetic treasury lifecycle tests' \
	  'make check-codec        Synthetic address-codec isolation test' \
	  'make cargo-check        Fast Cargo compilation check' \
	  'make run                Run the application or an example (CARGO_ARGS)' \
	  'make clippy             Clippy, warnings denied' \
	  'make fmt / fmt-check    Apply / check formatting' \
	  'make toolchain          Verify pinned Rust and Cargo' \
	  'make coverage           Default coverage report' \
	  'make coverage-branch    Nightly branch coverage' \
	  'make coverage-proving   Selected synthetic proving coverage' \
	  'make coverage-consensus TEST=NAME  One named consensus test' \
	  '' \
	  'CARGO_ARGS replaces target defaults; TEST_ARGS replaces test harness defaults.' \
	  'Example: make test CARGO_ARGS="--test payments" TEST_ARGS="case_name --exact --test-threads=1"' \
	  'See scripts/README.md for prerequisites, quoting and optional qualification.'
