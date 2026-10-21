# Rust complexity audit

Use `python3 scripts/complexity.py` from the repository root. `scripts/check.sh`
runs it and the analyzer's syntax qualification tests by default, separately from
application tests. Complexity warnings are advisory; there are no numerical merge
gates or application/runtime dependencies. First use downloads development crates;
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

The default comparison uses the merge base of `HEAD` and local `main`. Supply
`--base REF` for another PR target; `scripts/check.sh` accepts
`COMPLEXITY_BASE_REF=REF`. CI must fetch that ref and enough history for a merge
base. A missing ref or incomplete baseline fails visibly. `--no-compare` explicitly
requests a standalone report without regression warnings; the default check never
falls back to it silently.

The same current analyzer binary evaluates current source and exact Git blobs from
the base, including files omitted by archive attributes. No checkout, baseline build,
or historical code execution occurs. Baseline source, raw metrics, hashes and the
resolved commit are retained alongside the current report under ignored `target/`.

The reporter inventories tracked and non-ignored new `.rs` files under `src/`,
`tests/`, `examples/`, `tools/providers/` and `tools/diagnostics/`. Both current
and baseline discovery include these roots. It excludes vendor code, reference
checkouts, build output,
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
standalone test names (`tests.rs`, `*_tests.rs`, `regtest.rs`, `test_*.rs`)
also identify tests. Other `tests/` paths identify tests except the operational
runner under `tests/live/runner/`, its `tests/live/live_integration.rs` entry point,
and `tests/live/provider_preflight.rs`. Those sources and operational tools under
`tools/providers/` and `tools/diagnostics/` retain the `examples` category used
before relocation. Test names and test-only ranges take precedence over that
category. Moving operational code does not reduce its scrutiny or its scores.
Conditional lint levels (`allow`, `warn`,
`deny`, `forbid`, `expect`) do not affect classification and are accepted regardless
of their predicate. Other `cfg_attr` attributes, including mixed and nested lists,
are flagged for manual classification review and exclude their file rather than
silently guessing.

The pinned grammar predates Rust C strings. After validating original source with
`syn`, the helper replaces only each parsed C-string token's leading `c` with a
space for metrics. This covers ordinary/raw literals and macro tokens, preserves
byte positions/newlines and cannot add control-flow syntax. Raw evidence records
`c_string_lines`; syntax probes compare scores and locations with equivalent string
literals and still reject malformed Rust. Other grammar errors remain failures.

This is not a compiled-feature report: all other feature branches remain present,
including opt-in test code. Zcash application code is unconditional. Cargo feature
flags do not filter these metrics.
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

Review thresholds are **cognitive > 20** or **cyclomatic > 40**, using inclusive
scores. Console and Markdown warnings flag new/unmatched production or operational
example callables above either threshold, or matched callables whose above-threshold
metric increased. Unchanged and improving hotspots remain visible in the rankings;
test callables do not generate warnings. Warnings are also structured in
`report.json` under `comparison`, with the base revision, thresholds and old/new
values. Scores never change the command's exit status; incomplete current or base
analysis does, and preserves the previous successful publication.

Matching uses path, qualified name and category, never line numbers. Renames and
ambiguous duplicate names/closures are reported as new or unmatched rather than
guessed. When sources move, explicitly compare old/new path metric rows and
disposition unmatched warnings; a relocation is not a complexity improvement.
Unchanged score multisets for duplicate identities do not warn. Review
those warnings in context; helper extraction does not automatically justify a new
hotspot. These are review triggers, not an instruction to split coherent code merely
to meet a number. Own scores and nested callable rows remain available for review.

For substantive Rust refactors and architecture gates:

1. Compare the agreed baseline and changed sources with the same pinned analyzer.
   Check complete analysis, exclusions and source hashes before trusting results.
   Use a local comparison for a fixup as well as the original baseline when needed
   to distinguish a local improvement from accumulated branch complexity. Reuse a
   verified source-matched artifact when the relevant source has not changed.
2. Review relevant new/worsening warnings and unchanged high-risk hotspots. The
   short Markdown ranking and absence of new warnings do not establish that the
   whole design is simple; use the complete JSON/CSV. Prioritize financial
   transitions, permissions, network routing and cancellation ownership.
3. Inspect complete call chains, including new helpers, derived state, mandatory
   reads, blocking predicates and independent publication paths. Distinguish deleted
   responsibilities from moved complexity. Inclusive scores contain nested closures
   and functions; do not sum overlapping rows into a supposed chain score.
4. Give each relevant warning an explicit disposition: remove/simplify a
   responsibility, or evaluate simpler representations before explaining the
   concrete retained responsibility, why its ordering and ownership belong
   together, and which tests cover its failure boundaries. Necessary behavior and
   passing tests do not establish that its current organization is the clearest.
   Prefer a concrete simplification that removes redundant decisions, invalid states
   or ownership choreography while preserving required behavior, even when the
   existing implementation is correct and its high score has an explanation.
   Grouped conclusions should name the affected functions so warnings can be traced
   to their rationale. Saying scores are advisory is not a disposition.
5. Have the independent architecture reviewer examine those decisions and recurring
   fixup patterns. Resolve material findings; use any resulting simplification or
   design change to guide focused regression tests. A repeated pattern may justify
   a targeted wider audit even when its latest score did not increase.

Delete unnecessary policy and state before extracting coherent responsibilities.
Do not hide scores through helper extraction, add a workflow framework to distribute
branches, or weaken ownership/financial checks for a lower number. A justified
coordinator may retain a high score; thresholds are review triggers, not numerical
merge gates or instructions to refactor until a target is reached.

Record concise conclusions in the owning review or PR. Keep generated score tables,
before/after reports and investigation notes in ignored local output. Metrics and
coverage joins are review aids, not proof of correctness or test execution.
