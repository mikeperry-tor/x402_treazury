//! Durable allocation identities and accepted source ownership for supervised
//! funding. These records never authorize allocation, signing or broadcast.
pub mod registry;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS funding_permits(
 intent TEXT PRIMARY KEY,
 run TEXT NOT NULL REFERENCES runs(id),
 pool TEXT NOT NULL,
 job TEXT UNIQUE,
 at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS funding_source_attempts(
 operation TEXT PRIMARY KEY,
 job TEXT NOT NULL REFERENCES funding_permits(job),
 input_zatoshis INTEGER NOT NULL CHECK(input_zatoshis>0),
 released INTEGER NOT NULL DEFAULT 0 CHECK(released IN (0,1))
);
";

#[derive(Clone, Debug)]
pub struct Request {
    /// Stable before allocation; retries must reuse it, including after a crash.
    pub intent: String,
    pub pool: String,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub jobs: u32,
}

/// Caller must validate the session, current registry authority, run/pool binding,
/// and execution window under this same transaction before invoking `reserve`.
/// This function never begins/commits a transaction on the caller's behalf.
pub fn reserve(
    tx: &mut rusqlite::Transaction<'_>,
    run: &str,
    requests: &[Request],
    run_limit: Limits,
    cumulative_limit: Limits,
    at: i64,
) -> Result<()> {
    let savepoint = tx.savepoint()?;
    reserve_inner(&savepoint, run, requests, run_limit, cumulative_limit, at)?;
    savepoint.commit()?;
    Ok(())
}
fn reserve_inner(
    tx: &Connection,
    run: &str,
    requests: &[Request],
    run_limit: Limits,
    cumulative_limit: Limits,
    at: i64,
) -> Result<()> {
    ensure!(
        !requests.is_empty() && requests.len() <= 1000,
        "funding permit batch must contain 1..1000 allocations"
    );
    ensure!(at >= 0, "invalid funding reservation timestamp");
    let mut intents = std::collections::BTreeSet::new();
    for request in requests {
        ensure!(
            super::identifier(&request.intent) && super::identifier(&request.pool),
            "invalid funding allocation identity"
        );
        ensure!(
            intents.insert(&request.intent),
            "duplicate funding allocation intent in batch"
        );
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT run,pool FROM funding_permits WHERE intent=?1",
                [&request.intent],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some(prior) = prior {
            ensure!(
                prior == (run.to_owned(), request.pool.clone()),
                "funding allocation intent binding changed"
            );
        } else {
            tx.execute(
                "INSERT INTO funding_permits(intent,run,pool,at) VALUES(?1,?2,?3,?4)",
                params![request.intent, run, request.pool, at],
            )?;
        }
    }
    for (scope, limit) in [(Some(run), run_limit), (None, cumulative_limit)] {
        let jobs: i64 = tx.query_row(
            "SELECT COUNT(*) FROM funding_permits WHERE ?1 IS NULL OR run=?1",
            [scope],
            |r| r.get(0),
        )?;
        ensure!(
            jobs >= 0 && jobs <= i64::from(limit.jobs),
            "qualification_funding_denied: allocation count exceeds funding-job limit; orphan allocations remain counted"
        );
    }
    Ok(())
}

/// Bind the immutable planned job ID. This is not proof that treasury allocation
/// committed. All permits remain charged; recovery must find the stable intent,
/// never delete an orphan or infer a completed allocation from this field.
pub fn link(db: &mut Connection, run: &str, intent: &str, job: &str) -> Result<()> {
    ensure!(super::identifier(job), "invalid funding job identity");
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT job FROM funding_permits WHERE run=?1 AND intent=?2",
            params![run, intent],
            |r| r.get(0),
        )
        .context("funding permit missing")?;
    ensure!(
        existing.as_deref().is_none_or(|prior| prior == job),
        "funding job binding changed"
    );
    tx.execute(
        "UPDATE funding_permits SET job=?1 WHERE run=?2 AND intent=?3",
        params![job, run, intent],
    )?;
    tx.commit()?;
    Ok(())
}

/// Check the immutable allocation identity before preparing source bytes.
/// Funding capital is governed by SDK spendability, not a ZEC test budget.
pub fn check_preparation(db: &Connection, run: &str, job: &str) -> Result<()> {
    let present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_permits WHERE run=?1 AND job=?2)",
        params![run, job],
        |r| r.get(0),
    )?;
    ensure!(present, "funding permit missing for preparation");
    Ok(())
}

