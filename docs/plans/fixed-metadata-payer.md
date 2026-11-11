# Fixed bootstrap metadata payer and uniform allocation cadence

Implement the operator's approved simplification on
`default-wallet-bootstrap_squashed2`, starting at `bf3cb288`.
Paid metadata relay retrieval stays on the configured bootstrap group before and
after API handoff. Off-by-default API/raw relay fallback retains the API call's
captured authority. Catalogs may wait for ordinary bootstrap funding. Eligible
groups use independent 30–120-second allocation intervals.

With bootstrap policy enabled, its bootstrap group is authoritative for metadata
relay fees, including catalogs, pricing, help, imports, probes and built-in
directory discovery. API calls to directory URLs retain API classification.
The legacy explicit static relay override continues outside bootstrap policy.
Inspection and qualification preserve their existing authority boundaries.
Unsigned origin retrieval remains independent of Store availability. Runtime
document retrieval and offline refresh retain their existing no-funding-wait
contracts. Serving catalog waits retain ordinary funding notifications.

Preserve first-use API demand, permanent first-positive-credit API handoff,
captured payment caps, transports and journals, accepted charge ownership,
provider serialization and cancellation drain. Add no financial ledger, resource
limit, dependency checkout or scheduler deadline. Preserve unrelated `state`.

| Step | Commit scope | Acceptance evidence |
| --- | --- | --- |
| M1 | Fixed metadata payer across deployment, static catalog and runtime/offline consumers. Remove participant-dependent catalog readiness and cap-handoff machinery with its replacement in the same change. Update configuration, architecture, agent-source and wallet CLI guides. | Static and runtime metadata uses bootstrap after destination credit, including explicit legacy overrides. API/raw fallback retains captured API authority. Concurrent help/import coalescing, cap refusals, ordinary funding waits, unsigned origins with a busy Store, qualification and non-bootstrap compatibility remain covered. |
| M2 | One 30–120-second interval policy and simplified participant eligibility discovery. Update wallet rotation and both timer inventories. | Independent armed opportunities survive unrelated wakes and credit. Dormant groups remain unallocated. Historical credit without demand remains eligible. Busy dormant discovery cannot delay ordinary or already armed participant opportunities. |
| M3 | Independent exact-SHA and combined integration review, source-matched CogC/CycC review and validation. | Production compiler and all-target Clippy before affected focused runtime tests. Carry valid baseline compatibility evidence, run affected compatibility consumers when needed, record exact results and limitations. Resolve material findings in independently reviewed fixups. |
| M4 | Record acceptance evidence and remove completed plan. | Permanent guides contain enduring rules, incoming links remain valid and an independent reviewer approves exact cleanup plus final combined state. |

Independent metadata and scheduler design reviews precede implementation. Resolve
material design findings before building the affected decisions. Authors own
explicit disjoint files during parallel implementation and coordinate shared
interfaces. Freeze sources and executables for validation. No full test suite is
needed without a newly exposed concern. Repeat exact and combined reviews until
no material finding or compelling simplification remains. Preserve unsquashed
history and review evidence through cleanup.

The owning contracts are in `docs/configuration.md`, `docs/architecture.md`,
`docs/agent-sources.md`, `docs/wallet-cli.md`, `docs/wallet-rotation.md`,
`docs/local-resource-limits.md`, `docs/network-egress.md`, `docs/testing.md` and
`tests/COMPLEXITY.md`.

## Implementation and acceptance evidence

Independent metadata and scheduler design reviews approved the scoped policies
before implementation. M2 was committed before M1 because its Store projection
rename is consumed by the payment regression, while the two implementation areas
remained independently owned.

- M2 is `b6f6d62b`. It removes the cadence phase, the second opportunity map and
  due-time eligibility rereads. Independent exact and combined review verified
  monotonic admission, retained due instants and Store commit guards.
- M1 is `e17a9e91`, with non-bootstrap compatibility coverage in `c3f09dc1`.
  Metadata relay authority is fixed across static, runtime and offline consumers.
  API/raw fallback retains captured API authority. Catalog funding waits retain
  their preexisting non-bootstrap initialization and notification behavior.
- Initial independent review found late static clients missing the metadata
  binding and contradictory Coinbase/refresh documentation. `60a225c6` applies
  one shared binding step at both initialization boundaries and adds an actual
  signed-payer regression. `63992c69` updates the owning guides. Both exact
  fixups and combined affected consumers passed independent review.
- Final production compiler, all-target Clippy, format and diff checks pass.
  `target/fixed-metadata-focused.log` records 413 library and two CLI passes,
  with nine ignored named subprocess helpers. `target/fixed-metadata-compat.log`
  records 31 compatibility passes. `target/fixed-metadata-config.log` records
  18 managed configuration passes. Sources and executables stayed frozen during
  these runtime checks. The existing macOS test-linker compact-unwind warning
  remains a diagnostic limitation. Live-funded qualification was outside scope.
- The complete source-matched complexity report at `63992c69` covers 504 files
  with zero exclusions. The actual shared binding step restores initializer CogC
  to 61. Resolver CogC rises from 21 to 23 for bootstrap precedence within the
  retained legacy selection contract. Independent review retained that decision.
  Scheduler CogC/CycC falls from 66/76 to 65/74, with one opportunity map and no
  credit cadence phase. `Prepared::acquire` falls from 39/38 to 34/32 and the
  catalog activation/proof responsibilities are deleted. High-risk ownership
  call chains were independently examined beyond metric warnings.

The final independent combined review of `63992c69` found no material issue or
compelling further simplification. All M1–M3 acceptance gates are complete.
Enduring rules are in the maintained guides and examples. M4 plan deletion and
independent exact cleanup review remain.
