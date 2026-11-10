//! Explicit, offline note evidence from one authenticated committed SDK snapshot.
use anyhow::{Context, Result, ensure};
use pepper_sync::{
    keys::KeyId,
    wallet::{NoteInterface, OutputInterface, WalletTransaction},
};
use serde::Serialize;
use std::{collections::BTreeSet, io::Cursor, path::Path};
use zcash_client_backend::data_api::{
    InputSource, MaxSpendMode, TargetValue, WalletRead, locking::LockFilter,
    wallet::ConfirmationsPolicy,
};
use zcash_protocol::ShieldedPool;
use zingolib::wallet::{LightWallet, output::OutputRef};

#[derive(Serialize)]
pub(crate) struct NotesInspection {
    format_version: u32,
    treasury_id: String,
    state_schema_version: u32,
    snapshot_revision: i64,
    sdk_scanned_height: Option<u32>,
    sdk_min_confirmations: u32,
    selection_target_height: Option<u32>,
    selection_anchor_height: Option<u32>,
    spendability_limitations: [&'static str; 2],
    spendability_unavailable_reason: Option<&'static str>,
    notes: Vec<NoteEvidence>,
}

#[derive(Serialize)]
struct NoteEvidence {
    pool: &'static str,
    txid: String,
    output_index: u32,
    account_id: u32,
    scope: &'static str,
    value_zatoshis: u64,
    receiver: String,
    receiver_encoding: &'static str,
    transaction: TransactionEvidence,
    spend_status: &'static str,
    spending_txid: Option<String>,
    spending_transaction: Option<TransactionEvidence>,
    sdk_spendable_at_snapshot: Option<bool>,
}

#[derive(Serialize)]
struct TransactionEvidence {
    status: &'static str,
    confirmed_height: Option<u32>,
    saved_confirmations: Option<u32>,
    retained_block_hash: Option<String>,
}

/// No Store owner, LightClient, temporary wallet directory, repair or network call.
pub(crate) fn inspect_notes(
    dir: &Path,
    key_file: &Path,
    expected_id: Option<&str>,
    txid: Option<&str>,
) -> Result<NotesInspection> {
    let filter = txid
        .map(|value| {
            zcash_protocol::TxId::from_hex(value)
                .map(|txid| txid.to_string())
                .context("transaction ID must contain 64 hexadecimal digits")
        })
        .transpose()?;
    let snapshot = crate::rotation::store::inspect_note_snapshot(dir, key_file, expected_id)?;
    let mut reader = Cursor::new(snapshot.bytes.as_slice());
    let wallet = LightWallet::read(&mut reader, zingolib::config::ChainType::Mainnet)
        .map_err(|_| anyhow::anyhow!("corrupt or incompatible treasury snapshot"))?;
    ensure!(
        reader.position() == snapshot.bytes.len() as u64,
        "trailing bytes in treasury snapshot"
    );
    let mut result = evidence(&wallet, filter.as_deref())?;
    result.treasury_id = snapshot.treasury_id;
    result.snapshot_revision = snapshot.revision;
    Ok(result)
}

#[derive(Serialize)]
pub(crate) struct NativeReceipt {
    received_zatoshis: u64,
    pub(crate) confirmed_zatoshis: u64,
    spendable_zatoshis: Option<u64>,
    snapshot_revision: i64,
    sdk_scanned_height: Option<u32>,
    sdk_min_confirmations: u32,
    spendability_limitations: [&'static str; 2],
    spendability_unavailable_reason: Option<&'static str>,
    notes: Vec<NoteEvidence>,
}

/// Match the actual SDK note receiver, independently of sender memo metadata.
pub(crate) fn native_receipt(
    wallet: &LightWallet,
    address: &str,
    revision: i64,
) -> Result<NativeReceipt> {
    use zcash_keys::address::Address;
    let chain = wallet.chain_type();
    let bound = Address::decode(&chain, address).context("invalid native receiver")?;
    ensure!(
        matches!(&bound, Address::Unified(ua) if ua.transparent().is_none() && ua.orchard().is_some()),
        "native receiver must contain the Orchard-format receiver used by Ironwood"
    );
    let inspection = evidence_for_receiver(wallet, None, Some(&bound))?;
    let mut received = 0u64;
    let mut confirmed = 0u64;
    let mut spendable = inspection
        .spendability_unavailable_reason
        .is_none()
        .then_some(0u64);
    for note in &inspection.notes {
        if note.transaction.status != "failed" {
            received = received
                .checked_add(note.value_zatoshis)
                .context("native receipt amount overflow")?;
        }
        if note.transaction.status == "confirmed"
            && note
                .transaction
                .saved_confirmations
                .is_some_and(|count| count >= inspection.sdk_min_confirmations)
        {
            confirmed = confirmed
                .checked_add(note.value_zatoshis)
                .context("native receipt amount overflow")?;
        }
        if note.sdk_spendable_at_snapshot == Some(true) {
            spendable = spendable
                .map(|sum| {
                    sum.checked_add(note.value_zatoshis)
                        .context("native spendable amount overflow")
                })
                .transpose()?;
        }
    }
    Ok(NativeReceipt {
        received_zatoshis: received,
        confirmed_zatoshis: confirmed,
        spendable_zatoshis: spendable,
        snapshot_revision: revision,
        sdk_scanned_height: inspection.sdk_scanned_height,
        sdk_min_confirmations: inspection.sdk_min_confirmations,
        spendability_limitations: inspection.spendability_limitations,
        spendability_unavailable_reason: inspection.spendability_unavailable_reason,
        notes: inspection.notes,
    })
}

struct Selection {
    notes: Option<BTreeSet<OutputRef>>,
    target_height: Option<u32>,
    anchor_height: Option<u32>,
    unavailable_reason: Option<&'static str>,
}

fn selection(wallet: &LightWallet) -> Selection {
    let unavailable = |reason| Selection {
        notes: None,
        target_height: None,
        anchor_height: None,
        unavailable_reason: Some(reason),
    };
    if wallet
        .sync_state
        .last_known_chain_height()
        .is_some_and(|height| u32::from(height) == u32::MAX)
    {
        return unavailable("saved_target_height_unrepresentable");
    }
    // Avoid the SDK's target-minus-confirmations subtraction without its precondition.
    if wallet
        .sync_state
        .last_known_chain_height()
        .is_some_and(|height| {
            u64::from(u32::from(height)) + 1
                < u64::from(wallet.wallet_settings.min_confirmations.get())
        })
    {
        return unavailable("insufficient_saved_chain_height");
    }
    let Ok(Some((target, anchor))) =
        wallet.get_target_and_anchor_heights(wallet.wallet_settings.min_confirmations)
    else {
        return unavailable("saved_target_or_anchor_unavailable");
    };
    let notes = wallet.select_spendable_notes(
        zip32::AccountId::ZERO,
        TargetValue::AllFunds(MaxSpendMode::MaxSpendable),
        &[ShieldedPool::Ironwood],
        target,
        ConfirmationsPolicy::new_symmetrical(wallet.wallet_settings.min_confirmations, false),
        &[],
        LockFilter::Unfiltered,
    );
    let (notes, unavailable_reason) = match notes {
        Ok(notes) => (
            Some(
                notes
                    .ironwood()
                    .iter()
                    .map(|note| *note.internal_note_id())
                    .collect(),
            ),
            None,
        ),
        Err(_) => (None, Some("saved_sdk_selection_unavailable")),
    };
    Selection {
        notes,
        target_height: Some(u32::from(target)),
        anchor_height: Some(u32::from(anchor)),
        unavailable_reason,
    }
}

fn evidence(wallet: &LightWallet, filter: Option<&str>) -> Result<NotesInspection> {
    evidence_for_receiver(wallet, filter, None)
}
fn evidence_for_receiver(
    wallet: &LightWallet,
    filter: Option<&str>,
    receiver: Option<&zcash_keys::address::Address>,
) -> Result<NotesInspection> {
    use zcash_keys::address::Address;
    ensure!(
        wallet
            .unified_key_store
            .contains_key(&zip32::AccountId::ZERO),
        "saved wallet has no account zero"
    );
    let selected = selection(wallet);
    let chain = wallet.chain_type();
    let mut notes = Vec::new();
    for transaction in wallet.wallet_transactions.values() {
        for note in transaction.ironwood_notes() {
            let matches = receiver.is_none_or(|address| match address {
                Address::Unified(expected) => expected.orchard() == Some(&note.note().recipient()),
                _ => false,
            });
            if matches && included(note, filter) {
                // Ironwood shares the Orchard address receiver, not its pool identity.
                let receiver = zcash_keys::address::UnifiedAddress::from_receivers(
                    Some(note.note().recipient()),
                    None,
                    None,
                    None,
                    None,
                )
                .context("could not encode saved Ironwood receiver")?
                .encode(&chain);
                notes.push(note_evidence(
                    wallet,
                    transaction,
                    note,
                    "ironwood",
                    receiver,
                    &selected,
                ));
            }
        }
    }
    notes.sort_by(|left, right| {
        (left.pool, &left.txid, left.output_index).cmp(&(
            right.pool,
            &right.txid,
            right.output_index,
        ))
    });
    Ok(NotesInspection {
        format_version: 1,
        treasury_id: String::new(),
        state_schema_version: 23,
        snapshot_revision: 0,
        sdk_scanned_height: wallet.sync_state.fully_scanned_height().map(u32::from),
        sdk_min_confirmations: wallet.wallet_settings.min_confirmations.get(),
        selection_target_height: selected.target_height,
        selection_anchor_height: selected.anchor_height,
        spendability_limitations: [
            "live_process_locks_unavailable",
            "migration_reservations_may_be_selected",
        ],
        spendability_unavailable_reason: selected.unavailable_reason,
        notes,
    })
}

fn included(note: &impl OutputInterface<KeyId = KeyId>, filter: Option<&str>) -> bool {
    note.key_id().account_id == zip32::AccountId::ZERO
        && filter.is_none_or(|txid| txid == note.output_id().txid().to_string())
}

fn note_evidence<N: NoteInterface<KeyId = KeyId>>(
    wallet: &LightWallet,
    transaction: &WalletTransaction,
    note: &N,
    pool: &'static str,
    receiver: String,
    selected: &Selection,
) -> NoteEvidence {
    let spending = note.spending_transaction();
    let spender = spending.and_then(|txid| wallet.wallet_transactions.get(&txid));
    let spend_status = match spender {
        Some(tx) if tx.status().is_confirmed() => "spent",
        Some(tx) if tx.status().is_pending() => "pending_spend",
        Some(tx) if tx.status().is_failed() => "failed_spend",
        _ if spending.is_some() => "unknown",
        _ => "unspent",
    };
    NoteEvidence {
        pool,
        txid: note.output_id().txid().to_string(),
        output_index: note.output_id().output_index(),
        account_id: u32::from(note.key_id().account_id),
        scope: match note.key_id().scope {
            zip32::Scope::External => "external",
            zip32::Scope::Internal => "internal",
        },
        value_zatoshis: note.value(),
        receiver,
        receiver_encoding: "orchard_receiver_only_ua",
        transaction: transaction_evidence(wallet, transaction),
        spend_status,
        spending_txid: spending.map(|txid| txid.to_string()),
        spending_transaction: spender.map(|tx| transaction_evidence(wallet, tx)),
        sdk_spendable_at_snapshot: selected
            .notes
            .as_ref()
            .map(|notes| notes.contains(&OutputRef::new(note.output_id(), N::POOL_TYPE))),
    }
}

fn transaction_evidence(
    wallet: &LightWallet,
    transaction: &WalletTransaction,
) -> TransactionEvidence {
    let status = transaction.status();
    let confirmed = status.get_confirmed_height();
    TransactionEvidence {
        status: if status.is_confirmed() {
            "confirmed"
        } else if status.is_pending() {
            "pending"
        } else {
            "failed"
        },
        confirmed_height: confirmed.map(u32::from),
        saved_confirmations: confirmed.and_then(|height| {
            wallet
                .sync_state
                .fully_scanned_height()
                .and_then(|scanned| u32::from(scanned).checked_sub(u32::from(height)))
                .and_then(|distance| distance.checked_add(1))
        }),
        retained_block_hash: confirmed
            .and_then(|height| wallet.wallet_blocks.get(&height))
            .map(|block| block.block_hash().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::Store;
    use std::{io::Write, path::PathBuf};
    use zingolib::{
        config::{ChainType, WalletConfig},
        wallet::WalletSettings,
    };

    // Public unfunded BIP39 fixture; never use operator state in these tests.
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const MEMO: &str = "private fixture memo must never appear in note evidence";

    fn fixture() -> (tempfile::TempDir, Store, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let key = directory.path().join("key");
        let mut wallet = LightWallet::new(
            ChainType::Mainnet,
            WalletConfig::MnemonicPhrase {
                mnemonic_phrase: SEED.into(),
                no_of_accounts: 1.try_into().unwrap(),
                birthday: 3_543_405,
                wallet_settings: WalletSettings::default(),
            },
        )
        .unwrap();
        let bytes = wallet.save().unwrap().unwrap();
        let store = Store::create(&state, &key, 3_543_405, &bytes).unwrap();
        (directory, store, state, key)
    }

    fn saved_rows(state: &Path) -> serde_json::Value {
        let db = rusqlite::Connection::open_with_flags(
            state.join("state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let snapshots = db
            .prepare("SELECT revision,hex(bytes) FROM snapshots ORDER BY revision")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let outgoing = db
            .prepare("SELECT id,state,hex(raw),revision,hex(preparation) FROM outgoing ORDER BY id")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let identity: (String, String, u32) = db
            .query_row("SELECT id,network,account FROM instance", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        serde_json::json!({"snapshots":snapshots,"outgoing":outgoing,"schema":version,"identity":identity})
    }

    #[test]
    fn authenticated_notes_ignore_unrelated_reporting_and_preserve_owned_state() {
        let (_directory, store, state, key) = fixture();
        let (revision, bytes) = store.snapshot().unwrap();
        let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
        // These deliberately undecodable reporting records are outside note evidence.
        db.execute(
            "INSERT INTO treasury_sync(singleton,observation) VALUES (1,?1)",
            [MEMO],
        )
        .unwrap();
        db.execute("INSERT INTO outgoing(id,state,raw,revision,preparation) VALUES ('accepted-fixture','PREPARED',X'0102',?1,X'0304')", [revision]).unwrap();
        let before = saved_rows(&state);
        let key_before = std::fs::read(&key).unwrap();
        let database_before = std::fs::read(state.join("state.sqlite")).unwrap();
        assert!(crate::rotation::store::status(&state).is_err());
        let notes = inspect_notes(&state, &key, Some(store.id()), None).unwrap();
        assert_eq!(notes.snapshot_revision, revision);
        assert_eq!(notes.treasury_id, store.id());
        assert!(notes.notes.is_empty());
        assert!(notes.spendability_unavailable_reason.is_some());
        let output = serde_json::to_string(&notes).unwrap();
        assert!(!output.contains(SEED));
        assert!(!output.contains(MEMO));
        assert!(!output.contains("accepted-fixture"));
        assert_eq!(
            output,
            serde_json::to_string(&inspect_notes(&state, &key, Some(store.id()), None).unwrap())
                .unwrap()
        );
        assert_eq!(saved_rows(&state), before);
        assert_eq!(std::fs::read(&key).unwrap(), key_before);
        assert_eq!(
            std::fs::read(state.join("state.sqlite")).unwrap(),
            database_before
        );
        let (after_revision, after_bytes) = store.snapshot().unwrap();
        assert_eq!(after_revision, revision);
        assert_eq!(after_bytes.as_slice(), bytes.as_slice());
        // The original Store still holds exclusive ownership throughout inspection.
    }

    #[test]
    fn invalid_authentication_schema_and_snapshot_fail_without_repair() {
        let (directory, mut store, state, key) = fixture();
        let wrong_key = directory.path().join("wrong-key");
        crate::private_fs::create_file(&wrong_key)
            .unwrap()
            .write_all(&[81; 32])
            .unwrap();
        for (selected_key, identity) in [
            (&wrong_key, store.id()),
            (&key, "11111111-1111-4111-8111-111111111111"),
        ] {
            let before = saved_rows(&state);
            assert!(inspect_notes(&state, selected_key, Some(identity), None).is_err());
            assert_eq!(saved_rows(&state), before);
        }
        let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
        db.execute_batch("PRAGMA user_version=22").unwrap();
        let before = saved_rows(&state);
        assert!(inspect_notes(&state, &key, Some(store.id()), None).is_err());
        assert_eq!(saved_rows(&state), before);
        db.execute_batch("PRAGMA user_version=23").unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        for (change, restore) in [
            (
                "UPDATE instance SET network='testnet'",
                "UPDATE instance SET network='mainnet'",
            ),
            (
                "UPDATE instance SET account=1",
                "UPDATE instance SET account=0",
            ),
        ] {
            db.execute_batch(change).unwrap();
            let before = saved_rows(&state);
            assert!(inspect_notes(&state, &key, Some(store.id()), None).is_err());
            assert_eq!(saved_rows(&state), before);
            db.execute_batch(restore).unwrap();
        }
        // Authenticated encryption can contain incompatible SDK bytes: never repair them.
        let (revision, _) = store.snapshot().unwrap();
        store
            .save_snapshot(revision, b"invalid sdk snapshot")
            .unwrap();
        let before = saved_rows(&state);
        let error = inspect_notes(&state, &key, Some(store.id()), None)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "corrupt or incompatible treasury snapshot"
        );
        assert_eq!(saved_rows(&state), before);
        assert!(inspect_notes(&state, &key, Some(store.id()), Some("invalid")).is_err());
        assert_eq!(saved_rows(&state), before);
        db.execute("DELETE FROM snapshots", []).unwrap();
        let before = saved_rows(&state);
        assert!(inspect_notes(&state, &key, Some(store.id()), None).is_err());
        assert_eq!(saved_rows(&state), before);
    }

    #[cfg(feature = "zcash-testutils")]
    mod sdk {
        use super::*;
        use pepper_sync::wallet::{IronwoodNote, OutputId};
        use zcash_protocol::{TxId, memo::Memo};
        use zingo_status::confirmation_status::ConfirmationStatus;
        use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

        fn funded() -> LightWallet {
            SyntheticWalletBuilder::new(SEED)
                .tip(30)
                .ironwood_note(70_000)
                .ironwood_note(5_000)
                .build()
        }
        fn receiver(wallet: &LightWallet) -> String {
            let note = wallet
                .wallet_transactions
                .values()
                .flat_map(|tx| tx.ironwood_notes())
                .next()
                .unwrap();
            zcash_keys::address::UnifiedAddress::from_receivers(
                Some(note.note().recipient()),
                None,
                None,
                None,
                None,
            )
            .unwrap()
            .encode(&wallet.chain_type())
        }

        #[test]
        fn ironwood_receivers_and_membership_match_sdk_without_memos() {
            let mut wallet = funded();
            let transaction = wallet
                .wallet_transactions
                .values_mut()
                .find(|tx| tx.ironwood_notes().iter().any(|n| n.value() == 70_000))
                .unwrap();
            let note = transaction.ironwood_notes_mut().remove(0);
            *note = IronwoodNote::new_for_test(
                note.output_id(),
                note.key_id().account_id,
                note.key_id().scope,
                *note.note(),
                MEMO.parse::<Memo>().unwrap(),
                note.position(),
            )
            .with_nullifier_for_test(note.nullifier().unwrap());
            let result = evidence(&wallet, None).unwrap();
            assert_eq!(result.notes.len(), 2);
            assert_eq!(result.spendability_unavailable_reason, None);
            for note in &result.notes {
                assert_eq!(note.pool, "ironwood");
                assert_eq!(
                    note.sdk_spendable_at_snapshot,
                    Some(note.value_zatoshis > 5_000)
                );
                assert_eq!(note.scope, "external");
                assert_eq!(note.account_id, 0);
                assert_eq!(note.transaction.status, "confirmed");
                assert_eq!(note.spend_status, "unspent");
                assert!(note.receiver.starts_with("uregtest"));
                let original = wallet
                    .wallet_transactions
                    .values()
                    .find(|tx| tx.txid().to_string() == note.txid)
                    .unwrap();
                assert_eq!(
                    note.transaction.saved_confirmations,
                    Some(31 - u32::from(original.status().get_confirmed_height().unwrap()))
                );
            }
            let encoded = serde_json::to_string(&result).unwrap();
            assert!(
                !encoded.contains(SEED)
                    && !encoded.contains(MEMO)
                    && !encoded.contains("nullifier")
            );
            assert_eq!(
                encoded,
                serde_json::to_string(&evidence(&wallet, None).unwrap()).unwrap()
            );
            let filtered = evidence(&wallet, Some(&result.notes[0].txid)).unwrap();
            assert_eq!(filtered.notes.len(), 1);
            assert_eq!(filtered.notes[0].txid, result.notes[0].txid);
        }

        #[test]
        fn native_receipts_count_only_ironwood_and_retain_historical_spent_notes() {
            // Existing SDK snapshots may decode other pools. They grant neither
            // native receipt credit nor application note-inspection support.
            let mut wallet = SyntheticWalletBuilder::new(SEED)
                .tip(30)
                .ironwood_note(70_000)
                .sapling_note(50_000)
                .orchard_note(100_000)
                .build();
            let address = receiver(&wallet);
            let ironwood_receiver = wallet
                .wallet_transactions
                .values()
                .flat_map(|tx| tx.ironwood_notes())
                .next()
                .unwrap()
                .note()
                .recipient();
            let unsupported = wallet
                .wallet_transactions
                .values()
                .flat_map(|tx| tx.orchard_notes())
                .next()
                .unwrap();
            assert_eq!(
                unsupported.note().recipient(),
                ironwood_receiver,
                "same address encoding cannot credit a different pool"
            );
            let before = native_receipt(&wallet, &address, 4).unwrap();
            assert_eq!(before.received_zatoshis, 70_000);
            assert_eq!(before.confirmed_zatoshis, 70_000);
            assert_eq!(before.spendable_zatoshis, Some(70_000));
            assert_eq!(before.notes.len(), 1);
            assert_eq!(evidence(&wallet, None).unwrap().notes.len(), 1);
            assert_eq!(before.notes[0].pool, "ironwood");
            let spent = TxId::from_bytes([91; 32]);
            for transaction in wallet.wallet_transactions.values_mut() {
                for note in transaction.ironwood_notes_mut() {
                    note.set_spending_transaction(Some(spent));
                }
            }
            wallet.wallet_transactions.insert(
                spent,
                WalletTransaction::new_for_test(spent, ConfirmationStatus::Confirmed(25.into())),
            );
            let historical = native_receipt(&wallet, &address, 5).unwrap();
            assert_eq!(historical.received_zatoshis, 70_000);
            assert_eq!(historical.confirmed_zatoshis, 70_000);
            assert_eq!(historical.spendable_zatoshis, Some(0));
            assert!(historical.notes.iter().all(|n| n.spend_status == "spent"));
        }

        #[test]
        fn encrypted_mainnet_snapshot_preserves_ironwood_evidence_while_owner_is_held() {
            let (_directory, mut store, state, key) = fixture();
            let mut synthetic = funded();
            let (_, original) = store.snapshot().unwrap();
            let mut wallet = LightWallet::read(original.as_slice(), ChainType::Mainnet).unwrap();
            wallet.wallet_transactions = std::mem::take(&mut synthetic.wallet_transactions);
            wallet.wallet_blocks = std::mem::take(&mut synthetic.wallet_blocks);
            wallet.wallet_settings.min_confirmations = synthetic.wallet_settings.min_confirmations;
            wallet.sync_state = synthetic.sync_state;
            wallet.shard_trees = synthetic.shard_trees;
            let mut bytes = Vec::new();
            wallet.write(&mut bytes, &ChainType::Mainnet).unwrap();
            let (revision, _) = store.snapshot().unwrap();
            store.save_snapshot(revision, &bytes).unwrap();
            let before = saved_rows(&state);
            let key_before = std::fs::read(&key).unwrap();
            let notes = inspect_notes(&state, &key, Some(store.id()), None).unwrap();
            let mut identities = notes
                .notes
                .iter()
                .map(|n| (n.pool, n.value_zatoshis))
                .collect::<Vec<_>>();
            identities.sort_unstable();
            assert_eq!(identities, vec![("ironwood", 5_000), ("ironwood", 70_000)]);
            assert!(notes.notes.iter().all(|n| n.receiver.starts_with('u')));
            assert_eq!(saved_rows(&state), before);
            assert_eq!(std::fs::read(&key).unwrap(), key_before);
        }

        #[test]
        fn spent_pending_and_incomplete_ironwood_notes_keep_sdk_selection_evidence() {
            let mut wallet = funded();
            let spent = TxId::from_bytes([81; 32]);
            let pending = TxId::from_bytes([82; 32]);
            for tx in wallet.wallet_transactions.values_mut() {
                for note in tx.ironwood_notes_mut() {
                    note.set_spending_transaction(Some(if note.value() == 70_000 {
                        spent
                    } else {
                        pending
                    }));
                }
            }
            wallet.wallet_transactions.insert(
                spent,
                WalletTransaction::new_for_test(spent, ConfirmationStatus::Confirmed(25.into())),
            );
            wallet.wallet_transactions.insert(
                pending,
                WalletTransaction::new_for_test(pending, ConfirmationStatus::Mempool(31.into())),
            );
            let source = wallet
                .wallet_transactions
                .values()
                .flat_map(|tx| tx.ironwood_notes())
                .find(|n| n.value() == 70_000)
                .unwrap();
            let incomplete_id = TxId::from_bytes([83; 32]);
            let unconfirmed_id = TxId::from_bytes([84; 32]);
            let incomplete = IronwoodNote::new_for_test(
                OutputId::new(incomplete_id, 0),
                zip32::AccountId::ZERO,
                zip32::Scope::Internal,
                *source.note(),
                Memo::Empty,
                None,
            );
            let unconfirmed = IronwoodNote::new_for_test(
                OutputId::new(unconfirmed_id, 0),
                zip32::AccountId::ZERO,
                zip32::Scope::External,
                *source.note(),
                Memo::Empty,
                source.position(),
            )
            .with_nullifier_for_test(source.nullifier().unwrap());
            wallet.wallet_transactions.insert(
                incomplete_id,
                WalletTransaction::new_for_test_with_ironwood_notes(
                    incomplete_id,
                    ConfirmationStatus::Confirmed(26.into()),
                    vec![incomplete],
                    vec![],
                ),
            );
            wallet.wallet_transactions.insert(
                unconfirmed_id,
                WalletTransaction::new_for_test_with_ironwood_notes(
                    unconfirmed_id,
                    ConfirmationStatus::Mempool(31.into()),
                    vec![unconfirmed],
                    vec![],
                ),
            );
            let result = evidence(&wallet, None).unwrap();
            assert_eq!(result.notes.len(), 4);
            let spent_note = result
                .notes
                .iter()
                .find(|n| {
                    n.value_zatoshis == 70_000
                        && n.txid != incomplete_id.to_string()
                        && n.txid != unconfirmed_id.to_string()
                })
                .unwrap();
            assert_eq!(spent_note.spend_status, "spent");
            assert_eq!(spent_note.spending_txid, Some(spent.to_string()));
            assert_eq!(spent_note.sdk_spendable_at_snapshot, Some(false));
            let pending_note = result
                .notes
                .iter()
                .find(|n| n.value_zatoshis == 5_000)
                .unwrap();
            assert_eq!(pending_note.spend_status, "pending_spend");
            assert_eq!(
                pending_note.spending_transaction.as_ref().unwrap().status,
                "pending"
            );
            let incomplete = result
                .notes
                .iter()
                .find(|n| n.txid == incomplete_id.to_string())
                .unwrap();
            assert_eq!(incomplete.scope, "internal");
            assert_eq!(incomplete.sdk_spendable_at_snapshot, Some(false));
            let unconfirmed = result
                .notes
                .iter()
                .find(|n| n.txid == unconfirmed_id.to_string())
                .unwrap();
            assert_eq!(unconfirmed.transaction.status, "pending");
            assert_eq!(unconfirmed.transaction.confirmed_height, None);
            assert_eq!(unconfirmed.transaction.saved_confirmations, None);
            assert_eq!(unconfirmed.sdk_spendable_at_snapshot, Some(false));
            wallet.sync_state = pepper_sync::wallet::SyncState::new();
            let unavailable = evidence(&wallet, None).unwrap();
            assert!(unavailable.spendability_unavailable_reason.is_some());
            assert!(
                unavailable
                    .notes
                    .iter()
                    .all(|n| n.sdk_spendable_at_snapshot.is_none())
            );
            assert_eq!(unavailable.notes.len(), 4);
        }
    }
}
