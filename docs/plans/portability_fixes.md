# Portability fixes

## Intent, baseline and boundaries

Remove avoidable OS dependencies and support Linux, macOS and Windows through
small shared abstractions where native behavior is necessary. This document
records unfinished work and implementation options; the current request authorizes
documentation, not implementation, live qualification or spending.

The original audited baseline is `eece8ff`. The B8 private-storage follow-up was
rechecked against `1bd810eb`, after the review
of `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`. Independent source and build reviews were
read-only; no alternative-target compilation or native Windows/Linux validation
was performed. Findings below distinguish source-level blockers from expected
runtime failures and deliberately narrow qualification coverage.

The native suspend-aware clock implementation introduced in `7c4f0ae` was removed
in `d6bf6c7`. Application timing now uses standard Rust/Tokio clocks; no native
clock calls were found. The identified restrictions predate that removal. Do not
restore the removed observation-age authority or add suspend-specific clocks as
part of this work.

The permanent policy is [development portability](../development.md#portability),
linked from [AGENTS.md](../../AGENTS.md#engineering-preferences). Read the owning
[network guide](../network-egress.md), [wallet ownership/recovery guide](../wallet-rotation.md),
[runtime architecture](../architecture.md), [testing guide](../testing.md),
[development tools](../../scripts/README.md) and
[reproducible-build guide](../reproducible-builds.md) before changing their behavior.

Preserve journal-before-disclosure, exact accepted bytes and identities, restart
recovery, single-owner locking, private storage and accepted-work drain. All
production outbound clients remain in `src/network.rs`; Tor retains remote DNS,
identity isolation and no direct fallback. Do not introduce budgets, whole-work
deadlines or financial replay authority to make platform implementations easier.
Keep state formats, CLI/environment contracts, toolchain pins and existing
qualification evidence intact.

The refreshed B8 baseline includes `main` at `a626b998`, including randomized
funding and version-2 qualification registries with a running USDC API budget.
Preserve removed ZEC-budget policy, stable accepted work and historical registry
read-only access; a storage migration cannot upgrade old financial authority.

## Known gaps and removal options

P1 source correction is committed as `4fff946f`, with independently reviewed
fixture/CI fixups `574e0b38` and `5251194e`. Portable `ErrorKind` replaces the
Unix-only probe classification, and cfg-specific builder bindings remove the two
reported Windows warnings. All six controls and production/all-target warning
gates pass on macOS. Windows release build, CLI smoke tests and all six native
probe controls pass at `5218fef0`. The binary stdin/working-directory correction
`68a93c0b` and binary-output fixup `cec39403` also pass native vendor preflight.
Later all-target Clippy failed on C2's nested result tuple and two stored fixture
fields used only by Unix crash controls. The reviewed named-selection and fixture
storage corrections require a native rerun; full Checks and Linux execution
remain pending.
The standalone namespace experiment and buffer-only control `098a2397` failed
at rename before directory flush. Absolute-path/NULL-root control `3e97d27b`
passes both candidates at `5218fef0`: restricted-owner positive/admin-denial
controls, local NTFS, no-replace rename, preserved collision identities/bytes,
and child/root/anchor directory flushes. This is API compatibility evidence,
not complete P2 privacy, recovery or power-loss acceptance. The hosted image
also changed between runs, so the result alone does not isolate error 87's cause.
The simpler Win32 candidate is available for P2 design; full migrated-caller and
native security gates remain required. The table records the assessment baseline;
storage, parent control and native workflow gates remain open.

| Gap and evidence | Impact | Options and required preservation |
| --- | --- | --- |
| `qualification_probe` in [src/network.rs](../../src/network.rs) unconditionally references `libc::EPERM/EACCES`, while [Cargo.toml](../../Cargo.toml) declares `libc` only under `cfg(unix)`. Introduced in `185d968`. | Source-level non-Unix compilation blocker, even though the caller is a qualification tool. | Prefer portable `io::ErrorKind::PermissionDenied` if native tests establish the required denial classification. Otherwise isolate OS error mapping with Unix and Windows implementations. Simply making libc unconditional does not implement Windows error semantics. Preserve positive and denied probe controls; refusal, timeout and successful connection must remain distinct. |
| [Treasury store initialization](../../src/rotation/store.rs) and [backup publication](../../src/rotation/store/backup.rs) use plain `File::open(directory)?.sync_all()?`. | Expected Windows failure: directory handles need different opening semantics. This is a static assessment, not a Windows execution result. | Review all publication and recovery consumers, including key files and parent directories. Prefer an established portable primitive, or implement the necessary native handle/publication operations behind one contract. Opening a Windows directory handle alone does not establish flush or crash-durability equivalence. Never replace required synchronization with a successful no-op. |
| Private modes, owner/link checks and file identity checks are Unix-gated in [treasury storage/preflight](../../src/rotation/store/preflight.rs), [source storage](../../src/discovery/store.rs), [HTTP cache](../../src/http_cache.rs), [qualification registry checks](../../src/qualification.rs), wallet CLI and live-runner file helpers. | Non-Unix branches omit several privacy and alias protections. Compilation fixes alone cannot establish safe Windows operation. | Audit complete create/open/replace/backup call chains. Use Windows ACLs, handle-based file identity and reparse/link handling where required; retain Unix implementations. Reuse a reviewed dependency if it supplies the actual contract. Reject an operation explicitly while a required guarantee is unavailable, without treating that rejection as completed support. |
| [Parent supervision](../../src/supervision.rs) uses Tokio Unix pipe receivers and rejects enabled parent control off Unix. Unix SIGTERM handling also appears in serving and runner shutdown paths. | Qualification parent control is explicitly Unix-only; ordinary serving has a disabled-control path and Ctrl-C handling. | First consider deleting unnecessary control responsibilities if equivalent ownership is already established. Otherwise provide cancellable parent-lifetime control on all three OSes, preserving EOF/protocol errors, startup cancellation and draining of accepted financial work. Do not use a blocking stdin thread that prevents shutdown. Define Windows termination behavior explicitly. |
| [Live-integration execution tests](../../tests/live/runner/execution_tests.rs) import Unix permission APIs and execute a `#!/bin/sh` child; their module in [execution.rs](../../tests/live/runner/execution.rs) is only `cfg(test)`. Other shutdown/filesystem tests intentionally use Unix gates. | Windows `test --all-targets` has an additional compile blocker and shell-dependent fixtures. Gated tests leave behavior unverified elsewhere. | Replace the shell child with a small portable Rust fixture and use platform storage helpers. Retain native tests for OS-specific mechanisms, but exercise the shared supervision, permissions and recovery contracts on every OS. Guarding away the failing tests is only an interim compile fix. |
| [Makefile](../../Makefile) and remaining ordinary shell tools require GNU Make 3.81+ and POSIX `sh`. [protoc helper](../../tools/protoc/src/main.rs) expects a supported bundled host binary. | Native Windows commands need a shell environment or equivalent orchestration. The locked protoc packages cover common Linux, macOS and Windows hosts, not every Rust host. | Prefer one portable development driver, or document equivalent native commands preserving pins, locked resolution and protoc setup. Keep a system-protoc fallback explicit and version-checked if added. Distinguish the helper's build host from the application's target. Audit native SQLite, cryptography and default-Zcash dependencies on each target; do not infer portability from the Makefile alone. |
| [Reproducible release runner](../../scripts/reproducible.py) and [release profile](../../scripts/release-environment.json) use macOS sandbox-exec, Apple tools and an Apple Silicon/Homebrew environment. | Historical qualification covers the earlier Rust/Cargo 1.99.0 profile; the current 1.99.1 profile needs a fresh two-build comparison. Ordinary application builds are a separate concern. | Separate shared snapshot/hash/provenance logic from native environment and network-sandbox backends. Add Linux and Windows profiles with verified OS network denial before claiming qualification. Never weaken the sandbox or repin the existing profile merely to obtain a passing result. |
| [Tor qualification](../../scripts/qualify_tor.py), [live confinement](../../tests/live/runner/tor/confinement.rs) and [startup measurements](../../scripts/measure_startup_tor.py) assume macOS sandbox-exec and native measurement tools. | Optional qualification/measurement is macOS-specific; this does not make Tor serving itself macOS-only. | Share orchestration and evidence contracts; implement native confinement and measurement backends. Preserve positive/negative TCP and UDP controls, owned-process coverage and cancellation. A proxy configuration or local SOCKS fixture cannot replace OS confinement evidence. Mark unavailable measurements explicitly and retain native units/provenance. |

The Makefile's Rust/Cargo 1.99.1 pin, the manifest's Rust 1.94.1 language minimum,
native compiler requirements and public Sapling parameter preparation are build
requirements, not evidence of an OS exclusion. No CI platform matrix was found;
the complete dependency graph remains to be validated natively.

## B8: shared private-storage contract

B8 identifies a latent privacy/correctness gap, not merely duplicated mode-setting
code: non-Unix private-options branches can return success without equivalent
protection. Inventory treasury store and preflight, backups, HTTP cache, source
registry, wallet key/state CLI, qualification and live-runner file helpers before
P2. Include qualification registry authorization/budget state, atomic API admissions
and retained historical registries. Trace create, reopen, replace, recovery and cleanup, including the new
state-local preflight temporary directory. A private parent directory alone does
not establish every file/alias/publication contract.

P2 should consolidate these paths behind the smallest shared secure-filesystem
abstraction, using portable primitives where equivalent and isolated Linux/macOS/
Windows implementations where necessary. Specify owner access, existing-file
validation, symlink/reparse rejection, identity/alias checks, exclusive ownership,
atomic publication and required directory durability. Avoid a broad filesystem
framework or one helper per caller. Native ACLs and handles must establish actual
Windows guarantees; successful no-op implementations are unacceptable. Explicit
operation refusal when a required guarantee is unavailable is honest unfinished
support, not native acceptance. Review dependencies under the development policy
before introducing or vendoring one.

Extend P2 acceptance with native permissive-existing-file, hostile parent/link/
reparse, alias, lock-contention and publication-failure fixtures; verify accepted
financial journals survive interruption/reopen and failed publication never grants
replay. Check confidentiality from a distinct security principal where the OS
contract needs it, rather than infer it from Unix mode bits or current-user access.
Update the owning storage sections of wallet/source/cache guides and
`docs/development.md#portability`; retain the AGENTS portability link there.
At G1/G2 independently inspect every migrated caller and remaining non-Unix
branch, requiring explicit evidence or a documented refusal for each guarantee.

## Ordered implementation boundaries

These proposed commits define coherent work, not authorization to begin it.
Before each substantive implementation, an independent read-only design reviewer
must challenge the selected option, need for native code and opportunities to
remove responsibilities. Resolve material findings before building on a decision.

1. **P1 — Restore non-Unix library and binary compilation.** Fix probe error
   classification and inspect consumers in both qualification runners. Acceptance:
   default-Zcash library/binary compilation and focused probe tests on Linux,
   macOS and Windows, covering reached, permission-denied and other failures.
   All-target test compilation remains blocked by the fixture addressed in P2.
   Update `docs/development.md` and `docs/testing.md` with remaining support gaps.
2. **P2 — Implement equivalent storage guarantees.** Resolve directory publication,
   privacy, file identity and link handling together with their necessary recovery
   paths. Cover treasury initialization, backups, wallet CLI, source/cache stores
   and qualification/live-runner files. Replace the unguarded shell/Unix execution
   fixture with a portable Rust child using the storage helpers in this commit:
   that fixture calls `Store::create`, so Windows runtime acceptance depends on
   this storage work. Acceptance: default-Zcash all-target compilation, execution
   fixture tests that actually run, and native private-file/ACL,
   alias/link/reparse, ownership exclusion, publication-failure and restart tests,
   including absence of duplicate financial work or discarded evidence. Use
   temporary state and public unfunded keys. Update `docs/wallet-rotation.md`,
   `docs/wallet-cli.md`, `docs/agent-sources.md` and `docs/testing.md` as affected.
3. **P3 — Port parent control and process lifecycle.** Implement the selected
   cancellable control mechanism and equivalent native fixture coverage without
   splitting removal from replacement. Inspect startup, ordinary serving and
   live-runner consumers. Acceptance: EOF, invalid input, parent disconnect,
   cancellation during startup, accepted-work drain and ordinary Ctrl-C/native
   shutdown tests on all three systems. Update `docs/architecture.md`,
   `tests/STARTUP.md` and the live runbook where contracts change.
4. **P4 — Provide and verify ordinary native build workflows.** Choose the
   smallest portable driver or equivalent native commands; audit protoc hosts,
   default-Zcash dependencies and native toolchains. Establish repeatable checks
   on all three OSes through CI or retained reproducible commands. Acceptance:
   default build, relevant native tests, CLI smoke tests, format and applicable
   Clippy checks; record exact toolchains/targets and unresolved dependency gaps.
   Use the single application build and requested opt-in test features. Update
   `docs/development.md`, `scripts/README.md` and setup instructions in `README.md`.
5. **P5 — Extend optional qualification backends.** Independently port release
   reproducibility, Tor confinement and measurement where useful. Keep each backend
   and its evidence as a coherent commit if the work requires subdivision; assign
   stable identifiers before implementing those subdivisions. Acceptance: native
   sandbox positive/denied controls, unchanged failure semantics, preserved
   evidence and cancellation; reproducibility compares two original binaries in
   independent directories. Update `docs/reproducible-builds.md`, `tests/TOR.md`,
   `tests/STARTUP.md`, `scripts/README.md` and `docs/testing.md` with the exact
   qualified scope. P5 is not a prerequisite for ordinary Linux/Windows support;
   unavailable optional qualification continues to fail explicitly.

## Review gates, evidence and handoff

For each implementation commit, freeze sources during validation and obtain an
independent review of its exact committed SHA and relevant consumers. Give the
reviewer user intent, baseline, invariants, actual commands/results and limitations;
ask them to verify claims and question unnecessary enforcement. Address findings
with `git commit --fixup=<introducing-SHA>`, independently review each exact fixup
SHA and preserve unsquashed history through final integration review.

- **G1, after P2: storage and compilation gate.** Independently inspect all P1/P2
  commits and fixups with complete registry, treasury, backup and publication call
  chains. Exit requires no material compilation, privacy, durability or recovery
  findings, native evidence for claimed guarantees and accurate remaining gaps.
- **G2, after P4: ordinary application integration gate.** Review all prior commits
  and fixups with startup, serving, cancellation, storage and outbound transport
  consumers. Exit requires equivalent native behavior and usable documented build
  workflows before claiming three-platform application support. Keep optional
  qualification claims separate.
- **G3, after the selected P5 work and plan cleanup: final integration gate.**
  Independently review the accumulated branch, all fixups, qualification backends,
  evidence consumers and documentation. Review the exact cleanup SHA as well.
  Exit requires no unresolved material findings and claims limited to completed
  native validation; unfinished optional backends remain explicit focused work.

At every gate examine fixup causes, recurring assumptions, hazards and missing
coverage across the full call chains. Decide whether findings require a wider
audit, regression tests, simplification or a changed design. Earlier approvals
do not exempt code. Material later changes reopen affected gates and validation.
For substantive Rust refactors and architecture gates, compare CogC/CycC with the
agreed pre-change baseline using [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline);
review changed functions, full call chains and unchanged high-risk hotspots. Test
and complexity results do not replace native semantic validation or justify
helper extraction solely to reduce scores.

Keep permanent explanations in the owning guides with each implementation commit.
The AGENTS portability link must continue to point to `docs/development.md`, not
to a completed plan. Before removing this plan, verify guide handoff and references,
remove its link from development guidance and retain only concrete unfinished work
if any. Do not keep an implemented-plan archive. Generated reports stay under
ignored `target/` or private external storage; never delete financial evidence.
