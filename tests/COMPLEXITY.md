# Rust complexity audit

Use `python3 scripts/complexity.py` from the repository root. This is an optional,
local, reporting-only development tool. There are no numerical complexity gates
and no application/runtime dependencies. First use downloads development crates;
subsequent runs can use `CARGO_NET_OFFLINE=true`. No source is sent to an external
analysis service. The report orchestrator requires Python 3.11+.

The independent helper in `tools/complexity/` pins Mozilla's
[rust-code-analysis](https://github.com/mozilla/rust-code-analysis) to **0.0.25**.
Its separate committed lockfile is required: fresh dependency resolution can select
Tree-sitter 0.27 through tree-sitter-rust 0.20.3, incompatible with the analyzer's
0.20.9 API. The lockfile unifies that dependency at 0.20.9 without patching upstream
source or changing the application's lockfile. Requalify the syntax probes and
baseline when updating the analyzer or its grammar dependencies.

## Inputs and artifacts

The reporter inventories tracked and non-ignored new `.rs` files under `src/`,
`tests/` and `examples/`. It excludes vendor code, reference checkouts, build output,
fixtures in other formats, and the development tool itself. Source changes during
analysis abort publication. The default build command uses `--locked` and writes
only to `target/tools/complexity`; `--no-build` explicitly reuses that helper binary.

Each run writes a new `target/complexity-runs/<timestamp>-<id>/` containing:

- `raw.json`: complete upstream metric trees, parse errors, test ranges and macro locations.
- `report.json`: every callable's inclusive and own scores, classification, coverage
  match status, source hashes, revision/working-tree status, tool lockfile and binary hashes.
- `functions.csv`: all callable rows for sorting/filtering.
- `report.md`: highest-scoring callables per category; `--top N` changes only the
  displayed excerpt. Every omitted row is counted and remains in JSON/CSV.
- Build/analyzer logs, exact analyzer command and publication status.

Only a successful complete report replaces the `target/complexity` symlink.
Failure preserves the previous successful report and the failed run's evidence.
Syntax errors, missing metric output and unsupported classification produce a
nonzero exit; high complexity scores do not.

## Interpretation and parser qualification

The analyzer's [cognitive and cyclomatic metrics](https://mozilla.github.io/rust-code-analysis/metrics.html)
are source-structure heuristics. We retain its upstream scores, including nested
closures/functions, and separately show **own** scores obtained by subtracting
child-space totals. Closures have their own rows. Do not sum inclusive rows across
a file or compare scores with another tool as though definitions were identical.

The helper checks both the analyzer's syntax tree error flag and `syn` parsing.
A malformed file is retained in raw evidence but excluded from trusted rankings;
any excluded file prevents successful publication. Rust probes cover async/await,
`?`, let-chains, guarded `match`, closures, macros, malformed/missing syntax, and the
expected distinction between flat and nested decisions. These probes are evidence
for the syntax exercised here, not a guarantee for every Rust construct.

Test-only item spans come from Rust attributes and AST traversal. `cfg` predicates
are evaluated with `test=false` and other predicates unknown; only definitely
excluded items are classified as test-only. `#[test]`/`#[tokio::test]` and known
standalone test paths (`tests/`, `tests.rs`, `*_tests.rs`, `regtest.rs`) also identify
tests. Examples form a third category. `cfg_attr` is conservatively flagged for
manual classification review and excludes its file rather than silently guessing.

This is not a compiled-feature report: all other feature branches remain present.
A production function can contain test-only statements; its inclusive score still
includes them, and the **Test attrs** column flags such functions. This also applies
to production functions containing nested test-only callables. Macro bodies and
procedural expansions are not analyzed as expanded Rust; macro counts make that
limitation visible. Large `ensure!`, `json!`, SQL strings and async state machines
can carry work that these scores do not express. Scores cannot establish lock
ordering, interleaving safety or financial correctness.

## Coverage comparison

By default the reporter reads `target/coverage/full.json` and `provenance.txt`.
Use `--coverage PATH` for another report directory. A missing directory means no
coverage comparison; missing/malformed files in an existing directory fail visibly.

Coverage is joined only when its provenance records a clean tracked source tree
and the current file's SHA-256 matches that revision's Git contents exactly.
Changed files show **source changed since coverage**, never an obsolete percentage.
Unknown provenance and uninstrumented spans have separate explicit statuses.
Byte-identical source does not prove that dependencies, tests or build features are
unchanged; the coverage revision and full provenance remain in the report.

The displayed count combines LLVM code regions contained in the callable's inclusive
line span, deduplicating identical coordinates across instantiations and counting
execution in any instantiation. It includes nested closures and can conflate
callables on the same line. It is a prioritization aid, **not LLVM's official
per-function percentage**, branch coverage or a new execution of the test suite.

## Review discipline

Prioritize functions that combine financial transitions, permissions, network
routing or cancellation with many branches. Extract coherent responsibilities,
retain ordering and ownership, and compare behavior with focused regressions.
A lower score alone does not justify an abstraction or establish correctness.
Keep score tables and before/after reports in ignored local output.
