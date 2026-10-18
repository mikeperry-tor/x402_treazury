# Treasury owner follow-ups

Core best-effort balance simplification does not deliver these follow-ons. They
require separate implementation and review, with no live spending authorization.

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

Current explicit per-address shielding remains available. General automatic
shielding remains unfinished.
