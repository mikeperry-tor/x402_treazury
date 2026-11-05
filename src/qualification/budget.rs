//! Qualification's USDC API budget, derived from durable admissions.
//! Callers check inside the same write transaction that accepts new work.
use alloy_primitives::U256;
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
        let amount = event_amount(&event)?;
        total = total
            .checked_add(amount)
            .context("test budget total overflow")?;
    }
    Ok(total)
}

fn event_amount(event: &Value) -> Result<u64> {
    event["amount"]
        .as_str()
        .context("payment budget amount missing")?
        .parse()
        .map_err(Into::into)
}

/// Derived only inside the admission owner's existing IMMEDIATE transaction.
#[derive(Debug, Default)]
pub(crate) struct AdmissionTotals {
    pub case: U256,
    run: u64,
    registry: u64,
}

pub(crate) fn admission_totals(db: &Connection, run: &str, case: &str) -> Result<AdmissionTotals> {
    let mut totals = AdmissionTotals::default();
    let mut payments =
        db.prepare("SELECT run=?1,detail FROM events WHERE kind='application_payment'")?;
    for row in payments.query_map([run], |row| {
        Ok((row.get::<_, Option<bool>>(0)?, row.get::<_, String>(1)?))
    })? {
        let (current_run, raw) = row?;
        let event: Value =
            serde_json::from_str(&raw).context("malformed qualification payment event")?;
        if current_run == Some(true) && event["case"] == case {
            // Preserve the case parser's ORIGINAL text domain; u64 -> U256 would
            // wrongly accept historical +1 that this parser rejects.
            let value: U256 = event["amount"]
                .as_str()
                .context("recorded attempt amount missing")?
                .parse()?;
            totals.case = totals
                .case
                .checked_add(value)
                .context("qualified attempt total overflow")?;
        }
        // Every event, including the new attempt, keeps its original u64 parse.
        // Do not normalize underscores/prefixes through the parsed U256 value.
        let amount = event_amount(&event)?;
        if current_run == Some(true) {
            totals.run = totals
                .run
                .checked_add(amount)
                .context("test budget total overflow")?;
        }
        totals.registry = totals
            .registry
            .checked_add(amount)
            .context("test budget total overflow")?;
    }
    Ok(totals)
}

// Manifest was validated by authority() in this same admission transaction.
pub(crate) fn check(db: &Connection, manifest: &Value, totals: &AdmissionTotals) -> Result<()> {
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
        ("run", totals.run, limit),
        ("registry", totals.registry, cumulative.try_into()?),
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

    fn check_run(db: &Connection, run: &str) -> Result<()> {
        let raw: String = db.query_row("SELECT manifest FROM runs WHERE id=?1", [run], |row| {
            row.get(0)
        })?;
        check(
            db,
            &serde_json::from_str(&raw)?,
            &admission_totals(db, run, "case")?,
        )
    }

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
                check_run(&tx, "new")
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
        check_run(&db, "new").unwrap();
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

    fn history() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events(run TEXT,kind TEXT,detail TEXT)")
            .unwrap();
        db
    }
    fn payment(db: &Connection, run: Option<&str>, detail: &str) {
        db.execute(
            "INSERT INTO events VALUES(?1,'application_payment',?2)",
            params![run, detail],
        )
        .unwrap();
    }

    #[test]
    fn original_case_and_budget_numeric_domains_remain_distinct() {
        for (run, case, allowed, expected_run) in [
            (Some("run"), "case", false, 0),
            (Some("run"), "other", true, 1),
            (Some("other"), "case", true, 0),
            (None, "case", true, 0),
        ] {
            let db = history();
            payment(
                &db,
                run,
                &serde_json::json!({"case":case,"amount":"+1"}).to_string(),
            );
            assert_eq!(
                used(&db, None).unwrap(),
                1,
                "public budget parser still accepts its original plus domain"
            );
            let total = admission_totals(&db, "run", "case");
            assert_eq!(total.is_ok(), allowed, "run={run:?} case={case}");
            if let Ok(total) = total {
                assert_eq!(total.case, U256::ZERO);
                assert_eq!(total.run, expected_run);
                assert_eq!(total.registry, 1);
            }
        }
        for amount in [
            "1_0",
            "0x10",
            "0o10",
            "0b10",
            "",
            "_",
            "18446744073709551616",
        ] {
            for case in ["case", "other"] {
                let db = history();
                payment(
                    &db,
                    Some("run"),
                    &serde_json::json!({"case":case,"amount":amount}).to_string(),
                );
                assert!(
                    admission_totals(&db, "run", "case").is_err(),
                    "original per-event u64 validation must reject {amount:?}"
                );
                assert!(used(&db, None).is_err());
            }
        }
    }

    #[test]
    fn run_scoped_used_preserves_its_sql_scope_while_admission_validates_all_history() {
        let db = history();
        payment(&db, Some("run"), r#"{"case":"case","amount":"1"}"#);
        payment(&db, Some("other"), "not JSON");
        assert_eq!(used(&db, Some("run")).unwrap(), 1);
        assert!(used(&db, None).is_err());
        assert!(admission_totals(&db, "run", "case").is_err());
        db.execute("DELETE FROM events WHERE run='other'", [])
            .unwrap();
        payment(
            &db,
            None,
            r#"{"amount":"2","stage":"unknown","receipt":null}"#,
        );
        db.execute(
            "INSERT INTO events VALUES('other','application_receipt','not JSON')",
            [],
        )
        .unwrap();
        let total = admission_totals(&db, "run", "case").unwrap();
        assert_eq!(total.case, U256::from(1));
        assert_eq!(total.run, 1);
        assert_eq!(
            total.registry, 3,
            "unknown/NULL-run payment charges remain global"
        );
    }

    #[test]
    fn full_json_and_checked_per_event_and_total_bounds_are_preserved() {
        for detail in [
            r#"{"amount":1}"#,
            r#"{"case":"other"}"#,
            r#"{"amount":null}"#,
            r#"{"amount":"1","lifecycle":{"unused":1e400}}"#,
            r#"{"amount":"18446744073709551616"}"#,
        ] {
            let db = history();
            payment(&db, Some("other"), detail);
            assert!(
                admission_totals(&db, "run", "case").is_err(),
                "global event must be fully validated: {detail}"
            );
        }
        for second_run in [Some("run"), Some("other"), None] {
            let db = history();
            payment(
                &db,
                Some("run"),
                &serde_json::json!({"case":"case","amount":u64::MAX.to_string()}).to_string(),
            );
            let total = admission_totals(&db, "run", "case").unwrap();
            assert_eq!(total.case, U256::from(u64::MAX));
            assert_eq!(total.run, u64::MAX);
            assert_eq!(total.registry, u64::MAX);
            payment(&db, second_run, r#"{"amount":"1"}"#);
            assert!(
                admission_totals(&db, "run", "case")
                    .unwrap_err()
                    .to_string()
                    .contains("test budget total overflow")
            );
        }
    }
}
