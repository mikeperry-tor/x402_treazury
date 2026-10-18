# Treasury owner follow-ups

These follow-ons extend the core best-effort balance simplification. Their local
implementation and review do not authorize live spending or establish external
settlement qualification.

## R01: SDK and actor reconstruction

Reconstruct usable treasury capabilities after a known safe SDK failure or an
exited owner using existing supervision and exact committed state. Preserve the
single cleanup owner, accepted command, input owner, address derivation and prepared
bytes. Caller cancellation never transfers ownership. A live retained owner retrying
SQLite Busy/Locked is not task replacement. Do not introduce another lifecycle or
capability framework, renew stopped-process authority, replay uncertain effects or
claim recovery of unpublished state.

Completed SDK bytes stay with the existing owner through local validation or
publication failure. That owner retries only the identical local publication on
its existing maintenance cadence, including while an owned close drains. A cancelled
close waiter cannot discard it. This is in-owner resumption, not SDK reconstruction.
Irrecoverable storage failure or forced process death can still lose unpublished
bytes; the durable accepted input claim remains uncertain and blocks duplicate
calculation.

Implemented R01: the retained treasury driver reconstructs only the SDK from the
latest committed authenticated snapshot through its existing Store. It retains the
command receiver across interrupted commands, never replaying them. Funding and
reconciliation supervisors replace joined failed tasks through existing leases and
gates; source backoff/accepted work remain owned. Retry uses existing maintenance
cadences; explicit shutdown prevents replacements. Shared Store/authentication loss
still requires operator repair. Independent listeners and usable Base payers remain
available. Restart retains an accepted calculation even without outgoing bytes.

Independent exact-commit review and architecture gate accepted R01 `dce952b`.
Focused tests cover failed derivation publication, lost commit acknowledgment,
uncertain selected inputs and restrictions, shutdown during reconstruction,
interrupted command/driver waiter cancellation, actual worker task replacement and
completed publication retention. No live spending or private-state conversion.

## R02: Automatic recovery of owned transparent capital

Investigate the smallest SDK shielding path for confirmed spendable owned
transparent funds, including unattributed returns. Use the existing treasury owner,
current auto-funding/run authority, actual proposed fee safeguard and selected-input
ownership. Do not reserve returned principal, build a refund ledger or reopen an
initial-funding claim. Preserve address/network privacy and exact prepared shielding
bytes through cancellation and restart. Prove selected inputs/output ownership from
the pinned SDK. Cover disablement, insufficient inputs and uncertain submission
without preparing a duplicate spend.

Implemented R02 uses the existing owner after persisted sync. Each proposal selects
one owned account-zero transparent address, including unattributed outputs. The
pinned SDK selects confirmed spendable inputs and scans calculated bytes with the
wallet UFVKs; validation requires exact selected outpoints, no transparent outputs
and owned shielded notes equal to inputs less actual fee. Only the fee is cost.
Automatic work requires enabled ordinary funding and the configured shielding fee
safeguard. Job-scoped qualification permits cannot authorize jobless shielding.
Original encrypted input manifests provide stable per-address network isolation,
without another table, ledger or schema change. Existing explicit job recipient
bindings retain precedence.

Pending bytes precede new selection. Initial automatic submission requires zero
prior attempts and current permission/fee policy; uncertain submission is observation
only. Existing two-scan zero-lag expiry and original-unspent-input proof release a
resolved owner. Restart, caller cancellation and disabled automatic funding retain
accepted inputs and exact bytes. Current explicit per-address shielding remains
available. See the maintained [wallet rules](../wallet-rotation.md) and
[network identity rules](../network-egress.md).

Local synthetic-chain tests use public unfunded keys, actual SDK proofs/bytes,
transparent detection and shielded confirmation. They do not provide consensus,
live bridge, Tor route or funded lifecycle qualification.
