# Zingo integration and Pepper patches

The application and its SDK integration tests use ordinary upstream `zingolib` and
`zingo-netutils` Git sources at revision
`81d799775f11e882896e8710e8af12d6d4d29a41` (v8.0.0). Neither crate has a local
snapshot or override. The same cohort's `pepper-sync/` remains patched; the shared
workspace retains its path override. Pepper is excluded from workspace test
selection; its merge regression runs through public traits in the application
SDK integration target.

`pepper-sync-sources.json` records the upstream source archive hash, every retained
Pepper snapshot file, its ordered patches, original upstream file hashes and the
copied-code license notice.
`python3 vendor/verify_pepper.py` verifies those files and rejects unexpected Pepper
files. It reverses all patches in a temporary copy and compares the complete
restored tree with the pinned upstream Git source.
`pepper-sync-progress.patch` contains only Pepper's `client.rs` and `scan/task.rs`
changes; `pepper-sync-diagnostics.patch` covers `client/fetch.rs`, `error.rs`,
`sync.rs`, `wallet/traits.rs` and `witness.rs`. `pepper-sync-reorg.patch` keeps
the upstream 1000-block rolling rollback/checkpoint/transparent-discovery horizon
and repairs derived chain state when retained rollback boundaries are unavailable. The payload accepted by the public
sync trait has a public re-export and `from_parts` constructor; the witness module
and payload fields remain private. `pepper-sync-manifest.patch` records the
standalone manifest adaptations necessary for the path dependency; upstream
unit-test dependencies are retained
but are not selected by application checks. Never edit Cargo caches or
reference checkouts to implement a dependency patch. The
[checkout ledger](../vendor/README.md#checkout-retirement-tracking) owns remaining blockers
and candidate qualification.

The stable cohort uses `zcash_client_backend` 0.25.0, `zcash_primitives` and
`zcash_proofs` 0.31.0, `zcash_protocol` 0.11.0 and `zcash_keys` 0.17.0,
with Pepper 0.7.0 and netutils 7.0.0. Its mainnet schedule activates NU7 at
height 3,543,405 and branch ID `0x77190ad9`. The ordinary `sdk_compat`
mainnet regression checks the schedule, branch transition and SDK expiry on
both sides of activation; synthetic activation overrides alone cannot establish
mainnet readiness. SDK selection of those parameters does not qualify a deployed
indexer, transaction submission, consensus execution or funded settlement.

The v8 one-session online API does not change application ownership: the
LightClient stays offline and the application supplies the identity-bound indexer
to Pepper. The port retains upstream's previous-session-height reorg comparison,
as well as local missing-boundary and conservative-clear recovery. A larger
retention window cannot prove that shorter endpoints are divergent or replace
recovery outside retained history.

## Adapted application code and attribution

The network adapter retains an implementation adapted from the former v7 revision
`bfb0a2156819ea7f75a9b09b1724abfd06c8925e`; the current v8 cohort is assessed
against that retained adapter:

| Application code | Upstream origin | Purpose of the local adapter |
| --- | --- | --- |
| [`src/network/indexer.rs`](../src/network/indexer.rs) | `zingo-netutils/src/lib.rs`: `GrpcIndexer` and its `Indexer` implementation; `zingo-netutils/src/globally_public.rs`: its `TransparentIndexer` implementation | Thirteen indexer RPC delegations use the application's identity-bound SOCKS/progress transport. The six required transparent-indexer methods refuse locally without constructing a request. Upstream's private client uses tonic `Channel`; the application owns network construction, privacy authority and inactivity handling. |

The network adapter retains upstream copyright and revision attribution in its
source header. [`vendor/ZINGO-LICENSE-MIT`](../vendor/ZINGO-LICENSE-MIT) preserves the
upstream copyright and permission notice for these adapted implementations.
The notice matches the pinned upstream repository's root `LICENSE`. Its MIT terms
require retaining that notice with copies or substantial portions of the code.
These application implementations remain after removal of the Zingolib/netutils
vendor checkouts, so the notice remains relevant independently of Pepper's
checkout. Calls into ordinary upstream SDK dependencies are separate from these
locally adapted implementations.

## Ironwood-only policy

Ironwood is the application's only supported Zcash shielded pool. New treasury
inputs, native payments, change, receive requests and refunds use Ironwood. Reject
Sapling and legacy Orchard transfers, pool fallback and historical-fund support;
unsupported balances cannot become spendable capital or a reason to wait for new
funding. This applies to ordinary funding, direct tools and wallet administration.
There is no operator or agent pool selector that re-enables another pool.

Ironwood uses the Orchard **receiver format**. New addresses are unified addresses
containing only that receiver, without Sapling or transparent receivers. Its
encoding does not authorize legacy Orchard-pool payments. The SDK proposal must
select Ironwood before input ownership is accepted, and calculated transaction
validation checks actual Ironwood outputs and fees. Use the public backend's
Ironwood-only spending policy instead of LightClient's unrestricted selection.

The only external transparent-output exception is the exact NEAR-issued deposit
address bound to an accepted managed/direct swap quote, paid from Ironwood funds.
It does not authorize arbitrary transparent destinations, treasury transparent
receiving, transparent-input shielding or address-indexed RPC queries. All six
required `TransparentIndexer` methods fail locally, and wallet discovery remains
disabled across creation, import, reopen and reconstruction. Ordinary transaction
lookup/broadcast and compact-block synchronization retain their separate roles.

The application has no Sapling parameter parser, initialized prover, runtime
parameter cache or parameter downloader. Backend transaction APIs unavoidably
require Sapling prover traits: a parameter-free refusing adapter supplies those
types, with an uninhabited proof result. Proposal restrictions prevent normal calls;
any unexpected proof request fails closed rather than signing unsupported work.

The pinned SDK still derives unused keys, decodes global chain/old wallet formats
and can recognize unsupported outputs while scanning. Raw SDK snapshots and
accepted transaction/input journals remain exact; decoding is not permission for
fresh preparation, replacement or disclosure. Application balances and note
inspection project Ironwood only. No application legacy-pool transfer or recovery
implementation is retained for new work.

One build-only exception is unavoidable with the unchanged pinned SDK: Zingolib
`81d7997` calls `get_zcash_params` unconditionally in `build.rs`, and embeds Sapling
parameter bytes in `lib.rs`. It exposes no feature to disable this. An uncached SDK
build can therefore acquire them even though the application never initializes or
uses them. Do not add a maintained dependency checkout/fork to conceal this fact;
see [build-time parameters](#build-time-parameter-downloads) and the dependency
policy for any separately proposed upstream remedy. Existing parameter-cache
integrity and reproducibility checks remain necessary for the SDK build's inputs.

## Application SDK boundaries

LightClient is created/restored offline and stays offline. The application does
not use SDK online constructors, URI setters, sync launching, migration
transmitters, price fetchers or optional nym workers.

`src/network.rs` alone constructs the generated gRPC client over the existing
progress transport and returns `network::GrpcIndexer`. The thirteen `Indexer`
methods in `src/network/indexer.rs` retain generated request defaults, concrete
streams, clone ownership and request metadata. Its six required transparent-address
methods return a fixed local unsupported error without transport activity. Pepper
still calls deprecated aliases. Nonzero submission codes produce
`Status::Unknown` with the original message; successful transaction IDs retain
safe quote stripping. Submission validates the result against the durable exact
transaction. The adapter ignores the traits' legacy total-timeout arguments. The transport
removes any total `grpc-timeout` metadata and measures response inactivity below
protobuf decoding. Identity, authenticated SOCKS remote
DNS and no-direct-fallback rules remain in [network policy](network-egress.md).
`ping-very-insecure` and nym capabilities are not enabled consumers.

`src/treasury/mod.rs` runs one public Pepper engine task inside the existing
isolated session runtime, with local mode/progress state and finite
`shutdown_on_completion=true` configuration for current saved snapshots.
Pepper can hold the wallet write lock across asynchronous recovery fetches; the
engine must remain independently polled while encrypted checkpointing waits for
that lock. Cancellation destroys the runtime and joins blocking workers before
the final snapshot. There is no SDK online handle, first-poll handshake or second
controller to maintain. Successful scans call public `capture_migration_witnesses`
before final tip validation. Both join boundaries discard panic payloads into a
fixed diagnostic. `tests/treasury_sync.rs` covers the actual lock-held recovery
interleaving, periodic/final snapshots, cancellation, teardown and reopen.

`src/treasury/send.rs` retains the exact public proposal through durable input
acceptance and calls backend `create_proposed_transactions` by reference. It uses
the SDK's original account key, system randomness, clock and sender OVK policy,
with explicit `Some(wallet::expiry::tx_expiry_height(chain,
proposal.min_target_height().into()))` for ordinary NU7-aware expiry. Leaving the
high-level SDK calculation offline would instead choose epoch-end expiry.
Public `clear_proposal` runs after releasing the wallet guard; no private proposal
manager or parallel input ledger is needed. The actor reuses canonical validation
without a redundant connection; direct preparation retains network validation.
The application selects Ironwood inputs and change through the public backend API.
An empty change memo replaces the SDK's private advisory full-UA serializer; exact
accepted terms and actual note receivers own financial attribution. No additional
memo dependency or parallel proposal manager is needed. The required Sapling prover
adapter refuses operations without loading parameter bytes.
Financial authority, retained bytes, publication and recovery remain in [wallet ownership](wallet-rotation.md) and
[lifecycle coverage](../tests/LIFECYCLE.md).

## Remaining Pepper responsibilities

The progress patch removes complete-message timers on sync streams and retains
loader/worker ownership while draining, with periodic stage diagnostics instead
of forced timeout success. Header/body inactivity remains in the application
transport. Resource-related gRPC codes (`OutOfRange` and `ResourceExhausted`)
emit safe WARNs at the existing stream-consumption points before recovery can
replace the original status. The application adapter reports the same codes for
unary, stream-opening and mempool retry failures.
These statuses may originate at the peer or the local decoder:
logs contain fixed stages and numeric codes, never upstream prose or metadata.
Read retries, protobuf size bounds and mempool drain contracts remain unchanged. Remove these hunks only with equivalent progressing/stalled response,
cancellation/restart and worker-drain evidence through the full sync chain.

The diagnostic patch retains typed shielded-pool and operation context for actual
scan merge, reorg rollback and pool-recovery rollback failures. Recovery
recommendations are unchanged. The application extracts fixed labels and discards
underlying errors before logs/status; no tree positions, hashes or upstream prose
are exposed. Fetch spans are debug-only; mempool events contain fixed labels and
numeric status codes. Rollback, automatic pool reset and mempool retry diagnostics
stay at DEBUG while recovery remains available; the owner reports the final
capability outcome. Replace these changes only with equivalent safe context and
instrumentation. The three-pool merge-conflict fixture remains executable in the
application SDK integration target without a separate SDK unit-test build:

```sh
make test CARGO_ARGS='--test sdk_compat' TEST_ARGS='sdk_merge::scan_merge_conflict_preserves_pool_context --exact'
```

See [workspace verification](development.md#combined-workspace-verification).
Upstream owns same-height/deeper reorg truncation and synthetic branch hashes.
Recovery depth uses the previous session's chain height; the application removes
obsolete mutable verification-start accumulation from that path.
The upstream 1000-block rolling horizon is a cache-retention policy, not consensus
finality. A node's larger rollback window does not require matching local retention. A demonstrated divergent chain outside the retained horizon clears derived SDK
chain state and resumes from the effective scan floor. A shorter height alone
cannot establish divergence. The SDK protects retained block metadata,
scanned-priority ranges (including outstanding nullifier retrieval) and confirmed
transactions; an old unsigned scan target is not a protected observation. If
protected facts do not exceed the endpoint tip, the unscanned scheduled tail
is shortened without lowering the floor, and cached shard-completion ranges beyond
that tip are removed. Those scheduling projections cannot require a current note to
wait for future blocks. Actual tree leaves, cached roots and checkpoint witnesses
remain intact. The same normalization follows root ingestion, whose response can
advance beyond the cycle's observed tip; it also repairs saved equal-tip state
before later network failures can leave local SDK selection blocked. If protected
facts exceed the tip, the SDK compares a retained block at a height served by the
endpoint. The response must bind that exact height to a complete block identity using the
SDK's existing projection: a valid full header owns its computed hash; otherwise
the compact hash must contain exactly 32 bytes. A missing or conflicting compact
hash projection cannot override a valid header.
Mismatch permits existing recovery; matching or absent overlap preserves wallet
observations and reports a recoverable sync failure. Failed or malformed reads
also preserve them. These endpoint-history checks are not consensus validation.
The wait is inherently ambiguous: birthday100/tip50 has no overlap; an anchor100
can match a tip150 fork beginning120. A later healthy endpoint or a retained
comparison that actually differs enables progress, without a timer, quorum,
watermark ledger or expanded retention horizon. Missing older shard checkpoints use upstream's existing
clear-and-rescan fallback; older snapshots need not contain additional checkpoints.
The same fallback handles a pruned checkpoint or required wallet-block boundary
that rollback can no longer reconstruct locally. Startup also repairs persisted
scanned ranges with missing boundary blocks, including interrupted older rollbacks.
New-chain metadata is not attached to an unverified old scanned prefix.
A full clear without a proven common ancestor starts at supported Sapling
activation, even when the replacement tip is above the declared birthday. Clamping
to the replacement tip or old birthday could miss replacement credits already
mined below both. A fresh birthday hint beyond the observed tip uses the same conservative rescan
only when no retained block, scanned range or confirmed transaction would be
destroyed. An interrupted scheduled target alone does not establish history. Existing persisted scan ranges retain the earlier effective
floor through clearing, interruption and reopen; the declared birthday, keys and
original operation journals remain unchanged. A chain below supported activation
still reports a diagnostic rather than inventing usable history. This uses the
existing scan owner and ranges, with no additional recovery ledger or deadline.
The treasury retains keys, address derivation, accepted input claims, original
signed bytes and financial journals through either rescan. Sync readiness and
reuse are rebuilt by the existing scan owner; clearing chain state grants no
submission, release or new deposit authority. Application lifecycle tests cover
rollback beyond the rolling window
while retaining exact pending work. The patched SDK fixture
`future_birthday_rescans_supported_history_and_keeps_floor_across_clear` also
checks the activation-floor fallback with unchanged declared metadata before
the first resumed block is scanned. Mutable input/inclusion/confirmation
observations wait for canonical progress; malformed immutable transaction
identities require scoped inspection. Reorgs can move a Sapling note and change
its current position-derived nullifier. The SDK reconstructs those current inputs;
the application does not compare them with historical inputs to authorize expiry.
Confirmed source work continues the existing swap
observation rather than requesting manual review. A completed SDK observation of
the transaction's computed identity at configured depth proves confirmation without
a second indexer report. [ZIP 244](https://zips.z.cash/zip-0244) separates v5
transaction effects from authorization data: a valid authorization-data variant
can have the same transaction ID while its serialized bytes differ. Canonical SDK
matching therefore uses computed transaction identity/effects, while original
journaled bytes, transaction ID, expiry and input bindings remain strict. Legacy
v4 IDs commit to the whole transaction encoding. This comparison never replaces
the original disclosed bytes or grants replay authority.
Local broadcast counters are not part of this immutable transaction identity.
Malformed counters or unknown source-status labels conservatively retain possible
disclosure while canonical observation continues; they neither authorize another
submission nor veto an otherwise established confirmation or finite expiry.
For unfinished source expiry, the application validates the authenticated original
transaction and signed finite expiry, then uses a completed canonical SDK tip at
the configured depth. It does not reconstruct old SDK inputs or require current
unspent/alternative-spender proofs. Pepper already expires pending originals and
clears only their own spend links across all supported pools; confirmed competing
spenders remain spent. Actual SDK spendability, bridge terminal outcome and new
funding authority remain separate. Verified finite expiry can release an attempt
under current authority without waiting for a negative bridge outcome; the old
accepted request retains exact-operation status observation, and its later SUCCESS
prevents an undisclosed replacement. Application wallet serialization coverage
uses current SDK format 45. The existing `sdk_compat` offline roundtrip inserts a
public scan target, checks that it changes serialized sync state, saves and reopens
that exact state, and preserves addresses and next-address derivation. The
`treasury` encrypted roundtrip separately preserves wallet addresses and pool keys
through application storage. Near refund tests use one public unified address with
Orchard and Sapling receivers, independent of wallet snapshot serialization.
Dependency-owned historical codecs remain upstream implementation; application
fixture coverage does not require dependency patches. See
[wallet administration](wallet-cli.md) for independent storage/output versions and
backup obligations. Server protocol 0.5.0 capability requirements are distinct from
the Rust protocol crate version.

SDK calculation is offline and runs in one started blocking owner with the actual
LightClient, accepted proposal and same Store queue. That owner accepts inputs,
calculates once, extracts exact bytes and retries only local publication. It survives
caller/Treasury drop and normal runtime teardown; no async publisher or nested
runtime is needed. LightWallet's public synchronous clear/mark_dirty/save produce
the existing snapshot codec. A surviving Treasury joins without removing the handle,
installs client/revision while unhealthy and then calls the public async client
clear to release its private pause guard. Entire-owner drop releases the real client
after publication. Calculation/save never use its temporary scratch wallet path.
Only an atomically committed outgoing journal can supply submission authority. Successful exclusive offline reconstruction, with no
live calculation or completed result awaiting publication, closes a funding calculation whose outgoing
and factual transaction journals are both absent. Existing recovery archival
preserves its encrypted accepted inputs; the next calculation needs a fresh
operation and current authority. The persisted `funding_quote_refreshes` table also
marks these proven never-disclosed resets for recovery-count exclusion; its legacy
name and existing quote-refresh rows remain compatible. Raw pending originals and
committed results with lost acknowledgments retain their owner. A durable commit
acknowledgment is separate from loading a pending submission view: verified bound
fulfillment may intentionally leave no pending view, and decoding failure cannot
make a known commit uncommitted. The former finishes preparation without a source
handoff; the latter leaves the SDK unhealthy for reconstruction. Neither repeats
calculation, discards original bytes or grants disclosure. Invalid calculation
facts still fail after publication. Background view-repair diagnostics remain
DEBUG; only a genuine unacknowledged publication retains the local retry cadence.
Calculation metadata alone does not create a global source-work fence. Full SDK clear can
discard pending transaction records and their spend links; the application
therefore retains the fence for actual unfinished signed originals until closure.
The synthetic `sdk_reorg_rescan_preserves_original_signed_work` regression covers
this gap without requiring lifetime tracking of completed source transactions.

The synthetic `indexer_recovery` owner tests separately keep an actually disclosed
original through stale-prefix failure, a pre-canceled scan request and reopen, then
finish it only after healthy canonical confirmation without another submission.
A proven deep replacement reconstructs a new owned credit below declared birthday
while the orphaned original input, accepted snapshot and signed bytes remain in
the pending Store journal. Recovered SDK capital is not permission to release
that liability or create another deposit; an unburied original still waits even
when the recovery allowance is zero. These local fixtures establish no consensus
or live bridge qualification. Existing in-flight sync cancellation tests cover
the sync-session owner; the pre-canceled request alone does not prove active-I/O
drain.

The application does not mirror SDK reorg history into completed swap workflows.
NEAR owns the outcome of a bound swap; a validated SUCCESS closes source work
without requiring a local CONFIRMED label. Current SDK state independently owns
spendability. Actual unfinished sources retain their exact journals and existing
observation/finite-expiry recovery owner. Production deposits use the SDK's finite
expiry (40 blocks before NU7, 120 afterward). No production zero-expiry constructor requires a parallel recovery
framework.

Application owner opens and read-only inspection accept database schema 23 only.
Unsupported old or future schemas fail before changing source state; SDK codec
compatibility does not authorize database conversion. Current fresh stores include
source-disposition and payment-resolution histories; an interrupted schema-23
initialization can finish their empty structures idempotently under exclusive
ownership. Existing dispositions, original bytes and snapshot references remain
preserved. Historical rows do not reopen terminal work or authorize automatic
disclosure; the exact-ID history guard prevents previously closed originals from
being sent automatically. Actual pending originals continue through their existing
owner. Database schema 23 is independent of SDK wallet format 45 and public or
encrypted record formats; see [versioned output](wallet-cli.md#versioned-capacity-output).
No reader removal grants another deposit or permits rollback past accepted
financial work.

## Build-time parameter downloads

The upstream SDK build script obtains and embeds standard parameter-cache files
through `SAPLING_SPEND_PARAMS` and `SAPLING_OUTPUT_PARAMS`; it does not copy them
into its source checkout. Pre-populate the standard cache for an offline first
build. The application never parses or uses these legacy parameters and has no
runtime parameter cache or downloader; its required prover adapter refuses Sapling
requests. These files remain an unavoidable input of the pinned SDK build. Legacy v6 `vendor/zingolib/zcash-params/*.params`
copies remain ignored, unused and preserved. Build-time acquisition is outside
runtime Tor routing.

The enabled registry `zcash_proofs` 0.31.0 `download-params` build dependency
uses upstream minreq 2 and its legacy rustls 0.21 / rustls-webpki 0.101 /
webpki-roots 0.25 cohort. On an uncached ordinary build this downloader is genuinely
reachable. It downloads public proving parameters without runtime client secrets,
outside runtime Tor routing. It neither supplies a production runtime network
constructor nor changes the embedded parameter bytes.

The upstream downloader checks trusted BLAKE2b-512 digests, bounds acquisition by
the fixed parameter length and checks cached file sizes. These protect accepted
parameter integrity; they do not fix TLS verification vulnerabilities, hide build
network metadata or ensure availability. The known advisory exposure is explicitly
accepted in the [dependency disposition](development.md#locked-graph-advisory-disposition)
in preference to maintaining a downloader fork. Keep its raw audit findings
visible and reassess when inputs, consumers or the upstream dependency cohort
change. The [release runner](reproducible-builds.md#prepare-inputs) separately
verifies prefetched parameters and denies network access during qualification;
that stronger build boundary does not make ordinary uncached builds offline.

## Refund receivers and seed recovery

All generated and mnemonic-imported application wallets persist
`TransparentAddressDiscovery::disabled()`. Runtime owner open and SDK reconstruction
apply the same policy even when the saved seed snapshot enabled discovery; the
next ordinary save persists it. Read-only inspection retains the original facts.
This disables known-address RPCs and address-gap recovery without a legacy recovery
mode. Shielded trial decryption and already accepted transaction ownership remain.
The SDK still derives unused transparent keys separately from its initial
Orchard-receiver-only UA. Every application-issued UA uses that same Ironwood
receiver format without Sapling or transparent components.
Current storage accepts schema23 only; the SDK owns historical snapshot decoding,
which is separate from an application pre-seed migration path.

The privacy contract is suppression of address-based correlation between the
refund receiver disclosed to NEAR and transparent-address requests to the Zcash
indexer. New refunds contain no transparent receiver, and disabled discovery
prevents both known-address lookups and gap recovery for generated, imported,
reopened and reconstructed wallets. The real-SDK
[refund/discovery regressions](../src/treasury/lifecycle_tests/refund_addresses.rs)
assert an empty transparent-request log and unconsumed failure sentinels for all
six RPC methods across the transaction-history, balance and UTXO families, including
stream/deprecated aliases. Keep those forbidden-side-effect assertions when
updating the SDK or reconstructing wallets; successful shielded discovery alone
does not prove that no transparent query occurred.

Complete SDK transparent removal is not required for this contract.
`LightWallet::new` still derives a local transparent key/address, and shielded-
relevant compact/full/pending scanning can recognize already-owned transparent
outputs. Raw SDK snapshots preserve those identities and accepted work; they
never authorize new wallet transparent-input work. Do not clear keys or maps to
simulate retirement. Bridge-owned transparent outbound deposits retain their
separate destination/output validation. This contract does not establish blanket
network unlinkability or live-funded refund qualification; see
[network correlation](network-egress.md) and
[refund policy](wallet-rotation.md#refunds-state-and-recovery).

The requested NEAR quote window defaults to 1,800 seconds. Returned quote deadlines
and consensus-derived signed block expiry remain independent, including across NU7;
existing accepted deadlines and bytes survive restart unchanged.

New refund derivation calls the pinned SDK's existing
`generate_unified_address(ReceiverSelection::orchard_only(), AccountId::ZERO)`.
The result contains only the Orchard-format receiver used by Ironwood, without
Sapling or transparent fallback.
The application persists the SDK snapshot and exact job/address binding together
before quote disclosure. Saved transparent/mixed refund bindings remain exact
evidence but fail new refund selection; they do not enable discovery or shielding.

Compact-block trial decryption uses account viewing keys, so shielded receipt
discovery does not depend on a sequential transparent-address gap scan.
`treasury::lifecycle_tests::refund_addresses::distant_shielded_refund_is_spendable_after_seed_only_restore`
receives at a later generated address, then recovers the actual note and spendable
value into a fresh wallet using only the public test mnemonic and early birthday.
It imports no original address list, jobs or snapshot. This synthetic-chain test
does not validate consensus or live bridge delivery. See the maintained
[refund contract](wallet-rotation.md#refunds-state-and-recovery) for exact saved
bindings and financial-authority boundaries.

The [NEAR native bridge implementation](https://github.com/Near-One/bridge-sdk-rs/blob/d1361ac1c68fa579ed1a498d30f2124bc16e09cc/bridge-sdk/connectors/omni-connector/src/zcash.rs#L612)
constructs shielded refunds using an Orchard receiver. Zodl also supplies shielded
UAs to 1Click. Public unfunded dry quotes accept Orchard-only and Sapling+Orchard
refund UAs on our native-ZEC/Base-USDC route. These establish format compatibility,
not funded settlement. Refund receiver validation is separate from the
transparent-only source-deposit contract; mixed UAs may select a transparent path.

Native direct sends accept shielded-only recipients and use the existing SDK
proposal, accepted-input ownership, calculation and publication path. The direct
recipient proof checks exactly one positive OVK-recovered Ironwood payment against
the accepted receiver and amount, and rejects legacy-pool bundles. The pinned backend's
`OvkPolicy::Sender` uses an external OVK for payments and no OVK encryption for
implicit change in all three pools; Pepper's outgoing inventory therefore excludes
that change, including same-receiver self-payment change. Incoming note ownership
does not authorize extra outgoing payments. The direct
operation's optional total-debit authority checks principal plus the SDK's actual
network fee at proposal and calculated-byte publication; it does not estimate the
fee by subtracting the requested amount from an advisory balance. Existing
treasury network-fee safeguards apply independently. Direct previews reuse the
application's Ironwood-only `source_proposal` and SDK `total_fee` through the
serialized actor; the SDK's unrestricted preview wrapper is unsuitable. The
unsigned proposal is discarded, never stored or calculated. Passing `lock_inputs =
None` avoids SDK input locks; no snapshot, address derivation or cached-proposal
replacement occurs. The SDK's note selector ignores its generic lock filter, so
this is advisory and actual preparation still checks accepted-input ownership.
Native estimates use the validated receiver. Foreign dry quotes lack a deposit
address, so a private standard transparent-output shape estimates the fee; neither
that placeholder nor proposal may escape into accepted terms or signing. Foreign
previews use an existing shielded receiver; executable foreign sends persist their
own shielded refund binding before source preparation. Missing funds or scan data
return typed unavailable estimates, not an invented fee.

Native direct receiving uses the same persisted shielded derivation, with a fresh
UA per new request ID and no NEAR transport. Native receipt inspection matches
actual note receiver components before projection and reuses the SDK's saved
confirmations, spent links and selection evidence. Historical received funds and
snapshot spendability are separate facts; no local balance replica or transparent
address discovery is introduced. Shielded compact-block scanning remains the only
incoming-address discovery path.

Direct receive invoices use the same all-shielded derivation and encrypted snapshot
boundary, with their own persisted operation bindings. A public unfunded reverse
Base-USDC-to-ZEC dry quote accepted the shielded-only UA on 2026-11-07. The pinned
native bridge's `get_orchard_raw_with_memo` implements shielded withdrawals (and
selects Ironwood after activation), separately from shielded refunds. This supports
the implemented address contract despite the general chain guide's transparent-only
statement; neither code nor dry quotes qualify funded delivery. Direct receiving
keeps transparent discovery disabled and never falls back to a transparent receiver.

## Release descriptor inputs

The upstream Git build derives its embedded descriptor from checkout revision,
matching tags and dirty status. A source revision alone does not pin all those
inputs. The release runner records the resolved SDK package/source, observed Git
revision, matching tags and `diff-index` result under each isolated build
environment, then records the actual generated `git_description.txt`. It does not
predict the descriptor or modify cached sources/tags to force one. A repository-free
archive falls back to the crate version; with Git installed, the failed dirty
check can add `_dirty`. Shared-cache, same-host comparison does not qualify
reproducibility across independently populated caches. Parameter staging, integrity
checks and OS network denial remain separate release requirements.
