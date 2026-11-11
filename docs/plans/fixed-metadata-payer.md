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
