use super::*;
use rusqlite::params;
fn fixture() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("owner.lock");
    File::create(&lock).unwrap();
    let path = dir.path().join("state.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../../tests/fixtures/migration/privacy_schema12.sql"
    ))
    .unwrap();
    db.execute(
        "INSERT INTO instance VALUES ('fixture',1,1,'mainnet',0)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO snapshots VALUES (1,?1)",
        [b"synthetic encrypted treasury bytes".as_slice()],
    )
    .unwrap();
    for g in 0..3 {
        let pool = format!("pool{g}");
        db.execute(
            "INSERT INTO pools(id,name,target,bootstrapped) VALUES (?1,?1,'5000000',1)",
            [&pool],
        )
        .unwrap();
        for i in 0..2 {
            let id = format!("wallet{g}_{i}");
            let job = format!("job{g}_{i}");
            let op = format!("source{g}_{i}");
            db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,block_hash,block_height) VALUES (?1,?2,?3,?4,?5,'5000000',?6,'5000000',?7,100)",params![id,pool,i,format!("0x{:040x}",g*2+i+1),b"synthetic key ciphertext".as_slice(),if i==0{"ACTIVE"}else{"READY"},format!("0x{:064x}",100)]).unwrap();
            db.execute(
                "INSERT INTO funding_jobs VALUES (?1,?2,'COMPLETE','5000000')",
                params![job, id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO budget_entries VALUES (?1,?2,1,1,100,0,100)",
                params![op, pool],
            )
            .unwrap();
            db.execute(
                "INSERT INTO outgoing VALUES (?1,'RESOLVED',?2,1)",
                params![op, b"synthetic accepted bytes".as_slice()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO treasury_operations VALUES (?1,'{}','CONFIRMED',1)",
                [&op],
            )
            .unwrap();
            db.execute("INSERT INTO funding_progress(job_id,operation_id,phase,quote) VALUES (?1,?2,'\"VERIFYING_CREDIT\"',?3)",params![job,op,b"synthetic quote ciphertext".as_slice()]).unwrap();
            db.execute(
                "INSERT INTO funding_refunds VALUES (?1,?2)",
                params![job, b"synthetic refund ciphertext".as_slice()],
            )
            .unwrap();
            if g * 2 + i < 5 {
                let attempt = format!("payment{g}_{i}");
                db.execute("INSERT INTO payment_attempts VALUES (?1,?2,?3,0,'1','fixture','RESOLVED','payer','payee',?4,0,10)",params![attempt,pool,id,format!("0x{:064x}",g*2+i)]).unwrap();
                db.execute(
                    "INSERT INTO payment_resolutions VALUES (?1,'USED',100,?2,10)",
                    params![attempt, format!("0x{:064x}", 100)],
                )
                .unwrap();
            }
        }
    }
    for i in 0..2 {
        let op = format!("archive{i}");
        db.execute(
            "INSERT INTO funding_recovery VALUES (?1,'job0_0',?2,?3,?4)",
            params![
                op,
                if i == 0 {
                    "\"QUOTED\""
                } else {
                    "\"RECOVERY_REQUIRED\""
                },
                b"historical quote".as_slice(),
                b"historical refund".as_slice()
            ],
        )
        .unwrap();
        if i == 1 {
            db.execute(
                "INSERT INTO budget_entries VALUES (?1,'pool0',1,1,100,0,0)",
                [&op],
            )
            .unwrap();
            db.execute(
                "INSERT INTO outgoing VALUES (?1,'RESOLVED',?2,1)",
                params![op, b"expired accepted bytes".as_slice()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO treasury_operations VALUES (?1,'{}','PREPARED',0)",
                [&op],
            )
            .unwrap();
            db.execute("INSERT INTO expired_operations VALUES (?1,100,1)", [&op])
                .unwrap();
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        for p in [&path, &lock] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    (dir, db)
}
#[test]
fn known_source_rejects_before_mutation_reopen_and_repeat() {
    let (dir, db) = fixture();
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM funding_jobs WHERE state='COMPLETE'"
        )
        .unwrap(),
        6
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM funding_progress WHERE phase='\"VERIFYING_CREDIT\"'"
        )
        .unwrap(),
        6
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM treasury_operations WHERE submission='CONFIRMED'"
        )
        .unwrap(),
        6
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM treasury_operations WHERE submission='PREPARED'"
        )
        .unwrap(),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM funding_recovery WHERE phase='\"QUOTED\"'"
        )
        .unwrap(),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM funding_recovery WHERE phase='\"RECOVERY_REQUIRED\"'"
        )
        .unwrap(),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM payment_resolutions WHERE outcome='USED'"
        )
        .unwrap(),
        5
    );
    drop(db);
    let path = dir.path().join("state.sqlite");
    let before = fs::read(&path).unwrap();
    for _ in 0..2 {
        assert_eq!(
            inspect(dir.path()).unwrap_err().to_string(),
            "legacy_destination_readiness_support_missing"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "every ciphertext, accepted byte, liability, source correlation and gross total remains unchanged"
        );
    }
    assert!(
        !dir.path().join("wallet.key").exists(),
        "preflight never loads or creates keys"
    );
}
#[test]
fn unresolved_complete_source_and_missing_resolution_are_diagnostics() {
    let (dir, db) = fixture();
    db.execute(
        "UPDATE treasury_operations SET submission='UNKNOWN' WHERE id='source0_0'",
        [],
    )
    .unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "legacy_complete_source_obligation_unresolved"
    );
    db.execute(
        "UPDATE treasury_operations SET submission='CONFIRMED' WHERE id='source0_0'",
        [],
    )
    .unwrap();
    db.execute(
        "DELETE FROM payment_resolutions WHERE attempt_id='payment0_0'",
        [],
    )
    .unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "legacy_payment_resolution_support_missing"
    );
}
#[test]
fn stopped_service_and_schema_guards_precede_inspection() {
    let (dir, db) = fixture();
    let owner = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("owner.lock"))
        .unwrap();
    owner.try_lock_exclusive().unwrap();
    assert_eq!(inspect(dir.path()).unwrap_err().to_string(), "state_in_use");
    FileExt::unlock(&owner).unwrap();
    db.pragma_update(None, "user_version", 11).unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "unsupported_source_schema"
    );
    db.pragma_update(None, "user_version", 16).unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "converted_schema_shape_invalid"
    );
}
#[test]
fn exclusive_ownership_does_not_authorize_deleting_unsigned_rows_or_forging_support() {
    let (dir, db) = fixture();
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES ('unsigned','pool0','wallet0_0',0,'1','fixture','ADMITTED')",[]).unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "legacy_destination_readiness_support_missing"
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM payment_attempts WHERE state='ADMITTED'"
        )
        .unwrap(),
        1
    );
    db.execute(
        "ALTER TABLE funding_jobs ADD COLUMN readiness_support TEXT",
        [],
    )
    .unwrap();
    assert_eq!(
        inspect(dir.path()).unwrap_err().to_string(),
        "unsupported_source_shape"
    );
}