/// Bind source identity before Store acceptance and calculation. The input amount
/// records the first proposal for compatible historical reporting, not authority
/// or proof of acceptance. A failed Store commit may leave only this observation;
/// the Store owns exact accepted amounts and inputs, including on retries.
pub fn reserve_source(
    tx: &mut rusqlite::Transaction<'_>,
    run: &str,
    job: &str,
    operation: &str,
    input_with_fee: u64,
) -> Result<()> {
    ensure!(
        super::identifier(operation),
        "invalid source operation identity"
    );
    let savepoint = tx.savepoint()?;
    check_preparation(&savepoint, run, job)?;
    let input = i64::try_from(input_with_fee)?;
    ensure!(input > 0, "funding input must be positive");
    let existing: Option<(String, bool)> = savepoint
        .query_row(
            "SELECT job,released FROM funding_source_attempts WHERE operation=?1",
            [operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        ensure!(
            existing == (job.to_owned(), false),
            "qualification source operation binding changed or retired"
        );
    } else {
        savepoint.execute(
            "INSERT INTO funding_source_attempts(operation,job,input_zatoshis) VALUES(?1,?2,?3)",
            params![operation, job, input],
        )?;
    }
    savepoint.commit()?;
    Ok(())
}
/// Only the exclusive treasury owner may supply this proof, after committing
/// archival of the old operation and verifying no signed bytes or consumed funds.
/// The tombstone prevents the old operation from being accepted again.
pub fn retire_unprepared(db: &Connection, run: &str, job: &str, operation: &str) -> Result<()> {
    let owned: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_permits WHERE run=?1 AND job=?2)",
        params![run, job],
        |r| r.get(0),
    )?;
    ensure!(owned, "qualification funding job missing");
    let other: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_source_attempts WHERE operation=?1 AND job!=?2)",
        params![operation, job],
        |r| r.get(0),
    )?;
    ensure!(
        !other,
        "qualification source operation belongs to another job"
    );
    db.execute(
        "UPDATE funding_source_attempts SET released=1 WHERE operation=?1 AND job=?2",
        params![operation, job],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db(path: &std::path::Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        db.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY); INSERT OR IGNORE INTO runs VALUES('run'); INSERT OR IGNORE INTO runs VALUES('next');").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db
    }
    fn request(intent: &str) -> Request {
        Request {
            intent: intent.into(),
            pool: "pool".into(),
        }
    }
    fn reserve_commit(
        db: &mut Connection,
        run: &str,
        requests: &[Request],
        limit: Limits,
    ) -> Result<()> {
        let mut tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reserve(&mut tx, run, requests, limit, limit, 1)?;
        tx.commit()?;
        Ok(())
    }
    #[test]
    fn bootstrap_batch_is_atomic_orphans_persist_and_job_bindings_are_immutable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry");
        let mut conn = db(&path);
        let limit = Limits { jobs: 2 };
        assert!(
            reserve_commit(
                &mut conn,
                "run",
                &[request("a"), request("b"), request("c")],
                limit
            )
            .is_err()
        );
        reserve_commit(&mut conn, "run", &[request("a"), request("b")], limit).unwrap();
        drop(conn);
        let mut conn = db(&path);
        reserve_commit(&mut conn, "run", &[request("a"), request("b")], limit).unwrap();
        assert!(reserve_commit(&mut conn, "next", &[request("c")], limit).is_err());
        let mut changed = request("a");
        changed.pool = "other".into();
        assert!(reserve_commit(&mut conn, "run", &[changed], limit).is_err());
        assert!(check_preparation(&conn, "run", "job").is_err());
        link(&mut conn, "run", "a", "job").unwrap();
        link(&mut conn, "run", "a", "job").unwrap();
        assert!(link(&mut conn, "run", "a", "other").is_err());
        assert!(link(&mut conn, "run", "b", "job").is_err());
        check_preparation(&conn, "run", "job").unwrap();
        assert!(check_preparation(&conn, "next", "job").is_err());
    }
    #[test]
    fn rejected_batch_cannot_leak_reservations_when_caller_commits() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db(&dir.path().join("registry"));
        let limit = Limits { jobs: 1 };
        let mut tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            reserve(
                &mut tx,
                "run",
                &[request("a"), request("b")],
                limit,
                limit,
                1
            )
            .is_err()
        );
        tx.commit().unwrap();
        reserve_commit(&mut conn, "run", &[request("c")], limit).unwrap();
    }
    #[test]
    fn competing_connections_cannot_both_take_final_permit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry");
        drop(db(&path));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|intent| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut conn = db(&path);
                    barrier.wait();
                    reserve_commit(&mut conn, "run", &[request(intent)], Limits { jobs: 1 }).is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }
}
