# Rust complexity audit

Run `python3 scripts/complexity.py` from the repository root. `make check` runs
it and the analyzer's syntax tests by default, separately from application
tests. Complexity warnings are advisory (no numerical merge gates and no
runtime dependencies). First use downloads development crates; later runs can
use `CARGO_NET_OFFLINE=true`. No source is sent to an external service. The
orchestrator requires Python 3.11+.

The workspace helper in `tools/complexity/` pins Mozilla's
[rust-code-analysis](https://github.com/mozilla/rust-code-analysis) to
**0.0.25**, sharing the root `Cargo.lock` and development profile. That
lockfile pins Tree-sitter to **0.20.9** (fresh resolution can pick Tree-sitter
0.27 via `tree-sitter-rust` 0.20.3, which is incompatible with 0.20.9).
Preserve this grammar pin on updates, and re-run syntax probes when changing
the analyzer or grammar cohort. The analyzer stays outside the default
application package selection.

## Inputs and artifacts

The default comparison uses the merge base of `HEAD` and local `main`. Pass
`--base REF` or `COMPLEXITY_BASE_REF=REF` for another branch target (CI must
fetch enough history for a merge base; missing refs fail visibly). Pass
`--no-compare` for a standalone report without regression warnings.

The current analyzer binary evaluates both current source and exact Git blobs
from the base revision without checking out or running historical code.
Baseline source, raw metrics, hashes, and the resolved commit are kept under
ignored `target/`.

- **Inventoried roots:** Tracked and non-ignored new `.rs` files under `src/`,
  `tests/`, `examples/`, `tools/providers/`, and `tools/diagnostics/`,
  excluding `vendor/`, reference checkouts, `target/`, non-Rust fixtures, and
  `tools/complexity/`.
- **Build isolation:** Defaults to `--locked` in the root `target/` directory
  (`--no-build` reuses `target/debug/treazury-complexity`), overriding any
  inherited `CARGO_TARGET_DIR`. Source edits during analysis abort publication.
- **Per-run output (`target/complexity-runs/<timestamp>-<id>/`):**
  - `raw.json`: upstream metric trees, parse errors, test ranges, and macro
    locations.
  - `report.json`: every callable's inclusive and own scores, category,
    coverage match status, source hashes, working-tree status, `tool_lock_sha256`,
    and binary hashes.
  - `functions.csv`: all callable rows for sorting and filtering.
  - `report.md`: top callables per category (`--top N` adjusts only this
    excerpt; omitted rows stay counted and remain in JSON and CSV).
  - Build and analyzer logs, exact command, and publication status.

Only a complete successful run updates the `target/complexity` symlink;
failures preserve the previous report and failed logs. Syntax errors, missing
metrics, and unsupported `cfg_attr` classification exit nonzero; high scores
do not.

## Interpretation and parser qualification

The analyzer's
[cognitive and cyclomatic metrics](https://mozilla.github.io/rust-code-analysis/metrics.html)
are structural heuristics. Both upstream **inclusive** scores (including nested
closures and functions) and **own** scores (subtracting child spaces) are
reported; do not sum inclusive rows across a file or compare raw numbers
against tools with different definitions.

- **Dual parsing:** Both the analyzer's tree-sitter error flag and `syn`
  validate every file; malformed files abort publication. Syntax probes cover
  `async` and `await`, `?`, let-chains, guarded `match`, closures, macros,
  malformed syntax, and flat vs. nested decisions.
- **Classification:** `#[test]`, `#[tokio::test]`, `cfg(test)` items, known
  test filenames (`tests.rs`, `*_tests.rs`, `regtest.rs`, `test_*.rs`), and
  `tests/` files classify as tests, except operational code under
  `tests/live/runner/`, `tests/live/live_integration.rs`,
  `tests/live/provider_preflight.rs`, `tools/providers/`, and
  `tools/diagnostics/`, which stay in the `examples` category. Conditional lint
  attributes (`allow`, `warn`, `deny`, `forbid`, `expect`) do not alter
  classification; other `cfg_attr` attributes exclude their file for manual
  review.
- **Grammar compatibility preprocessing:** After `syn` validates original source,
  the helper blanks the leading `c` on C-string literals (`c_string_lines`) and
  the `unsafe` keyword on `unsafe extern` blocks (`unsafe_extern_lines`) while
  preserving byte positions and newlines (leaving `unsafe fn`, `unsafe` blocks,
  comments, and string contents untouched).
- **Metric limits:** All Cargo feature branches are analyzed together (not
  feature-filtered). Production functions containing test-only items flag the
  **Test attrs** column. Macro expansions, large `ensure!` or `json!` blocks,
  SQL strings, lock ordering, and financial correctness are outside what these
  scores measure.

## Coverage comparison

By default, the reporter reads `target/coverage/full.json` and `provenance.txt`
(`--coverage PATH` overrides). A missing directory skips coverage; malformed
files inside an existing directory fail visibly.

- Coverage joins only when provenance records a clean tracked tree and the
  file's SHA-256 matches that Git revision byte-for-byte; modified files show
  **source changed since coverage** instead of stale percentages.
- Counts combine deduplicated LLVM code regions within each callable's line
  span (including nested closures and co-located callables) as a prioritization
  aid rather than LLVM's official per-function or branch percentage.

## Review discipline

Review thresholds are **cognitive > 20** or **cyclomatic > 40** (inclusive).
Warnings flag new or unmatched production and operational example callables
above either threshold, or matched callables whose above-threshold metric
increased (test callables do not warn). Matching uses file path, qualified
name, and category (not line numbers); moved files produce unmatched warnings
that must be compared against their old rows.

For substantive Rust refactors and architecture gates:

1. Compare the agreed baseline and changed sources with the same pinned
   analyzer, checking complete analysis, exclusions, and source hashes (reuse
   a verified artifact when source has not changed).
2. Review new and worsening warnings alongside unchanged high-risk hotspots
   in the complete JSON and CSV, prioritizing financial transitions,
   permissions, network routing, and cancellation ownership.
3. Inspect complete call chains (helpers, derived state, blocking predicates,
   and publication paths) and distinguish deleted responsibilities from moved
   complexity. Never sum overlapping inclusive rows.
4. Disposition every warning above threshold: remove or simplify a
   responsibility, or explain why the retained responsibility's ordering and
   ownership belong together and which tests cover its failure boundaries
   (naming the retained functions). Saying scores are advisory is not a
   disposition.
5. Have the independent architecture reviewer examine those dispositions and
   recurring fixup patterns, using resulting simplifications to guide focused
   regression tests.

Delete unnecessary policy and state before splitting functions. Never hide
scores with artificial helper extraction, add a workflow framework to spread
branches, or weaken financial or ownership checks to hit a number. Record
concise conclusions in the owning review or PR and keep generated tables under
ignored `target/`.
