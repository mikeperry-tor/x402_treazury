//! Registry-backed production restriction. Every mutation revalidates immutable
//! session pins and current authority under the same SQLite write transaction.
#[path = "registry_state.rs"]
mod state;
use super::{Limits, Request};
use crate::{
    qualification::{self, Binding},
    rotation::restriction::FundingPermits,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub struct RegistryPermits {
    binding: Binding,
}
fn source_limit(value: &str) -> Result<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !whole.is_empty()
        && whole.bytes().all(|b| b == b'0')
        && fraction.len() <= 8
        && fraction.bytes().all(|b| b == b'0')
    {
        return Ok(0);
    }
    Ok(crate::rotation::config::zatoshis(value)?.try_into()?)
}
impl RegistryPermits {
    pub fn new(binding: Binding) -> Self {
        Self { binding }
    }
    fn authority(&self, db: &Connection, treasury: &str) -> Result<Value> {
        let at = qualification::now()?;
        let manifest = qualification::authority(db, &self.binding, at)?;
        ensure!(
            manifest["windows"]
                .as_array()
                .context("qualification windows missing")?
                .iter()
                .any(|w| {
                    w["not_before"]
                        .as_i64()
                        .zip(w["not_after"].as_i64())
                        .is_some_and(|(start, end)| at >= start && at < end)
                }),
            "qualification funding has no active execution window"
        );
        ensure!(
            manifest["treasury_id"] == treasury,
            "qualification treasury binding differs"
        );
        Ok(manifest)
    }
    fn pool_bound(&self, db: &Connection, manifest: &Value, pool: &str) -> Result<u64> {
        ensure!(
            manifest["start"]["pools"]
                .as_array()
                .context("qualification pool scope missing")?
                .iter()
                .any(|p| p == pool),
            "qualification funding pool is outside reviewed scope"
        );
        let raw: String = db.query_row(
            "SELECT payload FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
            [&self.binding.run],
            |r| r.get(0),
        )?;
        let pins = qualification::bounded(raw)?;
        let config = &pins["resolved_config"]["resolved_wallets"][pool];
        ensure!(
            config["mode"] == "zcash_rotation",
            "qualification funding pool is not managed"
        );
        // Frozen historical evidence remains readable; current TOML rejects old names.
        let bound = crate::rotation::config::zatoshis(
            config
                .get("max_funding_spend_zec")
                .or_else(|| config.get("max_input_zec"))
                .unwrap_or(&Value::Null)
                .as_str()
                .context("qualification requires explicit max_funding_spend_zec")?,
        )?;
        let bound = u64::try_from(bound)?;
        ensure!(bound > 0, "qualification source cap must be positive");
        Ok(bound)
    }
    fn limits(&self, db: &Connection, manifest: &Value) -> Result<(Limits, Limits)> {
        let run = Limits {
            jobs: manifest["limits"]["new_funding_jobs"]
                .as_u64()
                .context("run job limit missing")?
                .try_into()?,
            source_zatoshis: source_limit(
                manifest["limits"]["source_exposure_zec"]
                    .as_str()
                    .context("run source limit missing")?,
            )?,
        };
        let (jobs, source): (u32, i64) = db.query_row(
            "SELECT jobs,source FROM authorizations ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((
            run,
            Limits {
                jobs,
                source_zatoshis: source.try_into()?,
            },
        ))
    }
    /// Reserve all bootstrap pools under one registry transaction before the
    /// first production allocation. Later per-pool calls reuse these job IDs.
    fn reserve_batch(
        &self,
        tx: &mut rusqlite::Transaction<'_>,
        treasury: &str,
        manifest: &Value,
        allocations: &[(&str, i64)],
    ) -> Result<Vec<String>> {
        let (run_limit, cumulative_limit) = self.limits(tx, manifest)?;
        let mut requests = Vec::new();
        let mut jobs = Vec::new();
        for (pool, sequence) in allocations {
            let bound = self.pool_bound(tx, manifest, pool)?;
            ensure!(*sequence >= 0, "invalid allocation sequence");
            // Length-delimited JSON input, domain separated from all payment and
            // network identities. Session/process/quote changes do not change it.
            let input =
                serde_json::to_vec(&("qualification-allocation-v1", treasury, pool, sequence))?;
            let hash = Sha256::digest(input);
            requests.push(Request {
                intent: format!("{:x}", hash),
                pool: (*pool).into(),
                source_bound: bound,
            });
            jobs.push(uuid::Uuid::from_bytes(hash[..16].try_into()?).to_string());
        }
        super::reserve(
            tx,
            &self.binding.run,
            &requests,
            run_limit,
            cumulative_limit,
            qualification::now()?,
        )?;
        for (request, job) in requests.iter().zip(&jobs) {
            let saved: Option<String> = tx.query_row(
                "SELECT job FROM funding_permits WHERE intent=?1",
                [&request.intent],
                |r| r.get(0),
            )?;
            ensure!(
                saved.as_ref().is_none_or(|saved| saved == job),
                "qualification allocation job binding changed"
            );
            tx.execute(
                "UPDATE funding_permits SET job=?1 WHERE intent=?2",
                params![job, request.intent],
            )?;
        }
        Ok(jobs)
    }
}
impl FundingPermits for RegistryPermits {
    fn validate_treasury(&self, treasury: &str) -> Result<()> {
        let mut db = qualification::connection(&self.binding)?;
        let tx = db.transaction()?;
        self.authority(&tx, treasury)?;
        tx.commit()?;
        Ok(())
    }
    fn validate_state(&self, snapshot: &Value) -> Result<()> {
        state::validate(self, snapshot)
    }
    fn reserve_allocations(
        &self,
        treasury: &str,
        pool: &str,
        sequences: &[i64],
    ) -> Result<Vec<String>> {
        let mut db = qualification::connection(&self.binding)?;
        let mut tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let manifest = self.authority(&tx, treasury)?;
        let allocations: Vec<_> = sequences.iter().map(|sequence| (pool, *sequence)).collect();
        let jobs = self.reserve_batch(&mut tx, treasury, &manifest, &allocations)?;
        tx.commit()?;
        Ok(jobs)
    }
    fn check_preparation(&self, treasury: &str, job: &str, input_with_fee: u64) -> Result<()> {
        let mut db = qualification::connection(&self.binding)?;
        let tx = db.transaction()?;
        let manifest = self.authority(&tx, treasury)?;
        let pool: String = tx.query_row(
            "SELECT pool FROM funding_permits WHERE run=?1 AND job=?2",
            params![self.binding.run, job],
            |r| r.get(0),
        )?;
        ensure!(
            input_with_fee <= self.pool_bound(&tx, &manifest, &pool)?,
            "qualification pinned source cap exceeded"
        );
        super::check_preparation(&tx, &self.binding.run, job, input_with_fee)?;
        tx.commit()?;
        Ok(())
    }
    fn reserve_preparation(
        &self,
        treasury: &str,
        job: &str,
        operation: &str,
        input_with_fee: u64,
    ) -> Result<()> {
        let mut db = qualification::connection(&self.binding)?;
        let mut tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let manifest = self.authority(&tx, treasury)?;
        let pool: String = tx.query_row(
            "SELECT pool FROM funding_permits WHERE run=?1 AND job=?2",
            params![self.binding.run, job],
            |r| r.get(0),
        )?;
        ensure!(
            input_with_fee <= self.pool_bound(&tx, &manifest, &pool)?,
            "qualification pinned source cap exceeded"
        );
        super::reserve_source(&mut tx, &self.binding.run, job, operation, input_with_fee)?;
        tx.commit()?;
        Ok(())
    }
    fn retire_unprepared(&self, treasury: &str, job: &str, operation: &str) -> Result<()> {
        let mut db = qualification::connection(&self.binding)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.authority(&tx, treasury)?;
        super::retire_unprepared(&tx, &self.binding.run, job, operation)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_authority_accepts_explicit_zero_without_weakening_decimal_grammar() {
        for value in ["0", "0.00000000", "00.0"] {
            assert_eq!(source_limit(value).unwrap(), 0);
        }
        assert_eq!(source_limit("0.00000001").unwrap(), 1);
        for value in ["", ".0", "-0", "0.000000000", "0e0", "0.0.0", " 0"] {
            assert!(source_limit(value).is_err(), "{value}");
        }
    }
    pub(super) fn fixture() -> (tempfile::TempDir, RegistryPermits, Connection) {
        let (dir, guard, db) = qualification::tests::fixture();
        db.execute_batch("CREATE UNIQUE INDEX runs_id ON runs(id); ALTER TABLE authorizations ADD COLUMN jobs INTEGER; ALTER TABLE authorizations ADD COLUMN source INTEGER; UPDATE authorizations SET jobs=2,source=200;").unwrap();
        db.execute_batch(super::super::SCHEMA).unwrap();
        let mut binding = guard.binding.clone();
        let raw: String = db
            .query_row("SELECT manifest FROM runs", [], |r| r.get(0))
            .unwrap();
        let mut manifest: Value = serde_json::from_str(&raw).unwrap();
        manifest["start"] = serde_json::json!({"pools":["pool"]});
        manifest["limits"] =
            serde_json::json!({"new_funding_jobs":2,"source_exposure_zec":"0.000002"});
        manifest["windows"] =
            serde_json::json!([{"not_before":0,"not_after":qualification::now().unwrap()+600}]);
        db.execute("UPDATE runs SET manifest=?1", [manifest.to_string()])
            .unwrap();
        let raw: String = db
            .query_row("SELECT payload FROM pins", [], |r| r.get(0))
            .unwrap();
        let mut pins: Value = serde_json::from_str(&raw).unwrap();
        pins["resolved_config"] = serde_json::json!({"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_funding_spend_zec":"0.000001"}}});
        let raw = pins.to_string();
        binding.pin_digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
        db.execute(
            "UPDATE pins SET payload=?1,digest=?2",
            params![raw, binding.pin_digest],
        )
        .unwrap();
        db.execute(
            "UPDATE events SET detail=?1 WHERE kind='application_session'",
            [serde_json::to_string(&binding).unwrap()],
        )
        .unwrap();
        (dir, RegistryPermits::new(binding), db)
    }
    #[test]
    fn registry_adapter_revalidates_authority_and_reuses_stable_jobs() {
        let (_dir, permits, db) = fixture();
        permits.validate_treasury("treasury").unwrap();
        assert!(permits.validate_treasury("other").is_err());
        assert!(
            permits
                .reserve_allocations("treasury", "other", &[0])
                .is_err()
        );
        let jobs = permits
            .reserve_allocations("treasury", "pool", &[0, 1])
            .unwrap();
        let restarted = RegistryPermits::new(permits.binding.clone());
        assert_eq!(
            restarted
                .reserve_allocations("treasury", "pool", &[0, 1])
                .unwrap(),
            jobs
        );
        assert!(
            restarted
                .reserve_allocations("treasury", "pool", &[2])
                .is_err()
        );
        permits
            .check_preparation("treasury", &jobs[0], 100)
            .unwrap();
        assert!(
            permits
                .check_preparation("treasury", &jobs[0], 101)
                .is_err()
        );
        permits
            .reserve_preparation("treasury", &jobs[0], "operation", 100)
            .unwrap();
        permits
            .reserve_preparation("treasury", &jobs[0], "operation", 100)
            .unwrap();
        assert!(
            permits
                .reserve_preparation("treasury", &jobs[0], "new_operation", 100)
                .is_err()
        );
        permits
            .retire_unprepared("treasury", &jobs[0], "operation")
            .unwrap();
        permits
            .reserve_preparation("treasury", &jobs[0], "new_operation", 100)
            .unwrap();
        assert!(
            permits
                .reserve_preparation("treasury", &jobs[0], "operation", 100)
                .is_err()
        );
        db.execute(
            "INSERT INTO authorizations(id,seq,jobs,source) VALUES('new',2,100,100000)",
            [],
        )
        .unwrap();
        assert!(permits.check_preparation("treasury", &jobs[0], 1).is_err());
        assert!(
            permits
                .reserve_allocations("treasury", "pool", &[2])
                .is_err()
        );
    }
    #[test]
    fn changed_pins_and_closed_windows_deny_even_existing_permits() {
        let (_dir, permits, db) = fixture();
        let jobs = permits
            .reserve_allocations("treasury", "pool", &[0])
            .unwrap();
        db.execute(
            "UPDATE runs SET manifest=json_set(manifest,'$.windows[0].not_after',0)",
            [],
        )
        .unwrap();
        assert!(permits.check_preparation("treasury", &jobs[0], 1).is_err());
        db.execute(
            "UPDATE runs SET manifest=json_set(manifest,'$.windows[0].not_after',?1)",
            [qualification::now().unwrap() + 600],
        )
        .unwrap();
        db.execute("UPDATE pins SET payload='{}'", []).unwrap();
        assert!(permits.check_preparation("treasury", &jobs[0], 1).is_err());
    }
}
