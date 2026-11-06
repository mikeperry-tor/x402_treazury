# NU7 release readiness and bridge qualification

## Intent and scope

Own near-term consensus readiness, general Rust dependency updates, dependency
security, bridge evidence and retirement of
wallet-owned transparent address support under the
[SDK integration contract](../zingo-integration.md) and
[refund/recovery policy](../wallet-rotation.md#refunds-state-and-recovery). The operator reports NU7
activation on November 5, 2026; verify current network activation height, consensus
parameters, deployed indexer/submission capabilities and exact upstream cohort
before changing production dependencies. Do not hardcode that date or assume all
NU7 work is missing: current preparation already calls the pinned SDK's
upgrade-aware expiry rule and current sync has Ironwood-related integration.

This is unfinished release work, not a completion claim or authority for funded
execution, paid API calls, production key access, upstream submission or deployment.
Capture a fresh exact baseline when the track starts. Preserve original accepted
bytes, identities, input ownership, liabilities and stable storage/environment
contracts. Follow [SDK integration](../zingo-integration.md),
[wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[network policy](../network-egress.md), [development](../development.md),
[testing](../testing.md), [release builds](../reproducible-builds.md) and
[vendor provenance](../../vendor/README.md).

## Execution baseline and commit gates

Working branch: `nu7_release_readiness`. Starting baseline:
`571eba2f8a8f4df9d3f3e89d5992ccee93f81154`. The unrelated untracked `TODO`
is outside this work. Public source retrieval and unfunded diagnostics are permitted.
The operator separately approved the reviewed offline inspection of only
`state/research_desk` and its default key. That approval covers saved evidence
read/decryption and private output. The operator subsequently approved one
separately reviewed existing-quote status GET through the selected Tor policy;
that query has executed and its one-request authority is consumed. Further private
network requests, sync/rescan, signing, funded calls, submissions and deployment
remain outside current authority.

The N1–N6 assessment boundaries below remain acceptance requirements. Execute
these coherent commits in order, refining candidate details from exact sources:

| Commit | Boundary and permanent documentation | Acceptance and review gate |
| --- | --- | --- |
| C0 | Retain baseline, implementation boundaries and review evidence in this plan. | Independent read-only design review before substantive implementation. |
| C1 | N1: migrate the complete SDK/backend cohort to verified v8 tag, carry necessary Pepper patches and provenance, adapt offline/indexer/send/sync consumers; assess general workspace and protoc updates. Update SDK integration and vendor ledger. | Exact source comparison, vendor integrity, production compiler and all-target Clippy, compatibility and affected lifecycle/optimized checks. Independent exact-SHA and CogC/CycC review. |
| C2 | N2: reproducible local advisory assessment of active lockfiles and path sources, resolve signing advisory with compatible migration or evidence-based exposure/release disposition; update development/vendor/testing/tooling guidance. | Exact source and signing-call-chain assessment, independent vectors and payment compatibility; exact-SHA review. Dependency gate combines C1/C2 and all fixups. |
| C3 | N3/N4: verify deployed refund format/receiver and published service semantics; fix demonstrated gaps only, with network/wallet/live-runbook handoff. | Public unfunded route checks, faithful contract regressions where code changes, independent exact-SHA review. Bridge gate combines all preceding work before dependent retirement. |
| C4 | N5: offline authenticated note inspection and reviewable qualification recipe, preferring retained evidence and the simplest supported controlled-refund trigger. | Public-fixture tests prove read-only/no-network/no-secret-output behavior and exact SDK note facts; independently review command and recipe before separate production-key authority. Spending/copy-scan/harness authority remains separate. |
| C5 | N6: retire ordinary wallet-owned transparent behavior only when receiver and funded evidence support it; preserve explicit legacy recovery and accepted obligations. | Fresh/imported/reopened/seed-recovery negative RPC/address controls, shielded discovery/spendability, legacy/restart regressions; exact-SHA and applicable complexity review. Blockers remain explicit if prerequisites are unavailable. |
| C6 | Final integration and documentation handoff; delete completed plan only when all release conditions and reviews pass. | Independent full-chain combined review of all commits/fixups, required ordered checks, exact cleanup-SHA/final-branch review. |

Keep fixups unsquashed and review each exact SHA. Run cheap affected contracts and
`check-static`, complete applicable complexity review, then `check-clippy` and
`check-compat` before remaining named runtime stages. Carry forward unchanged
passing evidence. Optional native consensus, real Tor, funded and reproducibility
qualification require their actual runbooks; unavailable evidence stays missing.

### Evidence and remaining work

- C0: exact `411ecdce2eaaa340b0a9a7a89ce5afc72d53e91b` independently
  reviewed; no material finding. Design review selected upstream's coordinated
  1000-block retention and previous-session-height fix while retaining local
  conservative recovery; preserving 100 would add an unnecessary divergence.
  Exact mainnet activation/branch/expiry regression selected to cover the
  prerelease schedule gap. Subsequent implementation validation is recorded below.
- Refund design: assess documented EXACT_OUTPUT excess-input refund as a simpler
  candidate before a duplicate-deposit harness; applicability and fees still need
  route evidence. Dry quotes establish format acceptance only, not returned
  deadlines or terminal behavior. No funded execution authorized.
- Verified public v8 tag resolves to `81d799775f11e882896e8710e8af12d6d4d29a41`
  through Git remote metadata and a read-only checkout under ignored `target/`.
  Release notes and source cohort must agree before migration.
- N5 funded/copy-scan authority and actual refund attribution remain outstanding;
  bounded offline inspection is approved and executed as recorded below. N6 is conditional.

### Earlier review and validation checkpoint

Working branch remains `nu7_release_readiness`; accumulated implementation includes
C1 `69d7d7cd` and fixups `b4502c21`, `3c074f7c`, `a17ad30a`; C2 `469179d8`
and fixups `8a7b983e`, `1a272ba9`, `9fac678d`; timing `89a6d772` and fixups
`0cda4e00`, `48ba21dc`; bridge recipe `29ef2b83`; N6 assessment `ce0a2cb6`.
Each exact SHA has independent non-author source review. The following records
the earlier checkpoint; later checkpoints supersede its pending validation statuses.

- Initial `check-static` passed: formatting, 92 then-existing Python tests, ten
  analyzer qualification tests, complete 466-file application complexity comparison
  to baseline `571eba2f`, and all transport/Pepper provenance checks.
- Four new advisory-tool tests pass. A subsequent 96-test tooling run exposed a
  pre-existing release fixture merging Git stderr into expected revision stdout
  when macOS emitted a sandbox `confstr` warning. `a17ad30a` separates the streams;
  all 16 affected release-tool tests pass. The remaining 80 cases from that run
  passed and their evidence is unchanged; no runtime assertion was weakened.
- Independent application complexity review verified current Rust hashes, no
  exclusions or new warnings. Timing changes introduce no parallel owner/state;
  healthy interval selection delegates to SDK consensus. Complete JSON/CSV artifacts
  are under `target/complexity/`. Unrelated root dependency updates change the full
  lock hash but leave the analyzer's 96 dependency records unchanged.
- Supplementary same-analyzer comparison of eleven changed Pepper files is under
  `target/nu7-readiness/vendor-complexity/`. Valid hotspots: `scan_compact_blocks`
  CogC 51→53/CycC 43→44 (shared sparse-checkpoint membership), and `sync` CogC 46
  unchanged/CycC 83→84 (ordered per-task result admission). Independent review
  retained these coherent upstream responsibilities. Serialization and witness
  files have grammar failures in both snapshots: their numbers are untrusted;
  manual review covered address encoding, NU7 grid/spacing and checkpoint retention.
  This is partial vendor numerical coverage, not complete parser qualification.
- At this earlier checkpoint, production compiler/all-target Clippy was running;
  compatibility, remaining runtime/proving/lifecycle/codec and optimized evidence
  were outstanding. The current checkpoint below records their completion.
  No native consensus, funded settlement, real Tor or reproducibility pass claimed.

General-update assessment selected compatible root/workspace updates and protoc
3.3.0, plus MCP 3.9.0 and direct SHA-2 0.11.1. Independent design review approved base64 0.23.1, direct analyzer syn 3.0.9 and
native secp256k1 0.33.1 as a further batch. The first compiler stage stopped on
SHA-2 0.11's removed digest-array LowerHex trait; `ac58a6e9` converts all consumers
to explicit canonical lowercase encoding and adds an independent leading-zero
hash vector. No stored hash/domain/input representation changes. After that failed
stage drained, the batch was applied with current published Hyper 1.12.0 and h2
0.4.20: exact-source review found neither cancellation checkout can retire, and
both patches rebase without fuzz. Public archive bytes, upstream provenance-only
lockfiles and all blocker records remain verified. `74a3e95e` removes obsolete
mutable reorg-start accumulation after independent simplification review.
At that stage syntax/signature/wire and full compiler/runtime evidence were
outstanding; later runs below complete them. The analyzer's exact Tree-sitter
0.20.9/grammar cohort stays pinned because 0.27 is API-incompatible. The prior
96-record analyzer comparison predates direct syn 3 selection; refresh syntax
qualification and source-matched metrics for the resulting tool before broad suites.

Other assessed incompatible-version candidates are not forced into this cohort:
ChaCha20Poly1305 0.11 replaces the randomness API used for existing state keys/nonces
and needs an independent old-ciphertext/AAD reopen vector; rand 0.10 changes extension
traits and removes the cloned deterministic test RNG; TOML 1.2 changes parser/value
semantics; rusqlite 0.41 changes the bundled SQLite/FFI and requires custom VFS,
WAL/reopen/busy requalification. No blanket incompatibility claim is made. Retain
current supported versions for this readiness cohort; assess those migrations
against their named storage/configuration/sampling contracts when selecting the
next general-update boundary. Current advisory findings are not hidden by these
retentions, and the exact graph remains replayable.

### Follow-up validation and timing audit

The local source checkpoint at `21b5b961` includes independently reviewed SDK
fixture fixup `63e1c544`, native parity fixup `096fd1af`, timing fixture fixup
`83648b0a`, and notes fixups `7a39cd20`/`21b5b961` after inspector
`58a11493` and its CLI fixture `85cedd7b`. All exact SHAs have non-author review.
The original compiler failures exposed removed digest formatting, native recovery
conversion and SDK test APIs; fixes preserve original contracts rather than weaken
assertions. Production compilation passes in
`target/nu7-readiness/check-clippy-after7a39.log`; full all-target
`zcash-regtest` Clippy and emitted-dependency-warning checks pass in
`target/nu7-readiness/clippy-all-targets-after21b5.log`. The latter is a focused
continuation after test-only copy fixes; unchanged production evidence carries
forward. No runtime compatibility pass is claimed yet.

Final-batch static checks passed all 96 Python tests, ten Syn-3 analyzer
qualification tests and transport/Pepper provenance checks. The refreshed
`target/complexity/report.json` at `21b5b961` includes all 467 application Rust
files, matching hashes, no exclusions and no regression warnings; independent
source/complexity review found no new material issue. Supplementary same-analyzer
vendor coverage now covers 58 changed Pepper/Hyper/h2 files under
`target/nu7-readiness/vendor-complexity-final-batch/`, with four before/after
serialization/witness grammar errors still explicitly untrusted and manually
reviewed. Native consensus, live-funded, real-Tor and reproducibility claims remain
outside this evidence.

Advisory replay at the same checkpoint is retained under
`target/nu7-readiness/security-assessment-after21b5/`: root eight vulnerability
matches and five unmaintained notices, protoc and normalized nonregistry inventory
zero matches. Exit 1 means findings, not clean. The active lockfiles and pinned
database/tool versions remain recorded in that report; maintained exposure
assessments still apply.

A further independent timing audit found no other demonstrated pre-NU7 wall-time
assumption, but identified one material source-observation gap: gRPC lookup
acquisition/validation failures were collapsed twice into healthy `Unknown`.
Faster successful source polls would therefore bypass existing failure backoff
during lookup-only outages. The reviewed minimal fix propagates errors through the
existing Result contract, preserves sanitized diagnostics, explicit healthy unknown
and cancellation outcomes, and leaves fulfilled-source, SDK-confirmation and
buried-expiry proof precedence intact. No counter, timer, replay or new financial
guard is added. Existing transport, scheduler and synthetic-expiry fixtures cover
failure versus unknown, scoped backoff, retained bytes/inputs/attempts and proof
completion without lookup. Compiler/Clippy, refreshed complexity review and actual
fixture execution for this correction were pending at that point and subsequently
passed as recorded below. The compatibility build was
interrupted before runtime execution to keep source/executable evidence coherent;
its completed dependency artifacts are retained for resumption.

### Offline inspection implementation gate

Exact implementation `73df9ed23229d97e414178fe614c7c9d9740ca68` has a fresh
independent combined source/complexity review against `571eba2f`, including all
accumulated commits/fixups and complete affected consumers. No remaining material
source finding or compelling simplification was identified. Review resolved the
`703a1dae` scheduler assertion with `73df9ed2`: the unchanged second exponential
step remains below the 60-second floor, rather than producing 120 seconds.

The production compiler passes in `check-clippy-after73df.log`; after the operator's
VS Code restart, full all-target `zcash-regtest` Clippy and emitted-warning checks
pass in `clippy-all-targets-after-restart.log` (all paths here are under
`target/nu7-readiness/`). Two orphaned editor checks from before that restart were
stopped; no new editor Cargo launch was observed. At that checkpoint, source-matched complexity
covers all 467 files/10,358 callables with no exclusions or threshold warnings;
independent review also covered the supplementary vendor grammar gaps manually.

Executed evidence at this source:

- `check-compat-after73df.log`: 31 tests pass (crypto vectors2, payments15,
  SDK3, MCP/server11), including actual activation/expiry and wallet restore.
- `focused-cli-submission-after73df.log`: three tests pass, including actual
  standalone/configured offline notes CLI, source lookup failure classification,
  no implicit sends and unchanged financial records.
- Default notes/routing3, native signer vector1, source scheduler1, treasury
  observation4 and digest1 pass in their `focused-*-after73df.log` files.
- `focused-notes-feature-after73df.log`: all five `zcash-testutils` note fixtures
  execute and pass, including authenticated three-pool evidence with owner held,
  SDK membership, spent/pending/incomplete records and exclusion of memos/secrets.

Debug library-test codegen emits the macOS system-linker `__eh_frame` offset warning;
filtered runs replay it from the same artifact. Independent inspection found both
DWARF and compact-unwind sections retained, with a possible fallback-search cost
and no demonstrated correctness failure. Do not describe these runs as warning-free.
The disposition preserves panic/unwind/profile/toolchain contracts and still needs
planned panic/cancel/reopen/lifecycle and optimized execution. The owning testing
guide records this diagnostic; production/Clippy warning enforcement is unchanged.

Final-source replay in `security-assessment-after73df/` is independently verified:
all 16 nonregistry source fingerprints and both active lockfiles match, pinned
cargo-audit0.22.2/database `26fd2d43` retain root eight vulnerability matches and five
maintenance notices, with zero protoc/nonregistry matches. Exit1 remains findings;
maintained exposure dispositions agree. No claim of a clean advisory report.

C4's concrete bounded inspection recipe is independently reviewed at
`target/nu7-readiness/refund-inspection-recipe.md` (SHA-256
`83bb4ddc487c69b26109d602fba7e96cf5c13fe5524c8cb3605373d3143a9425`). It names only
`state/research_desk`, its default key and the selected deployment configuration,
with keyless JSON status and authenticated notes reads. A byte-identical frozen
copy of the actual tested default CLI is pinned in `inspection-bin-manifest.json`
so subsequent builds cannot replace the inspection executable. Config/executable
hash checks, treasury-identity comparison, coherent-revision limitations and fresh
owner-only private captures are explicit. The operator subsequently approved this
exact offline scope, including the selected key. Frozen CLI status and notes
inspections both succeeded with coherent identity/revision. No other wallet
directory was inspected. No scan, mutation, signing or spending was included.

The public unfunded UA helper is independently reviewed and successfully encodes
the current all-shielded and Orchard-only protocol receivers using the matching
Cargo application-profile keys0.17/protocol0.11 artifacts; identities are retained
in `public-format-dependency-artifacts.json`. Fresh provider dry quotes and all ordered local checks subsequently pass as
recorded in the current checkpoint; deployed financial qualification remains open.

### Current validation and approved offline evidence checkpoint

Exact application snapshot `0e8445aa5fd4c4d2ecd28f141f0a535417e10288` has independent
combined source/complexity review against `571eba2f`, including all accumulated
commits and fixups, with no remaining material source finding or compelling
simplification. Source-matched analysis covers all 467 files/10,358 callables,
without exclusions or comparison warnings. Manual vendor grammar-gap review carries
forward. Later changes are test-only: `8c9438e9` corrects four scheduler assertions
that retained the removed healthy minute floor; `0e8445aa` corrects the corresponding
real-worker lifecycle test, including the SDK's synthetic NU7 scan interval25.
No production timer, authority or financial assertion was weakened.

Full all-target Clippy passes without diagnostics in
`clippy-all-targets-after0e84.log`. Unchanged production compiler, 31 compatibility
tests, static/provenance checks and advisory dispositions carry forward from the
recorded earlier runs. Default continuation at `8c9438e9` passes 1,909 tests across
57 harnesses, with 42 ignored; all 40 scheduler tests pass. Proving passes four
cases. The initial lifecycle run passes 56/fails 1/ignores 1; the failure is exactly
the obsolete lookup-floor assertion, not a production failure. The corrected exact
worker test and codec each pass. The full lifecycle retry passes 57/ignores 1 in
`check-lifecycle-after0e84.log`; optimized payment/SDK/crypto 20 and focused
signer/timing/notes 10 all pass in their `release-*-after0e84.log` files, with no
compiler/linker diagnostics. Debug library tests retain the explicitly
dispositioned macOS unwind-table warning; no warning-free claim applies to them.

The approved offline CLI evidence was supplemented by an independently reviewed
one-off binding reader, retained only under ignored `target/nu7-readiness/`.
Source SHA-256 `1165c5ba8c96ac3cc912ee5ca305bdbfbe8b07ecbc3aa2d619bdca3af0da681f`
and frozen post-signing executable SHA-256
`52f945248cbb9aa4878c59ac747e720deaa6f1af19da03bd905f6c51a6682671` have four
passing public fixtures, including actual CLI, active WAL/held ownership,
authentication/context mismatch and scoped record corruption. Revised offline recipe
SHA-256 `253f17e7cdd6b169ce0558df7ccd304eb1fdb0971ed39c697556cc13e4419c7a` received
independent execution review under the existing operator approval. The reader uses
one read-only authenticated transaction; no SDK or owner is initialized. Its 17
unused shared-module warnings are recorded, not suppressed; its test build has no
diagnostics. Inspection succeeds with the same saved identity/revision as notes,
and selected database/key/WAL hashes are unchanged. Private artifacts remain in
fresh owner-only ignored storage; no keys, addresses, amounts or transaction IDs
are committed here.

Retained evidence supplies one shielded-receiver candidate with saved confirmation,
SDK selection and a provider-refunded flag. The subsequently authorized status
query associates the provider-listed origin transaction with that saved receipt,
as recorded below; amounts and fresh discovery remain unqualified.
Some legacy refund bindings are transparent, so their obligations remain preserved.
The status-query helper and frozen executable have independent source, complexity
and execution-method review, with four passing public fixtures including actual
CLI and direct-policy rejection. Its source SHA-256 is
`7bfdba32f14e6f3a50e168309d13cbeafd8d22cc43f97d97578cd643b01fe56c`;
frozen executable SHA-256
`763200d699c8a194c79f8f5c6cde8a0799ddc112f6f83a30843587ca3d576c78`.
Recipe `target/nu7-readiness/refund-status-inspection-recipe.md` SHA-256
`f9f9485fdfadf82a1af8c2fc117bbd7dcba02d1b8f107aa123f2ddeb1b2d83f2`
was approved for one selected existing-quote GET, with no credentials, retries or
additional endpoints. Network-only policy SHA-256
`f98d6d2079fd21ac5f0ab3eacc53bb9ac5299c0d7aefe9eb7d9cfd6bac9a989b`
exactly matches the selected deployment's Tor section. After explicit operator
approval, the exact frozen executable ran once with cleared environment, verified
source/binary/input/policy hashes and fresh exclusive owner-only captures. It exited 0
with HTTP 200 and a subset-bound `REFUNDED` result. No retry, other endpoint,
credential, wallet/key read, owner, sync, signing or spending occurred.

The sole reported origin-chain transaction matches the saved Ironwood note at the
original bound shielded receiver. The note is confirmed and SDK-selected in the
saved snapshot. Deduplicating all retained pool/transaction/output identities and
grouping that transaction/receiver still yields an amount larger than the provider
reports as refunded. Only one retained binding matches this receiver/deposit, and
no saved local outgoing identity matches the note transaction. These comparisons
do not establish an external sender. The [official status type](https://raw.githubusercontent.com/defuse-protocol/one-click-sdk-typescript/main/src/models/SwapDetails.ts)
describes the refund amount as transferred to the refund address, while its origin
hashes have no dedicated refund role. No reviewed fee breakdown explains the
difference; fees, batching or gross/net conventions are not assumed.

The evidence associates the transaction with the shielded receipt; its amount
remains unreconciled. Provider totals do not override SDK note facts or become a spendability gate. The saved
selection still does not prove fresh discovery, live unreserved funds or spending
authority. Private captures and exact integer comparisons remain under ignored
owner-only storage. Further private network queries are not authorized by this
consumed one-request grant. N3/N4 deployed semantics, N5 accounting/discovery
qualification and N6 retirement stay open. Native consensus, complete real-Tor,
new funded and reproducible-build qualification are not claimed.

Public unfunded dry probes through the same configured Tor policy return HTTP201
for both the current all-shielded UA and Orchard-only protocol UA at $2 nominal
USDC. Exact refund-address/public-recipient/dry echoes agree; neither response
allocates a deposit address or returned deadline. Initial $1 probes return HTTP400
with an amount-below-bridge-minimum result, not an established address-format
rejection. `public-refund-format-results.json` records the source/log identities.
These are format acceptance and bounded configured-transport observations, not
funded delivery, deployed receiver selection, watcher timing, settlement or full
Tor qualification. No production bridge change is justified by these probes.

### Bridge evidence and qualification status

N3/N4 public-source assessment found no demonstrated production bridge-code gap.
Maintained network guidance now distinguishes requested refund-start preference,
returned address deadline and transaction confirmation; current public sources
still do not establish the deployed ZEC watcher's arrival timestamp, confirmation
requirement, late-deposit outcome, refund minimum or unused-request terminalization.
The live reference now retains the phased shielded-refund evidence recipe, including
exact immutable fields and missing per-note inspection/submission-disabled scan
capabilities. Ordinary quote-input equality remains intact. No funded test is authorized/executed.
Bounded production evidence inspection and then one separate Tor status observation
were approved and executed as recorded in the current checkpoint above.
The operator selected `examples/deployments/research_desk.toml`; its treasury path
resolves to `state/research_desk`, with no explicit treasury UUID. Other wallet
directories are outside this scope and may have unsupported schemas. Initial
source-only review established that existing CLI
views lack per-note evidence. C4 implements `wallet notes` using direct offline
LightWallet decoding and the existing authenticated read-only snapshot contract;
production compilation, exact-SHA source review and default/three-pool public
fixture execution passed.
It reports saved SDK selection, not live unreserved funds or spending authority;
copy-scan and any funded excess harness remain separate missing capabilities.
Next action: independently review a narrow offline projection of retained numeric
fee/transaction facts if it can explain the discrepancy; otherwise prepare a
separately authorized provider clarification of this specific accounting/transaction
role. Do not repeat the consumed status query or infer a fee adjustment.
Submission-disabled copy-scan can qualify discovery but cannot explain provider
accounting; it and any funded qualification remain separate missing capabilities
and authority.

### C5a operator-directed discovery and refund policy

The operator now explicitly requests disabling transparent refund use and transparent
recovery scanning, accepting shielded-refund support as the working assumption.
This supersedes the proposed explicit legacy discovery mode for this narrower step;
full removal of SDK transparent key derivation/output recognition remains a separate
N6 boundary. No funded experiment, production state mutation or further private
network request is authorized by this source change.

Starting snapshot: `e0b70223a3b2d32bdd4c15fe3d8a2c8f229ebf58`. Independent read-only
design review verifies the pinned SDK and selects supported settings overrides,
without new vendoring or schema conversion. Generated/imported wallets persist
transparent discovery disabled. Runtime open and SDK reconstruction override saved
discovery settings; all known-address queries and gap recovery are disabled.
Read-only evidence keeps original saved facts. Current storage already accepts only
schema23; dependency-owned historical SDK snapshot parsing is not an application
pre-seed migration path and will not be forked for this settings change.

Enforce shielded-only Orchard-capable unified refunds before new outbound quote
requests and before new managed financial input acceptance, including cached quotes.
The existing Store binding check follows accepted-work detection, so accepted
calculations, signed bytes and status/recovery can finish. New-work quote validators
reject transparent/mixed receivers; ordinary journal deserialization retains exact
old bindings without a permissive historical refund validator.
Do not silently replace a previously disclosed transparent/mixed binding or rewrite
its quote. Existing owned-fund recognition/shielding does not enable address recovery.
No separate legacy mode, recovery ledger, timer or extra state is introduced.

History check at `d0e05a288b78be5e2dffeaaf6c7491b74c074e04` confirms new refunds
were already shielded, and the pruning plan explicitly left current refund
ownership plus dependency-owned snapshot codecs intact. The approved saved-state
inspection shows no outstanding transparent source transaction: associated current
jobs are complete and retained source transactions confirmed. Do not infer an
active liability merely from the presence of a historical transparent binding.
Generic accepted-work journaling remains shared infrastructure, not a new
transparent compatibility mode.
The P4/C06 linkability finding removed with
`556865ab42e93758f70324aff7c4ee5570f3b15a` originally disabled probing only for
newly generated seeds and retained import/reopen discovery. The current operator
direction removes that exception: no known-address queries or gap probing on
imports, reopening or recovery either. This is independent of the outstanding
provider amount discrepancy and requires no new funded qualification first.

Commit boundary C5a includes these policies, focused negative-RPC/new-request/
cached-preparation regressions, seed-only shielded discovery/reopen coverage, and
owning wallet/SDK/CLI/network guidance. Independent exact-SHA and combined review
must include all affected call chains and source-matched CogC/CycC. Complete compiler
and all-target Clippy before broad suites, reuse unchanged dependency/security
coverage, then run affected default/SDK and synthetic lifecycle consumers with
sources frozen. Record actual counts and limitations; no live qualification claim.

### N6 full-retirement assessment (beyond C5a discovery policy)

Independent read-only audit of v8 `81d799775f11e882896e8710e8af12d6d4d29a41`
found no complete transparent-retirement configuration. Zingolib
`wallet.rs::LightWallet::new` unconditionally derives/stores external taddr zero;
older wallet decoding can derive it too. Pepper disabled discovery returns before
known-address RPCs/gap probing but full/pending scan paths still recognize owned
transparent outputs, and compact scanning can process them after discovery's floor.
Ordinary serving still shields eligible already-owned transparent outputs. C5a
disables address recovery and new transparent refund use; it does not remove SDK
key derivation or incidental output recognition, so N6's original full-retirement
acceptance remains separate.

Technical next action: assess a supported upstream no-transparent construction/
recognition option or narrowly reviewed SDK source change preserving original
accepted work and owned funds rather than clearing address maps. The operator's
C5a direction supersedes the earlier proposal for a legacy discovery mode. A new Zingolib checkout requires the
vendoring discussion; Pepper alone cannot fix constructor derivation. Acceptance
must separately inspect internal maps, known/gap RPCs, compact/full/pending/calculated
recognition, fresh/import/reopen/seed+snapshot restore and original late obligations.
Existing shielded fresh/RPC and seed-refund tests are useful but insufficient.

The approved bounded selected-state inventory now includes current/archived refund
bindings, saved original outgoing identities and shielded-note facts. It does not
qualify live legacy transparent funds, late obligations or deployed refund delivery.
Next action: complete provider attribution and separately review submission-disabled
copy-scan/legacy recovery evidence before selecting any SDK retirement patch.
N3/N5 deployed receiver/pool/spendability evidence remains a full-retirement
release condition; N6 is explicitly incomplete. Current
SDK limitations are maintained in the integration guide, without claiming that
query isolation or disabled discovery retires transparent support.

### Earlier C2 implementation status

C2 `469179d8` implements the supported libsecp256k1 secret-signing path and
source-aware offline advisory replay. Independent security assessment identified
actual secret arithmetic in the previous k256 path and the SDK's public
`SignerLike` adapter as the smallest migration. No dependency checkout was added.
Four mocked advisory-tool tests pass. Final-source pinned-database replay,
compiler/Clippy and signature/payment execution subsequently completed as recorded
in the current checkpoint. Exact-SHA
review found missing owned-key erasure in Alloy's opaque native credential and
possible suppressed informational findings in advisory replay. Independently
reviewed fixup `8a7b983e` uses direct native key ownership with in-place best-effort
erasure and complete advisory settings/warning validation. Fixup `1a272ba9`
accepts only Cargo's root `.cargo-ok` cache marker while still rejecting changed
or extra Git source; review requested focused negative coverage, now added.
The full offline assessment uses cargo-audit 0.22.2 and RustSec
`26fd2d43f7280a1fb925a3743ec463017afe7119`: root has eight vulnerability
matches (mitigated crypto-bigint secret-signing path and seven build-only legacy
TLS matches) plus five maintenance notices; protoc and normalized nonregistry
inventory have no matches. Artifacts are under ignored
`target/nu7-readiness/security-assessment/`; the final-source replay is recorded
above under `security-assessment-after73df/`. Permanent exposure dispositions are in
`docs/development.md`; the report remains findings rather than clean. Cargo's compatible update retains `generic-array` 0.14.7
because `crypto-common` 0.1.7 requires it exactly; forcing 0.14.9 is incompatible.

### Added timing scope

The operator additionally requests review of Zcash polling, expiry, confirmations,
reorg/rescan timing and faster swap resolution, including a requested sub-hour
window. Independent design review selected a separate timing commit after C1/C2:
healthy demanded scans follow SDK target spacing at the next established height;
source/status success follows configured polling without the universal minute
floor; Base credit cadence and independent failure backoff remain separate.
The quote preference default becomes 1800 seconds, retaining overrides, returned
deadlines and accepted bytes. Confirmation counts and SDK finite expiry remain
unchanged: 40 pre-NU7 / 120 NU7 blocks each target about 50 minutes. None of these
changes manufactures bridge terminal outcomes or guarantees sub-hour abandonment.
Update wallet/configuration guides and both timing/resource inventories; test
activation, completion-based cadence, independent backoff and retained ownership.

### C1 review and validation

C1 `69d7d7cd85b7d212b6751bc0c25d498c827e2d9d` independently reviewed.
Material findings: removed mempool timeout argument, removed SDK diagnostic
variants/new panic payload, and changed shard-tree checkpoint fixture contract.
Fixup adapts the adapter/test signature, sanitizes `SyncPanicked`, and uses a real
one-block merge fixture with prior empty checkpoints in all pools and no RPC.
Also corrects copied-code origin wording and upstream clear-test expectation.
Vendor reconstruction passes for all 35 exact upstream files. Full compiler,
Clippy and applicable runtime evidence subsequently completed; refreshed application
and partial vendor complexity evidence is recorded above.
The independent manual review examined Pepper scan ordering/drain, checkpoints, conservative
recovery, reorg retention, offline/send/expiry and original-work ownership.

## Ordered assessment boundaries

| ID | Work and required exit evidence |
| --- | --- |
| N1 — activation/cohort and general Rust updates | Independently assess actual NU7 activation and update the Rust dependency graph generally, including application, build, development and workspace/tooling dependencies and standalone tool manifests/lockfiles. Assess released Zingolib v8.0.0 and its compatible Pepper, Zcash backend/crypto/proofs and indexer cohort as the upgrade candidate. Resolve related direct/transitive constraints together rather than change only the Zingolib revision. Review signing, receive/decrypt, sync/reorg, migration, transaction serialization, fee, expiry, source confirmation and recovery consumers across activation. Record selected versions, retained pins and concrete incompatibility/blocker decisions; blanket unpinning or forcing every crate to its newest incompatible release is not acceptance. Preserve every existing vendor blocker and provenance check. No fixed legacy block interval or application-maintained expiry algorithm. |
| N2 — B1 security updates | Reassess the whole locked production dependency graph, including path-vendored sources. Explicitly resolve the documented `crypto-bigint` signing-cohort advisory by a compatible migration, separately reviewed backport, or concrete exposure analysis and supported release disposition. A Zcash/NU7 upgrade does not automatically fix an EVM signing advisory. Preserve exact EIP-712/address/signature and transaction/state contracts; no incompatible forced manifest override or new vendoring without its required discussion. Establish reproducible local advisory evaluation for exact source versions even while Linux CI automation remains deferred. |
| N3 — refund format and receiver selection | Determine the deployed native-ZEC/Base-USDC route's support for shielded refunds and how it selects the actual receiver. Recheck dry quotes with public unfunded shielded-only refund UAs, including the exact receiver set generated by the selected SDK. Distinguish shielded-only UAs from mixed UAs that can receive transparently. SDK tests, bridge-source/Zodl precedent and historical dry acceptance are useful distinct evidence, not proof of deployed funded refund delivery. Preserve transparent source-deposit validation and existing quote/refund bindings; do not silently fall back to transparent refunds. |
| N4 — R7 service contract | Obtain current Zcash-specific 1Click/bridge semantics for deposit first observation, inclusion/confirmations, returned deadline versus requested preference, late deposits, refund fees/minimums, and unresolved unused `PENDING_DEPOSIT` requests. Identify terminal outcomes and transaction evidence; absence of an EXPIRED status cannot be replaced by a local timer. Map the contract to current source/credit/status/retirement ownership and repair only demonstrated gaps. |
| N5 — live qualification | Prepare an exact reviewable unfunded qualification recipe, then obtain the existing separate operator authority before spending. First inspect any suitable retained evidence; if it does not establish actual shielded refund delivery and SDK spendability for the selected route/cohort, qualify a controlled refund. Assess deliberate second funding of the same exact-output quote as the candidate trigger described below, without assuming it guarantees a refund. Retain exact UA receiver set, request/returned quote, every source identity/txid, provider outcome, refund transaction and SDK discovery/spendability. Test selected near-deadline/late-arrival cases only within separately reviewed authority. Keep private evidence out of Git. |
| N6 — retire wallet-owned transparent support | Use N3's deployed receiver contract and N5's actual refund evidence to establish whether this upgrade can fully disable wallet-owned transparent address generation/publication, refund selection, discovery/scanning and shielding. Audit fresh wallets, imports, reopened snapshots, SDK defaults, CLI paths and recovery consumers. Select the smallest supported SDK configuration or code removal that achieves shielded-only operation; improving isolation of transparent queries is not the objective. Preserve accepted refund bindings, original signed work and recoverable owned funds through an explicit migration/recovery path for legacy transparent obligations. Exit with implemented retirement and focused acceptance evidence, or concrete blockers and remaining work; accepting shared-indexer clustering does not complete this track. |

N1 and N2 inform any coherent dependency update; N3 and N4 inform bridge behavior
and N5's test cases. The requested half-hour quote preference never
rewrites old quotes or signed bytes and is not a guaranteed refund completion time.
SDK block expiry and the provider's returned wall deadline remain separate.

The published [Zingolib v8.0.0 release](https://github.com/zingolabs/zingolib/releases/tag/zingolib_v8.0.0)
is a candidate, not an already-qualified replacement. Its release notes describe
the NU7 Zcash cohort, Pepper 0.7.0, changed reorg retention and one-session indexer
APIs. Inspect the exact tagged sources and resolved revisions against every
retained adapter/patch and consumer; a larger upstream rollback window alone does
not establish that the local reorg correction or entire Pepper checkout can retire.
Include dependency migration changes and source-matched compatibility, network,
SDK, financial lifecycle and vendor-integrity evidence in the implementation
boundaries. N2's advisory disposition covers the resulting complete graph; a
Zcash update does not automatically fix the separate EVM signing cohort.
Include `tools/complexity` in the root workspace graph and `tools/protoc`'s
independently locked graph. Assess path-vendored versions and patch consumers
explicitly; provenance-only vendor lockfiles are not the application's active
dependency resolution and must not be regenerated as though they were.

N1's selected SDK/indexer cohort informs N6's retirement design; N3 and N5 establish
whether deployed refunds can avoid transparent receivers. Current new swaps already
derive Sapling+Orchard unified refund addresses without a transparent receiver,
while existing bindings are reused unchanged. Fresh wallets disable transparent
discovery, but imported/reopened settings and legacy recovery still support it.
These facts do not establish actual deployed refund delivery or complete retirement.

N6 supersedes the earlier per-address isolation assessment. Its goal is removal of
wallet-owned transparent use and scanning, not a different circuit for each query.
Paying a bridge-owned transparent source-deposit address from shielded funds is a
separate outbound contract: it does not require a treasury-owned transparent refund
receiver or address scan. Identify any actual SDK dependency before removing
transparent primitives needed to construct or validate those outbound payments.

Before removing legacy handling, identify disclosed transparent/mixed refund
bindings, possible late receipts, existing owned transparent funds and pending
shielding transactions using established state. Define a supported migration or
explicit recovery path that preserves their original identities and accepted
bytes without rewriting old quotes, deleting keys/evidence or abandoning funds.
Do not turn legacy compatibility into permanent ordinary transparent scanning by
default. If the deployed route or unresolved legacy obligations prevent complete
retirement, record the exact blocker and next action rather than claim completion
or substitute a query-isolation improvement.

N6 acceptance must demonstrate shielded refund discovery and spendability without
transparent scans on fresh, imported and reopened supported states, with negative
controls proving no transparent discovery RPCs or new transparent receive/refund
addresses are produced. Inspect SDK transparent key/address derivation separately
from published addresses, and known-address queries separately from gap probing;
disabling one does not demonstrate that the others are absent. Check recognition
of owned transparent outputs in otherwise wallet-relevant shielded transactions
as well; disabled discovery alone does not prove transparent support is retired.
Exercise restart, snapshot/seed recovery and the selected
legacy migration/recovery path, preserving accepted pending work and late-receipt
coverage. Distinguish any remaining offline legacy recovery from ordinary serving.
N5 must retain evidence of the actual refund receiver/pool and SDK-observed funds;
a UA string, accepted quote or successful forward swap alone does not qualify
refund delivery. Funded evidence retains its separate operator authorization.

## Controlled refund qualification candidate

First inspect existing retained swap/refund evidence: the operator reports that
one of the existing wallets may already hold a shielded refund from the earlier
duplicate-deposit experiment. Identify the actual wallet and bind its saved quote,
refund receiver, source transactions and returned funds before proposing another
funded test. Prefer existing evidence when it establishes the actual deployed
receiver/pool, attribution and SDK spendability. A historical refund need not be
re-created solely because dependencies change: separately qualify discovery and
recovery of that receipt with the selected upgraded SDK and transparent scanning
disabled. Document missing evidence or relevant service/receiver changes before
deciding that a new funded refund is needed. Preserve original state and private
evidence; any production-key access still requires the reviewed inspection recipe.

The operator reports that a second deposit using one exact-output quote previously
caused a refund. Treat this as a useful experiment to reproduce, not a guaranteed
service contract. N4 must assess whether that route accepts quote reuse,
how it accounts for additional deposits, which refund binding it uses and how
status/transaction evidence distinguishes the two deposits. Record unresolved
semantics as experimental questions in the reviewed recipe rather than claim a
guaranteed refund. The experiment is not
an automatic retry, nor may it be implemented by changing ordinary funding recovery
to send another deposit after an uncertain outcome.

Prepare a concrete recipe using `research_desk` or another operator-selected
treasury wallet with sufficient SDK-confirmed spendable ZEC. Confirm the actual
wallet/state mapping before selecting it; do not infer a ZEC treasury from an EVM
wallet label. Record the exact executable/cohort, state owner, route, shielded-only
refund UA and decoded receivers, USDC recipient, quote mode/identity, returned
deadline, amounts, network/refund fees and the intended deposit count. Review the
two proposed transfers and their financial authority before loading production
keys or spending. No separate permanent ZEC cap or ordinary-funding budget is
introduced by this qualification recipe.

Identify an execution path that can represent two intentionally authorized source
transactions against one quote without violating existing job/operation bindings.
Current allocation permits do not automatically authorize an extra deposit to a
completed quote. If a focused production-owner harness or CLI change is necessary,
design, implement and independently review it before execution; do not add a
parallel financial ledger or weaken normal recovery to make the test possible.

Use one exact-output quote and, if the reviewed recipe supports the method,
observe the first deposit's swap outcome before deliberately making the separately
authorized second deposit to its deposit address. Recheck whether the saved quote
is still usable at that boundary; do not invent an extension or replace it with
a fresh quote while claiming to have tested reuse. Journal each transfer's intent,
inputs, exact signed bytes and identity before disclosure through the existing
treasury owner. Do not rebroadcast the first transaction to simulate a second
deposit or bypass exclusive Store/SDK ownership with an ad hoc signing script.
If either outcome is unknown, retain its liability and observe/reconcile it;
absence of a result grants no repeat transfer or additional refund experiment.

Retain both source transactions, the first swap's actual USDC outcome and all
provider status/refund evidence, including any unexpected second swap or pending
deposit. A provider `REFUNDED` label alone is insufficient: establish the returned
ZEC transaction and the SDK-decrypted note's actual pool, amount, confirmation and
spendability at the bound shielded refund receiver, with no transparent receipt
or shielding transaction used as a substitute. Correlate the received funds with
the experiment without exposing keys or private financial evidence in Git.
Demonstrate the selected upgraded SDK can discover and recover the refund without
transparent address scans, including restart/recovery evidence required by N6.
Report a pending, failed or unsupported trigger as incomplete qualification rather
than silently buying another attempt. Use the existing live runbooks and record
any purpose-built recipe gaps before execution; the general provider runner is
not presumed to already support duplicate Zcash deposits.

Before implementation, revise these boundaries into coherent ordered commits for
the actual selected cohort/contract, with owning documentation and tests in each.
Have an independent read-only design reviewer challenge necessity and simpler
alternatives before affected code. Require non-author exact-SHA/fixup reviews,
unsquashed history and source-matched CogC/CycC review under
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline). At the dependency/
bridge gate and final integration, inspect all preceding commits and fixups through
full signing, network, financial and recovery chains; analyze repeated assumptions
and testing gaps and resolve material findings before dependent work.

Use ordered production/compiler/all-target Clippy, compatibility, synthetic lifecycle
and affected optimized-profile checks before broad/optional qualification. Native
consensus, live-funded, real-Tor and reproducible-build claims need their actual
runbooks and evidence, not fixture watchdogs or a newer version number.

## Completion and handoff

Keep actual consensus/SDK behavior in SDK and wallet guides, exact advisory status
and update constraints in development/vendor guidance, qualification scope in testing
and its runbooks, and current provider deadline semantics and the N6 shielded-only
support/migration contract in network/wallet guidance.
AGENTS.md already links these owners. Linux CI infrastructure remains in the
[deferred release plan](deferred/release_followups.md), not a hidden prerequisite
for resolving current dependency/security findings locally.

Unmet release conditions stay explicit in this plan with a concrete next action;
guides retain truthful concise limitations and links. After completion and verified
handoff, remove the plan without an archive; independently review the cleanup SHA
and final combined branch, including previous fixups and financial evidence handling.
