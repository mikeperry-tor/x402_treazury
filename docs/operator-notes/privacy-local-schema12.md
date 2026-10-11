# Known privacy-local schema 12 conversion preconditions

The inspected operator state is schema 12, with three funded active/ready pairs,
six completed funding jobs and five USED payment resolutions. All six current
funding progress rows remain `VERIFYING_CREDIT`, while their jobs are `COMPLETE`
and source operations are confirmed. Two archived operations retain original
quote/refund bindings in `QUOTED` and `RECOVERY_REQUIRED`; the latter has a
`PREPARED` source operation with resolved bytes and canonical-expiry metadata. No return outputs or outstanding source reservations were recorded.
Inspection used schema and relevant accounting metadata only, without reading
keys or seeds. No real conversion has been performed.

The source has no immutable destination-readiness support bound to the original
funding operation. Its old `record_credit` overwrote the wallet balance/block,
and completion saved a job phase. Neither proves the required historical D fact.
The operator explicitly chose strict preflight rejection rather than reconstructing
that fact from current balances or introducing an alternate legacy proof format.

The one-time Rust utility is outside the production wallet command tree:

```sh
scripts/zcash.sh run --example convert_privacy_local -- --state-dir state/privacy-local
```

This is the invocation to use for the stopped-service preflight; it has **not** been
run against operator state during implementation. It acquires the existing
exclusive state lock, opens a private database snapshot with query-only enabled, and checks
schema, integrity, ownership and relevant financial metadata. It reads no key file,
loads no wallet, makes no network request and creates no backup or new authority.
The known source returns `legacy_destination_readiness_support_missing` before any
mutation. Unresolved source obligations behind a COMPLETE job and missing exact
payment-resolution metadata have separate fixed diagnostics. Unsupported shapes
require investigation; there is no proof-import or automatic repair path.

Transformation is deliberately unavailable for this source. The utility contains
no successful schema-12 transformation branch, backup path or transactional schema
rewrite. A schema-16 database with the expected structural markers is recognized
without mutation; recognition does not certify financial readiness. Do not edit
`user_version` or add purported support fields to bypass rejection.

Before any later separately reviewed conversion, stop serving, retain exclusive
ownership, and create a consistent full backup containing the database/journals,
treasury state and all required key material, including external key files.
Review explicit payment authority, small/optional-large reserve targets and
per-operation safeguards manually; preserve user-owned configuration files.
The source needs an actually supported proof/ownership precondition before a
transactional conversion can be implemented. Ordinary observation/recovery may
clarify existing work but does not manufacture historical destination evidence.
Any incident-specific reconstruction requires separate review.

The synthetic schema-12 fixture contains no operator secrets or identifiers. Tests
compare entire database bytes across rejection, reopen and repeated invocation,
including original ciphertext placeholders, accepted bytes, correlations, archived
operations, gross accounting and payment histories. They test exclusive ownership,
version guards, missing proof and source-pending COMPLETE diagnostics. These are
rejection/preservation tests, not successful conversion, restored-key derivation,
rollback-after-transformation or live migration qualification. Those gates remain
unavailable while the known source fails the strict precondition. Existing gross
totals are preserved; their removal as ordinary funding gates is covered by the
funding implementation independently of this rejected conversion.

Retain the source backup before any future conversion. After subsequent spending,
never restore an obsolete financial snapshot to undo an incident. Preserve the
latest journals for incident-specific recovery; neither backups nor retained keys
grant permission to replay uncertain operations.

The stopped-source preflight holds the existing exclusive application owner lock
and copies the database plus any WAL/rollback journal into a private
disposable directory. SQLite recovery and query-only inspection use that copy;
no source shared-memory sidecar is created or modified. The copy contains no
loaded keys and is removed after inspection. Wallet payloads remain encrypted,
but database metadata is plaintext. A synthetic WAL fixture verifies
that committed WAL changes affect rejection and that repeated inspection leaves
the source database, WAL and owner file unchanged. Runtime schema preflight shares
this path so committed crash-recovery state is not mistaken for an older schema.
