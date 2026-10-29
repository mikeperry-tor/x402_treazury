# Seed-based wallet recovery and key retention

## Intent, baseline and scope

Research and plan recovery of rotating Base USDC wallets and treasury Zcash
funds without recreating a second transaction ledger. The starting branch is
`wallet_retirement` at `a0249da36636728f4aba65e8082fff7878e932c3`.
Everything below is proposed unless identified as current behavior. This plan
does not authorize funded execution, sweeping or secret destruction.

The user also wants the ability to discard old signing secrets. Recovering every
generation from an original seed and irreversibly forgetting selected generations
are competing requirements. Resolve that policy before enabling Base HD allocation;
do not turn it into a custom cryptographic project by default.

Owning guides are [wallet ownership and reconciliation](../wallet-rotation.md),
[wallet administration](../wallet-cli.md), [configuration](../configuration.md),
[Zingo integration](../zingo-integration.md) and
[network policy](../network-egress.md). The separate
[sweeping plan](wallet_merge_sweeping.md) owns moving recovered USDC, gas funding
and explicit consolidation/privacy choices. Address recovery alone does not
implement a Base withdrawal or transaction relayer.

Preserve SDK-owned ZEC spendability, finite signed source expiry, canonical Base
finality, bound NEAR fulfillment, original accepted bytes/identities, no top-ups,
per-operation failure isolation and journal-before-disclosure. Seed recovery must
not reopen funding claims, replay accepted work or add cross-chain consistency
vetoes. Normal reorgs remain automatic reconciliation, with INFO for meaningful
recovery delays and extra unresolved observation requests.

## Research and current implementation

