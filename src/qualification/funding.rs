//! Durable allocation reservations for supervised funding. This ledger is an
//! accounting restriction, not permission to allocate, sign, or broadcast.
pub mod registry;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS funding_permits(
 intent TEXT PRIMARY KEY,
 run TEXT NOT NULL REFERENCES runs(id),
 pool TEXT NOT NULL,
 source_bound INTEGER NOT NULL CHECK(source_bound>0),
 job TEXT UNIQUE,
 at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS funding_source_attempts(
 operation TEXT PRIMARY KEY,
 job TEXT NOT NULL REFERENCES funding_permits(job),
 source_bound INTEGER NOT NULL CHECK(source_bound>0),
 released INTEGER NOT NULL DEFAULT 0 CHECK(released IN (0,1))
);
";

#[derive(Clone, Debug)]
pub struct Request {
    /// Stable before allocation; retries must reuse it, including after a crash.
    pub intent: String,
    pub pool: String,
    /// Maximum source input INCLUDING fee, in zatoshis.
    pub source_bound: u64,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub jobs: u32,
    pub source_zatoshis: u64,
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
        let bound = i64::try_from(request.source_bound)?;
        ensure!(
            bound > 0,
            "funding permit must include positive input-plus-fee exposure"
        );
        let prior: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT run,pool,source_bound FROM funding_permits WHERE intent=?1",
                [&request.intent],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some(prior) = prior {
            ensure!(
                prior == (run.to_owned(), request.pool.clone(), bound),
                "funding allocation intent binding changed"
            );
        } else {
            tx.execute("INSERT INTO funding_permits(intent,run,pool,source_bound,at) VALUES(?1,?2,?3,?4,?5)", params![request.intent,run,request.pool,bound,at])?;
        }
    }
    for (scope, limit) in [(Some(run), run_limit), (None, cumulative_limit)] {
        let (jobs, source): (i64, i64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(source_bound),0) FROM funding_permits WHERE ?1 IS NULL OR run=?1",
            [scope], |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            jobs >= 0
                && jobs <= i64::from(limit.jobs)
                && u64::try_from(source)? <= limit.source_zatoshis,
            "qualification_funding_denied: allocation or input-plus-fee reservation exceeds funding limit; orphan reservations remain charged"
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

/// Check the immutable permit immediately before preparing source bytes. This
/// does not replace treasury budget, quote validity, or spend-readiness checks.
pub fn check_preparation(db: &Connection, run: &str, job: &str, input_with_fee: u64) -> Result<()> {
    let bound: i64 = db
        .query_row(
            "SELECT source_bound FROM funding_permits WHERE run=?1 AND job=?2",
            params![run, job],
            |r| r.get(0),
        )
        .context("funding permit missing for preparation")?;
    ensure!(
        input_with_fee > 0 && input_with_fee <= u64::try_from(bound)?,
        "qualification_funding_denied: preparation input plus fee exceeds immutable permit"
    );
    Ok(())
}

/// Reserve before transaction calculation. The same operation is idempotent;
/// another operation must fit the job's remaining source exposure. A crash
/// after this commit remains charged even without treasury-side evidence.
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
    check_preparation(&savepoint, run, job, input_with_fee)?;
    let bound = i64::try_from(input_with_fee)?;
    let existing: Option<(String, i64, bool)> = savepoint
        .query_row(
            "SELECT job,source_bound,released FROM funding_source_attempts WHERE operation=?1",
            [operation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        ensure!(
            existing == (job.to_owned(), bound, false),
            "qualification source operation binding changed or retired"
        );
    } else {
        savepoint.execute(
            "INSERT INTO funding_source_attempts(operation,job,source_bound) VALUES(?1,?2,?3)",
            params![operation, job, bound],
        )?;
    }
    let (used, limit): (i64, i64) = savepoint.query_row(
        "SELECT COALESCE(SUM(s.source_bound),0),p.source_bound FROM funding_permits p LEFT JOIN funding_source_attempts s ON s.job=p.job AND s.released=0 WHERE p.job=?1 GROUP BY p.intent",
        [job], |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        used <= limit,
        "qualification_funding_denied: source attempts exceed job input-plus-fee permit"
    );
    savepoint.commit()?;
    Ok(())
}
/// Only the exclusive treasury owner may supply this proof, after committing
/// archival of the old operation and verifying no signed bytes or consumed funds.
/// The tombstone prevents the old operation from claiming that allowance again.
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
    fn request(intent: &str, bound: u64) -> Request {
        Request {
            intent: intent.into(),
            pool: "pool".into(),
            source_bound: bound,
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
    fn bootstrap_batch_is_atomic_orphans_persist_and_fee_bounds_are_immutable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry");
        let mut conn = db(&path);
        let limit = Limits {
            jobs: 2,
            source_zatoshis: 100,
        };
        assert!(
            reserve_commit(
                &mut conn,
                "run",
                &[request("a", 50), request("b", 51)],
                limit
            )
            .is_err()
        );
        reserve_commit(
            &mut conn,
            "run",
            &[request("a", 50), request("b", 50)],
            limit,
        )
        .unwrap();
        drop(conn);
        let mut conn = db(&path);
        reserve_commit(
            &mut conn,
            "run",
            &[request("a", 50), request("b", 50)],
            limit,
        )
        .unwrap();
        assert!(reserve_commit(&mut conn, "next", &[request("c", 1)], limit).is_err());
        assert!(reserve_commit(&mut conn, "run", &[request("a", 49)], limit).is_err());
        assert!(check_preparation(&conn, "run", "job", 50).is_err());
        link(&mut conn, "run", "a", "job").unwrap();
        link(&mut conn, "run", "a", "job").unwrap();
        assert!(link(&mut conn, "run", "a", "other").is_err());
        assert!(link(&mut conn, "run", "b", "job").is_err());
        check_preparation(&conn, "run", "job", 50).unwrap();
        assert!(check_preparation(&conn, "run", "job", 51).is_err());
        assert!(check_preparation(&conn, "next", "job", 1).is_err());
    }
    #[test]
    fn rejected_batch_cannot_leak_reservations_when_caller_commits() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db(&dir.path().join("registry"));
        let limit = Limits {
            jobs: 1,
            source_zatoshis: 50,
        };
        let mut tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            reserve(
                &mut tx,
                "run",
                &[request("a", 50), request("b", 1)],
                limit,
                limit,
                1
            )
            .is_err()
        );
        tx.commit().unwrap();
        reserve_commit(&mut conn, "run", &[request("c", 50)], limit).unwrap();
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
                    reserve_commit(
                        &mut conn,
                        "run",
                        &[request(intent, 50)],
                        Limits {
                            jobs: 1,
                            source_zatoshis: 50,
                        },
                    )
                    .is_ok()
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
