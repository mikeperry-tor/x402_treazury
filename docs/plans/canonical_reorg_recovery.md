# Canonical evidence and automatic reorg recovery

Starting baseline: `0a6dbb93f5017c91e8eca1487b6d8416ecf26d2e`.
The operator wants blockchain evidence to resolve recoverable uncertainty instead
of permanent manual-repair stops. Current Base depth-only proofs and repair
latches do not meet that requirement. Current Pepper rollback retention also
predates Zebra's wider local rollback policy.

Preserve original authorization identities, accepted inputs, exact signed bytes,
first-positive-credit claim consumption, retired signer exclusion, existing
observation owners/cadence and financial authority. No replay, replacement deposit,
quorum, watcher, whole-work timer or parallel monetary ledger is authorized by
reconciliation. Corrupt/missing original authority remains a scoped record error.

Owning guides: wallet-rotation.md, wallet-cli.md, configuration.md,
network-egress.md, zingo-integration.md, vendor/README.md, tests/LIFECYCLE.md.
AGENTS.md already links these enduring rules.

Independent initial design reviews challenged checkpoint necessity, archive
representation, SDK rollback and full consumers. Accepted requirements: preserve
superseded exact proof records; atomically reopen original liability and rebuild
exposure; invalidate stale captures; avoid historical anchors recreating latches;
finality unavailability must never silently downgrade evidence.

Ordered commits:

0. **K0 Historical support schema fixup.** Correct the stale local validity-field
   requirement left after age-policy removal, against its introducing change.
   Use faithful same-block support in regression coverage. Implemented as
   `cbaacabb`; independently reviewed, final runtime evidence pending.
1. **R1 Base finality and canonical correction.** Require both configured depth
   and the RPC finalized declaration for balance/nonce/receipt final evidence.
   Latest scalars remain advisory. Replace permanent repair latches with scoped,
   atomic original-authorization reopening and append-only historical proof
   preservation. Existing pending pages own restart recovery. Repair derived
   checkpoint/debit projections without changing original journals. Cover fork
   replacement, used→unused-valid→expired, expiry rollback, stale publication,
   restart, transaction failure, unrelated progress and unavailable finality.
   Close never-credited completed destinations only when every current/archived
   source operation has exact expiry evidence or was never accepted; preserve
   unknown delivery from confirmed deposits. Recover optional historical-support
   damage without confusing it with missing original authorization identities.
   Update Base configuration, reconciliation, CLI and observation documentation.
2. **R2 Zcash canonical continuation.** Align the shared rolling SDK rollback
   horizon with the node's local 1000-block policy, preserve automatic deeper-fork
   rescan and recover shorter-tip/missing-checkpoint cases without losing keys or
   application input ownership. Treat recoverable chain/SDK observation disagreement
   as waiting and let confirmed source work continue ordinary settlement lanes.
   Preserve exact transaction/input release checks.
   Let a completed canonical SDK observation confirm exact original bytes without
   an independent lookup veto. After buried original expiry, accept a different
   sufficiently confirmed spend of an exact original input as positive exclusion,
   while validating every immutable original input before that shortcut.
   Review the saved-birthday/deeper-fork boundary and preserve scan coverage.
   Add wired SDK/lifecycle
   regressions; update vendor provenance and Zingo/recovery guides.
3. **R3 Integration and documentation handoff.** Resolve discovered gaps with
   fixups against introducing commits; independently review each exact SHA/fixup.
   Remove this plan only after canonical documentation covers implemented rules.

Review gates after R1 and R2 examine all preceding changes/fixups and complete
financial/network consumers, concurrency, cancellation and historical journals.
An independent reviewer must challenge necessity and opportunities to remove
enforcement. Compare CogC/CycC against the starting baseline, inspect changed and
unchanged high-risk call chains, and explicitly disposition warnings. Material
findings reopen affected design before dependent implementation.

Validation: no repeat of the previously completed full suite merely for audit.
For these changes run static/provenance/complexity, production compiler/all-target
Clippy and compatibility gates, then focused Base/store/retirement/treasury and
synthetic-chain regressions on frozen matching sources. Broaden only where changed
behavior invalidates prior evidence. Verify nonzero test counts and forbidden
side-effect assertions. Keep detailed evidence/metrics in ignored target output.
Final integration independently examines all commits/fixups, final cleanup SHA,
canonical docs and remaining genuine operator boundaries before completion.