#[test]
fn already_converted_is_recognized_without_transforming_or_granting_readiness() {
    let (dir, db) = fixture();
    // Structural recognition only, not a fake successful source conversion.
    db.execute_batch("CREATE TABLE payment_admission_evidence(payer_role TEXT,reserve_slot TEXT,claim_generation INTEGER); CREATE TABLE reserve_claims(generation INTEGER); ALTER TABLE pools ADD COLUMN payment_limit TEXT; ALTER TABLE funding_jobs ADD COLUMN readiness_support TEXT; ALTER TABLE wallets ADD COLUMN unresolved_exposure TEXT; PRAGMA user_version=16;").unwrap();
    drop(db);
    let before = fs::read(dir.path().join("state.sqlite")).unwrap();
    for _ in 0..2 {
        assert_eq!(inspect(dir.path()).unwrap(), Outcome::AlreadyConverted);
    }
    assert_eq!(before, fs::read(dir.path().join("state.sqlite")).unwrap());
}

#[test]
fn snapshot_preflight_reads_committed_wal_without_creating_source_sidecars() {
    let (source, db) = fixture();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; UPDATE treasury_operations SET submission='UNKNOWN' WHERE id='source0_0';").unwrap();
    let frozen = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(frozen.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    // Copy the synthetic committed snapshot while this sole fixture connection
    // is quiescent. The inspected destination has no open writer or WAL index.
    for name in ["owner.lock", "state.sqlite", "state.sqlite-wal"] {
        fs::copy(source.path().join(name), frozen.path().join(name)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(frozen.path().join(name), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
    }
    let before: Vec<_> = ["owner.lock", "state.sqlite", "state.sqlite-wal"]
        .map(|name| fs::read(frozen.path().join(name)).unwrap())
        .into();
    for _ in 0..2 {
        assert_eq!(
            inspect(frozen.path()).unwrap_err().to_string(),
            "legacy_complete_source_obligation_unresolved"
        );
        assert!(!frozen.path().join("state.sqlite-shm").exists());
    }
    let after: Vec<_> = ["owner.lock", "state.sqlite", "state.sqlite-wal"]
        .map(|name| fs::read(frozen.path().join(name)).unwrap())
        .into();
    assert_eq!(before, after);
}
