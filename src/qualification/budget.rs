//! Qualification's USDC API budget, derived from durable admissions.
//! Callers check inside the same write transaction that accepts new work.
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;

pub fn atomic(text: &str) -> Result<u64> {
    anyhow::ensure!(!text.is_empty(), "test budget must be finite");
    let amount = crate::payment::SpendPolicy::dollars(text)?
        .max_atomic
        .context("test budget must be finite")?;
    Ok(amount.try_into()?)
}

/// Each potentially chargeable API authorization consumes budget independently.
/// Funding transfers and later settlement observations do not change this total.
pub fn used(db: &Connection, run: Option<&str>) -> Result<u64> {
    let mut total = 0_u64;
    let mut payments = db.prepare(
        "SELECT detail FROM events WHERE kind='application_payment' AND (?1 IS NULL OR run=?1)",
    )?;
    for raw in payments.query_map([run], |r| r.get::<_, String>(0))? {
        let event: Value = serde_json::from_str(&raw?)?;
        let amount: u64 = event["amount"]
            .as_str()
            .context("payment budget amount missing")?
            .parse()?;
        total = total
            .checked_add(amount)
            .context("test budget total overflow")?;
    }
    Ok(total)
}

pub(crate) fn check(db: &Connection, run: &str) -> Result<()> {
    let raw: String = db.query_row("SELECT manifest FROM runs WHERE id=?1", [run], |r| r.get(0))?;
    let manifest: Value = serde_json::from_str(&raw)?;
    let limit = atomic(
        manifest["limits"]["total_test_budget_usdc"]
            .as_str()
            .context("total_test_budget_usdc missing")?,
    )?;
    let cumulative: i64 = db.query_row(
        "SELECT budget FROM authorizations ORDER BY seq DESC LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    for (scope, spent, ceiling) in [
        ("run", used(db, Some(run))?, limit),
        ("registry", used(db, None)?, cumulative.try_into()?),
    ] {
        if spent > ceiling {
            tracing::warn!(
                scope,
                used_atomic = spent,
                limit_atomic = ceiling,
                "Qualification stopped new work because new API authorizations would exceed its total USDC test budget; accepted work remains owned"
            );
            anyhow::bail!(
                "qualification_test_budget_exceeded: {scope} API authorization total would be {spent} micro-USDC, above the {ceiling} micro-USDC budget; no new work accepted"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn cumulative_budget_counts_authorizations_across_runs_without_funding() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE runs(id TEXT,manifest TEXT); CREATE TABLE events(run TEXT,kind TEXT,detail TEXT); CREATE TABLE authorizations(seq INTEGER,budget INTEGER); INSERT INTO authorizations VALUES(1,10);").unwrap();
        for run in ["old", "new"] {
            db.execute(
                "INSERT INTO runs VALUES(?1,?2)",
                params![run, r#"{"limits":{"total_test_budget_usdc":"0.000010"}}"#],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO events VALUES('old','application_payment','{\"amount\":\"6\"}')",
            [],
        )
        .unwrap();
        {
            let tx = db.transaction().unwrap();
            tx.execute(
                "INSERT INTO events VALUES('new','application_payment','{\"amount\":\"5\"}')",
                [],
            )
            .unwrap();
            assert!(
                check(&tx, "new")
                    .unwrap_err()
                    .to_string()
                    .contains("registry")
            );
        }
        assert_eq!(used(&db, None).unwrap(), 6);
        db.execute(
            "INSERT INTO events VALUES('new','application_payment','{\"amount\":\"4\"}')",
            [],
        )
        .unwrap();
        check(&db, "new").unwrap();
        assert_eq!(used(&db, Some("new")).unwrap(), 4);
        assert_eq!(used(&db, None).unwrap(), 10);
        db.execute(
            "INSERT INTO events VALUES('new','application_payment','{\"amount\":\"broken\"}')",
            [],
        )
        .unwrap();
        assert!(
            used(&db, None).is_err(),
            "invalid evidence cannot reset the total"
        );
    }
}
