# Unfinished treasury owner work

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

Until implemented, unhealthy SDK calculation and exited treasury/funding
or reconciliation owners can require process restart. Independent listeners and
usable Base payers remain available. Restart retains an accepted calculation even
when it has no outgoing bytes; it does not certify that calculation never happened.

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
shielding and automatic owner reconstruction remain unfinished.