| Standard or reference | Consequence for this design |
| --- | --- |
| [BIP39](https://bips.dev/39/) | A mnemonic encodes entropy; its normalized words and optional passphrase produce a seed. Generate new roots with 256-bit entropy/24 English words. A passphrase is part of recovery, not a replaceable login password. |
| [BIP32](https://bips.dev/32/) and [BIP44](https://bips.dev/44/) | Standard secp256k1 derivation and `m/44'/coin_type'/account'/change/index`. A retained ancestor private root regenerates descendants, including hardened descendants. BIP44 discovery stops at unused accounts and a 20-address external gap; this is not exhaustive for our allocations. |
| [SLIP44 registry](https://github.com/satoshilabs/slips/blob/master/slip-0044.md) | ETH is 60, ZEC is 133 and Base also has registered coin type 8453. Path selection and EVM chain ID are distinct choices. |
| [Phantom's documented paths](https://help.phantom.com/articles/12988493966227) | `m/44'/60'/0'/0/i` is an implemented Ethereum/Base interoperability choice. It is the proposed Base path here, not a requirement to use Base's registered coin type. |
| [ZIP32](https://zips.z.cash/zip-0032) | Standard shielded Sapling/Orchard derivation; new seeds require at least 256 bits of entropy. BIP39 seed material can serve both this tree and a BIP32 tree. Different wordlists are not inherently necessary. |
| [ZIP315](https://zips.z.cash/zip-0315), [ZIP339](https://zips.z.cash/zip-0339) | ZIP315 is draft recovery guidance, including 24-word generation; ZIP339 is reserved, not a competing normative mnemonic format. Preserve existing valid treasury imports. |
| [ZIP316](https://zips.z.cash/zip-0316) | Unified addresses encode shielded and/or transparent receivers. Transparent address derivation alone does not make arbitrary unused gaps discoverable. Qualify the revision and receiver support used by our pinned SDK. |
| [ERC3009](https://eips.ethereum.org/EIPS/eip-3009) | Signed transfers use independently chosen authorization nonces and validity intervals. A seed or chain scan cannot enumerate undisclosed signatures. Key deletion does not revoke signatures already given to others. |

Current Base allocation uses `PrivateKeySigner::random()` in
`src/rotation/store/capacity.rs::allocate_claim`. Each key is encrypted with the
store key and wallet/treasury/network binding. Existing random keys cannot be
reconstructed from a new root or the current Zcash phrase. Preserve their encrypted
records and backups; do not silently migrate by transferring funds.

Current retirement retains signing keys. `Store` reports that remaining USDC stays
owned and keys remain available. Retirement stops ordinary use/observation according
to its existing rules; it does not promise secret destruction or prove that nobody
can send future funds to the public address.

The treasury already imports a mnemonic through the SDK and derives account zero.
`Treasury::refund_address` generates successive external transparent addresses and
saves the SDK snapshot and refund binding before requesting a quote. There is no
special high-offset refund branch. However, many unused addresses can separate
receipts: Pepper's normal transparent discovery gap is 10, and recovery uses 20.
Saved known addresses are queried independently of that discovery gap. Thus a
snapshot restore and a mnemonic-only restore have materially different coverage.

Pinned Zingolib is `bfb0a2156819ea7f75a9b09b1724abfd06c8925e`. Its existing
`generate_unified_address` and transparent-address APIs are sufficient candidates
for this work; qualify them before changing callers. `bip0039` is already used by
the application; `bip32` is locked transitively. Review its current backend/API
before choosing a direct dependency. Enabling Alloy's optional mnemonic support
could add another derivation stack. Do not vendor a dependency merely for this work.

Reference Zodl Android `9f4e719e` uses `RequestSwapQuoteUseCase` and
`AccountDataSource` to supply a fresh shielded custom UA for refunds. Zodl iOS
`6a7dc6b3` rotates/persists a shielded UA in `SwapAndPayStore`, with receiver
selection in `WalletAccount+Stash`, then forwards it through `Near1Click`. These
are precedent for seed-recoverable shielded ZEC refunds, not Base HD custody:
their foreign-chain recipient is supplied by the caller. Verify these contracts
in `reference_repos/` and against our actual NEAR integration before implementation.

## Key-lifecycle decision gate

| Choice | Recovery promise | Forgetting and retained wallets |
| --- | --- | --- |
| Standard HD root | Original phrase regenerates all derived keys. | Individual generations cannot be forgotten while any ancestor secret survives. Operational retirement can proceed while keys remain recoverable. |
| Existing independent keys | Complete key backups recover the wallets; one phrase does not. | A particular key can potentially be destroyed without affecting siblings, once every copy is removed. |
| One-way evolving secret | A current checkpoint derives future keys; an original checkpoint also derives everything after it. | Forgetting requires removing every earlier ancestor/checkpoint backup. Retain isolated signing keys for old wallets still needed, not old ratchet ancestors that regenerate the discarded range. |

The suggested ratchet is conceptually possible: advance secret state with a
one-way function and derive each wallet key separately. A retained old wallet
need not prevent advancing the current secret if its individual signing key is
saved separately. Avoid using a directly exposed wallet private key as the next
generation's derivation secret. This is a design observation, not an approved
cryptographic construction or a standard wallet recovery format.

A ratchet would require current checkpoint backups and careful atomic updates;
an old backup either defeats forgetting or, if destroyed, can no longer provide
recovery. It therefore does not meet the original permanent-seed recovery promise.
Compromise of current evolving state also exposes future deterministic generations
unless a separately designed entropy-refresh mechanism intervenes. Do not build a
ratchet, puncturable derivation tree or epoch framework in this plan. If forgetting
wins, retain independent Base keys and specify a separate reviewed destruction and
backup policy. Epoch roots would provide only coarse deletion and still have the
ancestor-backup problem.

Unresolved work can require retention of particular keys and accepted evidence;
it need not halt creation of future wallets under existing authority. Operational
retirement, obligation resolution and irreversible destruction are separate
decisions. Even a retired address can receive arbitrary late credit; destroying
its key accepts permanent loss of such credit. Deleting keys cannot terminate
already disclosed signed authorizations or erase public blockchain history.

The current shared store encryption key is not per-wallet cryptographic erasure.
Overwriting one database field does not remove previous copies from WALs, journals,
snapshots, exports, memory or backups. SQLite documents
[secure-delete behavior](https://www.sqlite.org/pragma.html#pragma_secure_delete)
and [WAL storage](https://www.sqlite.org/wal.html); neither gives a guarantee about
all external copies. No automatic deletion is proposed here.

**Recommended default pending the decision:** preserve existing keys and backups.
If permanent seed recovery is the priority, choose ordinary HD and document that
retired descendants remain recoverable. Keep the existing Zcash phrase unchanged;
propose a separate Base phrase to avoid broadening its compromise domain. A shared
BIP39 seed is technically possible, but cannot retroactively derive old random
Base wallets and couples the two key trees. Do not implement multiple modes merely
to avoid deciding the recovery promise.

## Proposed recovery contract if HD is selected

The Zcash refund/discovery work in points 4–5 and S3 is independently useful with
the existing random Base keys. It does not depend on choosing HD or ratcheting.

1. Allocate future Base wallets under the immutable, versioned path
   `m/44'/60'/0'/0/i`, using one global monotonically allocated index per Base root,
   independent of pool UUIDs, assignment names and per-pool sequence. Serialize
   allocation through the existing store transaction. Persist the chosen index,
   identity, address, key material/derivation reference and operation ownership
   before disclosing an address or requesting funding. A failed undisclosed
   allocation may leave a harmless gap; never reuse a disclosed identity.
2. Back up the root securely and an explicit recovery descriptor: derivation
   version/path, network/contracts, root identity, accounts, issued-index frontier
   and scan birthday. The descriptor is not a second financial ledger, but it is
   privacy-sensitive. Do not export account xpubs by default. A root fingerprint
   identifies a candidate; it is not collision-proof authorization or a substitute
   for authenticated root/address validation.
   Reject root/network mismatches only against trusted descriptor/address bindings
   and configured chain contracts. Without such metadata, a different valid phrase
   cannot be identified as wrong from empty scan results; an EVM address does not
   encode the Base chain ID. Do not add an empty-history readiness gate.
3. Discover Base USDC using token history as well as balances, including an address
   with zero ETH transaction nonce and formerly spent USDC. Use established
   finality and retry/reconcile reorgs through existing transports. A phrase alone
   cannot reveal the last issued unused index. With a descriptor, scan its entire
   issued range. Without it, allow an explicit expanding/resumable search range
   and report coverage as incomplete; a gap of 20 or a zero balance cannot prove
   that higher used addresses do not exist. An old descriptor is also a lower
   bound, not proof that later allocations never occurred.
   Restoring a stale database can otherwise reuse a previously disclosed but
   still-unfunded index: scanning cannot detect this. Seed-only/stale restores
   remain discovery-only and cannot restart allocation from the highest observed
   address. Resuming allocation under the same root requires an authenticated
   complete issuance frontier and exclusive ownership of that issuance namespace;
   otherwise use a separately reviewed explicit new-root/namespace transition.
   Existing local store ownership does not coordinate cloned stores on separate
   machines. Do not add a distributed allocator to solve this implicitly.
4. Prefer shielded-only UAs for **new ZEC refunds**, following Zodl's simpler
   account-scanning model. Persist the address/binding before the quote as today.
   Qualify NEAR acceptance and SDK receiver selection first. Change only refund
   validation in `src/rotation/near.rs`; its transparent source-deposit validation
   serves a different contract and must remain. Preserve observation/shielding of
   legacy transparent funds. For legacy mnemonic-only recovery, support an
   explicit external-index range prederivation before scanning; increasing the
   ordinary discovery gap alone cannot guarantee completeness.
5. Scan Zcash from a known birthday no later than relevant receipts. If unknown,
   use the supported network activation boundary rather than a recent tip. Let
   the SDK discover shielded notes by account keys, including fresh refund UAs
   whose ordinal was not saved. Retain existing automatic rollback/full-rescan
   behavior, accepted pending ownership and finite transaction expiry semantics.
6. Distinguish **fund/key recovery** from **resuming financial workflows**. The
   full backup remains necessary for legacy random keys, signed authorizations,
   exact original transactions, accepted NEAR requests and pool/claim ownership.
   A seed restore must not fabricate these or treat missing journals as proof of
   no liabilities. Discover/import keys and funds without signing, allocating, funding,
   clearing claims or broadcasting. Reuse of a recovered old payer for new live
   work requires retained obligations or a separately reviewed explicit authority
   workflow; changing an EVM transaction nonce does not cancel ERC3009 grants.

## Ordered commits and acceptance evidence

Before substantive code, an independent reviewer challenges necessity, complete
call chains, and opportunities to delete responsibilities. Resolve material
findings before building on the decision. IDs below are stable plan boundaries;
the gate may select independent keys and cancel the HD-specific commits.
S1 precedes S2. S3 can proceed independently after its own SDK/NEAR design review,
without waiting for S1/S2. S4 integrates whichever recovery scope was selected.
Keep stable IDs when that dependency order changes.

| ID | Coherent change and consumers | Acceptance evidence and permanent documentation |
| --- | --- | --- |
| S1 | Decide recovery versus erasure; if HD chosen, introduce versioned derivation/root storage and explicit recovery descriptor using existing encryption/ownership. No managed allocation switches yet. | Independent design review; standard vectors and exact addresses/signatures, passphrase behavior, wrong-root/network rejection, backup/restore interoperability and dependency assessment. Update wallet CLI backup/seed contract, configuration and wallet rotation; current random-key behavior remains accurately described. |
| S2 | Enable atomic future Base allocation and mixed legacy/HD restore, with its complete backup/frontier path in the same commit. Consumers: capacity, assignment, funding, payer loading, backup/CLI. | Concurrent allocations across pools, crash before/after commit/disclosure, no disclosed index reuse, stale-backup/seed restore cannot allocate over a higher issued-but-unfunded index, explicit cloned-root ownership scope, migration preserving legacy keys and accepted journals byte-for-byte, multiple pools/restores without UUID-derived paths. Update wallet rotation, wallet CLI and configuration. |
| S3 | Replace new transparent refund generation with qualified shielded UAs and provide legacy index-range recovery. Keep existing accepted refund bindings and legacy shielding. | Real SDK shielded refund receipt restored from phrase/birthday at a distant UA ordinal without address metadata; legacy receipt after more than 20 unused transparent addresses recovered by explicit range; old quotes unchanged; source deposits still enforce their actual format. Update Zingo integration, wallet rotation and wallet CLI. |
| S4 | Expose explicit fund/key recovery and coverage reporting, with no implicit financial resumption. Consumers: CLI, SDK import/scan, Base history, backup descriptor and network constructors. | Widely separated USDC indices, spent-to-zero history, token receipt at ETH nonce zero, stale/missing frontier, unknown birthday fallback, cancellation/restart, reorg/finality changes, partial provider outage and independent sibling progress. Assert no signatures, funding or broadcasts; report missing legacy keys/journals and incomplete ranges clearly. Update wallet CLI, configuration and relevant network/observation guides. |

Do not add a generic operator-repair registry, workflow ledger, global readiness
fence, cap or whole-operation deadline. Each remaining stop must name the missing
authority or actual liability; derived state should be reconstructed instead.

## Review gates, validation and handoff

- **G1 before S2, after S1:** independently settle the key-lifecycle choice, path,
  discovery coverage, backup rollback/frontier semantics, legacy migration and
  dependency API qualification. Review S1 and its fixups alongside the existing recovery call chains. If the
  choice remains open, stop HD rollout here; Zcash refund work can proceed independently.
- **G2 after S3:** review all selected S1–S3 changes and fixups, complete allocation/disclosure,
  backup/restore, NEAR quote/refund and SDK scan/shielding chains. Compare actual
  pinned reference implementations. Repeated fixups require identifying the
  mistaken assumption and deciding whether a wider simplification/audit is needed.
  If S3 precedes the HD work, review its complete independent chains first and
  reopen this combined gate when S2 is added; cancelled HD scope is not a blocker.
- **G3 after S4:** independently review the combined branch and every prior fixup,
  concurrency, cancellation/crash boundaries, financial ownership, finality,
  network isolation, recovery coverage, logs and documentation. Seed restoration
  must not masquerade as recovery of undisclosed off-chain obligations.

For each implementation commit, independently review its exact committed SHA with
intent, baseline, affected consumers and validation evidence. The reviewer is not
the author. Address findings using `git commit --fixup=<introducing-SHA>` and
independently review each exact fixup; preserve unsquashed history. Material later
changes reopen affected gates. Reviews remain read-only by default.

Use CogC/CycC against the agreed baseline for substantive Rust changes and at the
architecture gates, including complete call chains and unchanged high-risk
hotspots. Evaluate deletion/simpler ownership before extracting helpers. Follow
`tests/COMPLEXITY.md` and `docs/testing.md`: applicable cheap contracts and design/
complexity review, production compiler, all-target Clippy and compatibility checks
precede broader runtime stages. Run focused meaningful SDK/lifecycle tests for
changed behavior; verify they are actually wired and execute. Coordinate builds
and preserve source/executable evidence. Funded or Docker qualification needs its
own authorization. Do not rerun an unchanged full suite merely for research or
documentation; changed runtime behavior requires its focused acceptance evidence.

Each implementation commit updates the owning permanent guides listed above;
tests and complexity evidence belong under ignored `target/`, not plan archives.
At final handoff verify AGENTS.md still links the wallet ownership/reconciliation,
wallet CLI and Zingo guides; add a link only if a new enduring guide is introduced.
Verify those guides contain the chosen recovery/retention promise, paths, legacy
limits, scan coverage and safe operator actions. Remove this plan only after its
work is complete or retained unfinished scope has been narrowed in place. Review
the exact cleanup SHA and final combined branch independently; do not retain a
completed-plan archive or delete financial evidence during documentation cleanup.
