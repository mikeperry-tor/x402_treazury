//! Reconcile immutable registry lineage with the exclusive treasury owner's view.
use super::*;
use std::collections::BTreeMap;
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value[key]
        .as_array()
        .with_context(|| format!("qualification state missing {key}"))
}
fn name<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("qualification state missing {key}"))
}
fn index<'a>(rows: &'a [Value], key: &str) -> Result<BTreeMap<&'a str, &'a Value>> {
    let mut map = BTreeMap::new();
    for row in rows {
        ensure!(
            map.insert(name(row, key)?, row).is_none(),
            "duplicate qualification state identity"
        );
    }
    Ok(map)
}
pub(super) fn validate(guard: &RegistryPermits, current: &Value) -> Result<()> {
    let status = &current["treasury_status"];
    let mut db = qualification::connection(&guard.binding)?;
    let mut tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let manifest = guard.authority(&tx, name(status, "treasury_id")?)?;
    let (run_limits, _) = guard.limits(&tx, &manifest)?;
    let existing_funds_only = manifest["start"]["mode"] == "funded_pools"
        && run_limits.jobs == 0
        && run_limits.source_zatoshis == 0;
    let (raw, hash): (String, String) = tx.query_row(
        "SELECT baseline,baseline_hash FROM identity WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        format!("{:x}", Sha256::digest(raw.as_bytes())) == hash,
        "qualification baseline hash mismatch"
    );
    let baseline = qualification::bounded(raw)?;
    ensure!(
        status["snapshot_revision"]
            .as_i64()
            .zip(baseline["treasury_status"]["snapshot_revision"].as_i64())
            .is_some_and(|(now, old)| now >= old),
        "treasury snapshot regressed below registry baseline"
    );
    ensure!(
        baseline["treasury_status"]["treasury_id"] == status["treasury_id"],
        "qualification baseline treasury mismatch"
    );
    let raw: String = tx.query_row(
        "SELECT payload FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
        [&guard.binding.run],
        |r| r.get(0),
    )?;
    let pins = qualification::bounded(raw)?;
    let profiles = pins["resolved_config"]["resolved_wallets"]
        .as_object()
        .context("pinned wallet profiles missing")?;
    let pools = index(array(status, "pools")?, "id")?;
    let old_pools = index(array(&baseline["treasury_status"], "pools")?, "id")?;
    let mut old_wallets = BTreeMap::new();
    for (id, old) in &old_pools {
        let pool = pools.get(id).context("baseline pool disappeared")?;
        ensure!(
            pool["name"] == old["name"],
            "baseline pool identity changed"
        );
        let wallets = index(array(pool, "addresses")?, "id")?;
        for wallet in array(old, "addresses")? {
            let id = name(wallet, "id")?;
            let now = wallets.get(id).context("baseline wallet disappeared")?;
            ensure!(
                now["address"] == wallet["address"],
                "baseline wallet address changed"
            );
            old_wallets.insert(id, wallet);
        }
    }
    let jobs = index(array(status, "funding_jobs")?, "id")?;
    let old_jobs = index(array(&baseline["treasury_status"], "funding_jobs")?, "id")?;
    for (id, old) in &old_jobs {
        let job = jobs.get(id).context("baseline funding job disappeared")?;
        for field in ["wallet_id", "recipient", "pool_id", "pool_name"] {
            ensure!(
                job[field] == old[field],
                "baseline job binding changed: {field}"
            );
        }
    }
    for pool in pools.values() {
        let pool_name = name(pool, "name")?;
        ensure!(
            profiles
                .get(pool_name)
                .is_some_and(|p| p["mode"] == "zcash_rotation"),
            "managed configuration would omit or disable existing pool {pool_name}"
        );
        for wallet in array(pool, "addresses")? {
            if old_wallets.contains_key(name(wallet, "id")?) {
                continue;
            }
            let job = jobs
                .values()
                .find(|j| j["wallet_id"] == wallet["id"])
                .context("wallet has no funding job")?;
            ensure!(
                job["recipient"] == wallet["address"] && job["pool_id"] == pool["id"],
                "wallet/job binding differs"
            );
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM funding_permits WHERE job=?1 AND pool=?2)",
                params![name(job, "id")?, pool_name],
                |r| r.get(0),
            )?;
            ensure!(
                known,
                "unexplained wallet allocation outside registry baseline"
            );
        }
    }
    // A fresh run cannot adopt another run's funding authority. A known, wholly
    // unprepared refill can remain paused in a zero-funding run, or outside the
    // selected pool scope. Its permit stays owned and charged to the old run.
    for (id, job) in &jobs {
        if job["phase"] == "COMPLETE" {
            continue;
        }
        let owned: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_permits WHERE run=?1 AND job=?2)",
            params![guard.binding.run, id],
            |r| r.get(0),
        )?;
        let outside_scope = !array(&manifest["start"], "pools")?
            .iter()
            .any(|pool| pool == &job["pool_name"]);
        let retained = !owned
            && (existing_funds_only || outside_scope)
            && retained_unprepared_job(&tx, job, status)?;
        ensure!(
            owned || retained,
            "incomplete funding job requires its original run or explicit reconciliation before a fresh run"
        );
    }
    for operation in array(current, "pending_source_operations")? {
        let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_source_attempts s JOIN funding_permits p ON p.job=s.job WHERE p.run=?1 AND s.operation=?2 AND s.released=0)", params![guard.binding.run,operation.as_str().context("invalid pending operation")?], |r| r.get(0))?;
        ensure!(
            owned,
            "pending source operation requires its original run before fresh qualification"
        );
    }
    validate_source(&tx, &baseline, current)?;
    validate_payments(&tx, &guard.binding.run, &baseline, current)?;
    let resumed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_permits WHERE run=?1) OR EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_claim')", [&guard.binding.run], |r| r.get(0))?;
    for selected in array(&manifest["start"], "pools")? {
        let pool = pools.values().find(|p| p["name"] == *selected);
        if manifest["start"]["mode"] == "treasury_only" {
            ensure!(
                resumed || pool.is_none(),
                "treasury-only start already has selected allocations"
            );
        } else {
            let pool = pool.context("selected funded pool missing; bootstrap must be explicit")?;
            ensure!(
                pool["bootstrapped"] == true,
                "selected pool is not bootstrapped"
            );
            if !resumed {
                let wallets = array(pool, "addresses")?;
                ensure!(
                    wallets.iter().any(|w| w["role"] == "ACTIVE")
                        && (existing_funds_only || wallets.iter().any(|w| w["role"] == "READY")),
                    "funded start needs active and standby; repair must be explicit"
                );
            }
        }
    }
    let mut bootstrap = Vec::new();
    for (pool_name, profile) in profiles {
        if profile["mode"] == "zcash_rotation"
            && !pools.values().any(|pool| pool["name"] == *pool_name)
        {
            bootstrap.extend([(pool_name.as_str(), 0), (pool_name.as_str(), 1)]);
        }
    }
    if !bootstrap.is_empty() {
        guard.reserve_batch(&mut tx, name(status, "treasury_id")?, &manifest, &bootstrap)?;
    }
    tx.commit()?;
    Ok(())
}
fn retained_unprepared_job(db: &Connection, job: &Value, status: &Value) -> Result<bool> {
    if !matches!(job["phase"].as_str(), Some("ALLOCATED" | "QUOTED")) {
        return Ok(false);
    }
    let known: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_permits p JOIN runs r ON r.id=p.run WHERE p.job=?1 AND p.pool=?2 AND NOT EXISTS(SELECT 1 FROM funding_source_attempts s WHERE s.job=p.job AND s.released=0))",
        params![name(job, "id")?, name(job, "pool_name")?],
        |r| r.get(0),
    )?;
    let bound = array(status, "pools")?.iter().any(|pool| {
        pool["id"] == job["pool_id"]
            && pool["name"] == job["pool_name"]
            && pool["addresses"].as_array().is_some_and(|wallets| {
                wallets.iter().any(|wallet| {
                    wallet["id"] == job["wallet_id"]
                        && wallet["address"] == job["recipient"]
                        && wallet["role"] == "ALLOCATED"
                })
            })
    });
    Ok(known && bound)
}
fn validate_source(db: &Connection, old: &Value, current: &Value) -> Result<()> {
    let before = index(array(old, "source_budget_entries")?, "id")?;
    let after = index(array(current, "source_budget_entries")?, "id")?;
    for (id, prior) in &before {
        if let Some(now) = after.get(id) {
            ensure!(
                now["requested"] == prior["requested"]
                    && now["original_day"] == prior["original_day"],
                "baseline source reservation changed"
            );
            ensure!(
                now["consumed"]
                    .as_u64()
                    .zip(prior["consumed"].as_u64())
                    .is_some_and(|(now, old)| now >= old),
                "baseline source spending regressed"
            );
        } else {
            ensure!(
                prior["consumed"] == 0
                    && array(current, "unprepared_archives")?
                        .iter()
                        .any(|x| x == id),
                "baseline source exposure disappeared without unprepared recovery"
            );
        }
    }
    for (id, entry) in after {
        if before.contains_key(id) {
            continue;
        }
        let (bound, job, pool): (i64,String,String) = db.query_row("SELECT s.source_bound,s.job,p.pool FROM funding_source_attempts s JOIN funding_permits p ON p.job=s.job WHERE s.operation=?1 AND s.released=0", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).context("source exposure outside registry baseline/permits")?;
        ensure!(
            array(&current["treasury_status"], "funding_jobs")?
                .iter()
                .any(|j| j["id"] == job && j["pool_name"] == pool),
            "reserved source operation lacks its allocated funding job"
        );
        ensure!(
            entry["requested"]
                .as_i64()
                .is_some_and(|v| v > 0 && v <= bound),
            "source reservation exceeds permit"
        );
    }
    Ok(())
}
fn validate_payments(db: &Connection, run: &str, old: &Value, current: &Value) -> Result<()> {
    let before = index(array(old, "payment_attempts")?, "id")?;
    let after = index(array(current, "payment_attempts")?, "id")?;
    for (id, prior) in &before {
        let Some(now) = after.get(id) else {
            ensure!(
                prior["state"] == "ADMITTED",
                "baseline signed payment attempt disappeared"
            );
            continue;
        };
        for field in ["pool", "wallet", "generation", "amount"] {
            ensure!(
                now[field] == prior[field],
                "baseline payment attempt changed"
            );
        }
    }
    for (id, attempt) in after {
        if before.contains_key(id) && attempt["state"] == "RESOLVED" {
            continue;
        }
        let count: i64 = db.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?2", params![run,id], |r| r.get(0))?;
        if count == 0 && attempt["state"] == "RESOLVED" {
            validate_prior_payment(db, run, id, attempt, current)?;
            continue;
        }
        if count == 0 && attempt["state"] == "ADMITTED" {
            tracing::warn!(
                category = "qualification_unsigned_admission",
                "unsubmitted admission lacks correlation; production reconciliation may discard it; case reservations remain charged"
            );
            continue;
        }
        ensure!(
            count == 1,
            "payment liability requires correlated application evidence or reconciliation before a fresh run"
        );
        let raw: String = db.query_row("SELECT detail FROM events WHERE run=?1 AND kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?2", params![run,id], |r| r.get(0))?;
        let event = qualification::bounded(raw)?;
        for field in ["pool", "wallet", "generation", "amount"] {
            ensure!(
                event[field] == attempt[field],
                "payment correlation differs from treasury attempt"
            );
        }
        let dispatched: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM cases WHERE run=?1 AND id=?2 AND execution NOT IN ('UNATTEMPTED','SKIPPED_TARGET_REACHED'))", params![run,name(&event,"case")?], |r| r.get(0))?;
        ensure!(dispatched, "payment correlation lacks a reserved case");
    }
    Ok(())
}

/// Registry baselines stay immutable across runs. A payment added by an earlier
/// run is no longer a liability only after its canonical outcome is journaled
/// and the original charged case has observed that same outcome.
fn validate_prior_payment(
    db: &Connection,
    run: &str,
    id: &str,
    attempt: &Value,
    current: &Value,
) -> Result<()> {
    let resolutions = index(array(current, "payment_resolutions")?, "attempt_id")?;
    let resolution = resolutions
        .get(id)
        .context("earlier payment lacks canonical resolution")?;
    let outcome = name(resolution, "outcome")?;
    ensure!(
        matches!(outcome, "USED" | "EXPIRED_UNUSED")
            && resolution["height"].as_u64().is_some()
            && resolution["block_time"].as_u64().is_some(),
        "earlier payment resolution is unclassified or malformed"
    );
    let _: alloy_primitives::B256 = name(resolution, "hash")?.parse()?;
    let rows = db.prepare("SELECT run,detail FROM events WHERE kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?1 LIMIT 2")?
        .query_map([id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        rows.len() == 1 && rows[0].0 != run,
        "resolved earlier payment requires exactly one original run correlation"
    );
    let (prior_run, raw) = &rows[0];
    let event = qualification::bounded(raw.clone())?;
    for field in [
        "pool",
        "wallet",
        "generation",
        "amount",
        "requirements_hash",
    ] {
        ensure!(
            !attempt[field].is_null() && event[field] == attempt[field],
            "earlier payment correlation differs from journal: {field}"
        );
    }
    let payer: alloy_primitives::Address = name(attempt, "payer")?.parse()?;
    ensure!(
        name(&event, "address")?.parse::<alloy_primitives::Address>()? == payer,
        "earlier payment payer differs from journal"
    );
    let reserved: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM cases WHERE run=?1 AND id=?2 AND execution NOT IN ('UNATTEMPTED','SKIPPED_TARGET_REACHED') AND reservation>0 AND settlement=?3)", params![prior_run,name(&event,"case")?,outcome],|r| r.get(0))?;
    ensure!(
        reserved,
        "earlier payment needs its original charged case and matching observed settlement; reconcile that run first"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, RegistryPermits, Connection, Value) {
        let (dir, guard, db) = super::super::tests::fixture();
        let state = json!({
            "treasury_status": {"treasury_id":"treasury","snapshot_revision":1,
                "pools":[{"id":"p","name":"pool","bootstrapped":true,"addresses":[
                    {"id":"a","address":"0xa","role":"ACTIVE"},
                    {"id":"b","address":"0xb","role":"READY"}]}],
                "funding_jobs":[
                    {"id":"ja","pool_id":"p","pool_name":"pool","wallet_id":"a","recipient":"0xa","phase":"COMPLETE"},
                    {"id":"jb","pool_id":"p","pool_name":"pool","wallet_id":"b","recipient":"0xb","phase":"COMPLETE"}]
            },
            "payment_attempts":[],"source_budget_entries":[],
            "pending_source_operations":[],"unprepared_archives":[]
        });
        db.execute_batch("ALTER TABLE identity ADD COLUMN baseline TEXT; ALTER TABLE identity ADD COLUMN baseline_hash TEXT;").unwrap();
        baseline(&db, &state);
        db.execute(
            "UPDATE runs SET manifest=json_set(manifest,'$.start.mode','funded_pools')",
            [],
        )
        .unwrap();
        (dir, guard, db, state)
    }
    fn baseline(db: &Connection, state: &Value) {
        let raw = state.to_string();
        db.execute(
            "UPDATE identity SET baseline=?1,baseline_hash=?2",
            params![raw, format!("{:x}", Sha256::digest(raw.as_bytes()))],
        )
        .unwrap();
    }

    #[test]
    fn zero_funding_run_can_use_active_wallet_while_prior_unprepared_refill_stays_owned() {
        let (_dir, guard, db, mut state) = fixture();
        db.execute(
            "INSERT INTO runs(id,manifest) SELECT 'previous',manifest FROM runs LIMIT 1",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO funding_permits(intent,run,pool,source_bound,job,at) VALUES('prior_intent','previous','pool',100,'jc',0)", []).unwrap();
        state["treasury_status"]["pools"][0]["addresses"][1]["role"] = json!("RETIRED");
        state["treasury_status"]["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"c","address":"0xc","role":"ALLOCATED"}));
        state["treasury_status"]["funding_jobs"].as_array_mut().unwrap().push(
            json!({"id":"jc","pool_id":"p","pool_name":"pool","wallet_id":"c","recipient":"0xc","phase":"QUOTED"}));
        assert!(
            guard.validate_state(&state).is_err(),
            "positive funding cannot adopt another run's job"
        );
        db.execute("UPDATE runs SET manifest=json_set(manifest,'$.limits.new_funding_jobs',0,'$.limits.source_exposure_zec','0') WHERE id!='previous'", []).unwrap();
        guard.validate_state(&state).unwrap();
        assert!(guard.check_preparation("treasury", "jc", 1).is_err());
        assert!(
            guard
                .reserve_preparation("treasury", "jc", "op", 1)
                .is_err()
        );
        assert!(
            guard
                .reserve_allocations("treasury", "pool", &[99])
                .is_err()
        );
        let permit: (String, i64) = db
            .query_row(
                "SELECT run,source_bound FROM funding_permits WHERE job='jc'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(permit, ("previous".into(), 100));
        for phase in [
            "PREPARING",
            "PREPARED",
            "DEPOSIT_PENDING",
            "RECOVERY_REQUIRED",
        ] {
            let mut changed = state.clone();
            changed["treasury_status"]["funding_jobs"][2]["phase"] = json!(phase);
            assert!(guard.validate_state(&changed).is_err(), "{phase}");
        }
        for (field, value) in [
            ("recipient", "different"),
            ("pool_id", "other"),
            ("pool_name", "other"),
            ("wallet_id", "a"),
        ] {
            let mut changed = state.clone();
            changed["treasury_status"]["funding_jobs"][2][field] = json!(value);
            assert!(guard.validate_state(&changed).is_err(), "{field}");
        }
        let mut changed = state.clone();
        changed["pending_source_operations"] = json!(["unexplained"]);
        assert!(guard.validate_state(&changed).is_err());
        db.execute(
            "INSERT INTO funding_source_attempts(operation,job,source_bound) VALUES('op','jc',100)",
            [],
        )
        .unwrap();
        assert!(
            guard.validate_state(&state).is_err(),
            "an accepted preparation reservation remains a liability even without bytes"
        );
        db.execute("DELETE FROM funding_source_attempts", [])
            .unwrap();
        db.execute("DELETE FROM funding_permits", []).unwrap();
        assert!(
            guard.validate_state(&state).is_err(),
            "unknown allocation cannot be retained"
        );
    }

    #[test]
    fn selected_pool_can_rotate_without_adopting_another_pools_unprepared_job() {
        let (_dir, mut guard, db, mut state) = fixture();
        state["treasury_status"]["pools"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "id":"q","name":"other","bootstrapped":true,"addresses":[
                {"id":"x","address":"0xx","role":"ACTIVE"},
                {"id":"y","address":"0xy","role":"READY"}]}));
        baseline(&db, &state);
        db.execute(
            "INSERT INTO runs(id,manifest) SELECT 'previous',manifest FROM runs LIMIT 1",
            [],
        )
        .unwrap();
        db.execute("UPDATE runs SET manifest=json_set(manifest,'$.start.pools',json('[\"other\"]')) WHERE id!='previous'", []).unwrap();
        let raw: String = db
            .query_row("SELECT payload FROM pins", [], |r| r.get(0))
            .unwrap();
        let mut pins: Value = serde_json::from_str(&raw).unwrap();
        pins["resolved_config"]["resolved_wallets"]["other"] =
            pins["resolved_config"]["resolved_wallets"]["pool"].clone();
        let raw = pins.to_string();
        guard.binding.pin_digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
        db.execute(
            "UPDATE pins SET payload=?1,digest=?2",
            params![raw, guard.binding.pin_digest],
        )
        .unwrap();
        db.execute(
            "UPDATE events SET detail=?1 WHERE kind='application_session'",
            [serde_json::to_string(&guard.binding).unwrap()],
        )
        .unwrap();
        db.execute("INSERT INTO funding_permits(intent,run,pool,source_bound,job,at) VALUES('prior_intent','previous','pool',100,'jc',0)", []).unwrap();
        state["treasury_status"]["pools"][0]["addresses"][1]["role"] = json!("RETIRED");
        state["treasury_status"]["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"c","address":"0xc","role":"ALLOCATED"}));
        state["treasury_status"]["funding_jobs"].as_array_mut().unwrap().push(json!({"id":"jc","pool_id":"p","pool_name":"pool","wallet_id":"c","recipient":"0xc","phase":"QUOTED"}));
        guard.validate_state(&state).unwrap();
        assert!(guard.reserve_allocations("treasury", "pool", &[2]).is_err());
        assert!(guard.check_preparation("treasury", "jc", 1).is_err());
        assert!(
            guard
                .reserve_preparation("treasury", "jc", "old_operation", 1)
                .is_err()
        );
        let jobs = guard
            .reserve_allocations("treasury", "other", &[2])
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_ne!(jobs[0], "jc");
        let retained: (String, i64) = db
            .query_row(
                "SELECT run,source_bound FROM funding_permits WHERE job='jc'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(retained, ("previous".into(), 100));
        state["treasury_status"]["funding_jobs"][2]["phase"] = json!("PREPARED");
        assert!(guard.validate_state(&state).is_err());
        state["treasury_status"]["funding_jobs"][2]["phase"] = json!("QUOTED");
        db.execute("INSERT INTO funding_source_attempts(operation,job,source_bound) VALUES('old_operation','jc',100)", []).unwrap();
        assert!(guard.validate_state(&state).is_err());
    }

    #[test]
    fn all_missing_pools_reserve_atomically_before_first_allocation_and_reuse_after_restart() {
        let (_dir, mut guard, db, mut state) = fixture();
        state["treasury_status"]["pools"] = json!([]);
        state["treasury_status"]["funding_jobs"] = json!([]);
        baseline(&db, &state);
        db.execute(r#"UPDATE runs SET manifest=json_set(manifest,'$.start.mode','treasury_only','$.start.pools',json('["pool","second"]'))"#, []).unwrap();
        let raw: String = db
            .query_row("SELECT payload FROM pins", [], |r| r.get(0))
            .unwrap();
        let mut pins: Value = serde_json::from_str(&raw).unwrap();
        pins["resolved_config"]["resolved_wallets"]["second"] =
            pins["resolved_config"]["resolved_wallets"]["pool"].clone();
        let raw = pins.to_string();
        guard.binding.pin_digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
        db.execute(
            "UPDATE pins SET payload=?1,digest=?2",
            params![raw, guard.binding.pin_digest],
        )
        .unwrap();
        db.execute(
            "UPDATE events SET detail=?1 WHERE kind='application_session'",
            [serde_json::to_string(&guard.binding).unwrap()],
        )
        .unwrap();
        let count = || {
            db.query_row("SELECT COUNT(*) FROM funding_permits", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        assert!(guard.validate_state(&state).is_err());
        assert_eq!(
            count(),
            0,
            "first pool must not consume authority when the second cannot fit"
        );
        db.execute("UPDATE authorizations SET jobs=4,source=400", [])
            .unwrap();
        db.execute("UPDATE runs SET manifest=json_set(manifest,'$.limits.new_funding_jobs',4,'$.limits.source_exposure_zec','0.000003')", []).unwrap();
        assert!(guard.validate_state(&state).is_err());
        assert_eq!(count(), 0, "source exposure refusal is also atomic");
        db.execute(
            "UPDATE runs SET manifest=json_set(manifest,'$.limits.source_exposure_zec','0.000004')",
            [],
        )
        .unwrap();
        guard.validate_state(&state).unwrap();
        assert_eq!(count(), 4);
        let first = guard
            .reserve_allocations("treasury", "pool", &[0, 1])
            .unwrap();
        let second = guard
            .reserve_allocations("treasury", "second", &[0, 1])
            .unwrap();
        assert_ne!(first, second);
        let restarted = RegistryPermits::new(guard.binding.clone());
        restarted.validate_state(&state).unwrap();
        assert_eq!(
            restarted
                .reserve_allocations("treasury", "second", &[0, 1])
                .unwrap(),
            second
        );
        assert_eq!(
            count(),
            4,
            "orphan reservations survive and are reused without a second charge"
        );
    }
    #[test]
    fn funded_baseline_preserves_wallets_and_rejects_unexplained_work() {
        let (_dir, guard, db, state) = fixture();
        guard.validate_state(&state).unwrap();
        for change in 0..8 {
            let mut changed = state.clone();
            match change {
                0 => {
                    changed["treasury_status"]["pools"][0]["addresses"][0]["address"] =
                        json!("different")
                }
                1 => changed["treasury_status"]["pools"] = json!([]),
                2 => changed["treasury_status"]["funding_jobs"][0]["phase"] = json!("PREPARING"),
                3 => changed["pending_source_operations"] = json!(["unknown"]),
                4 => changed["source_budget_entries"] = json!([{"id":"unknown","requested":1}]),
                5 => {
                    changed["payment_attempts"] =
                        json!([{"id":"unknown","state":"POSSIBLY_SUBMITTED"}])
                }
                6 => changed["treasury_status"]["snapshot_revision"] = json!(0),
                _ => {
                    changed["treasury_status"]["pools"][0]["addresses"][1]["role"] =
                        json!("RETIRED")
                }
            }
            assert!(
                guard.validate_state(&changed).is_err(),
                "change {change} accepted"
            );
        }
        db.execute("UPDATE identity SET baseline='{}'", []).unwrap();
        assert!(guard.validate_state(&state).is_err());
    }
    #[test]
    fn source_baseline_only_releases_durably_archived_unprepared_reservations() {
        let (_dir, guard, db, mut state) = fixture();
        state["source_budget_entries"] =
            json!([{"id":"old","requested":100,"original_day":1,"reserved":100,"consumed":0}]);
        baseline(&db, &state);
        let mut current = state.clone();
        current["source_budget_entries"] = json!([]);
        assert!(guard.validate_state(&current).is_err());
        current["unprepared_archives"] = json!(["old"]);
        guard.validate_state(&current).unwrap();
        state["source_budget_entries"][0]["consumed"] = json!(10);
        baseline(&db, &state);
        assert!(guard.validate_state(&current).is_err());
        current = state.clone();
        current["source_budget_entries"][0]["consumed"] = json!(9);
        assert!(guard.validate_state(&current).is_err());
    }
    #[test]
    fn known_registry_allocation_can_resume_but_cannot_disable_an_existing_pool() {
        let (_dir, guard, _db, mut state) = fixture();
        // Current-run reservations may account for a replacement not in baseline.
        let job = guard
            .reserve_allocations("treasury", "pool", &[2])
            .unwrap()
            .remove(0);
        state["treasury_status"]["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"c","address":"0xc","role":"ALLOCATED"}));
        state["treasury_status"]["funding_jobs"].as_array_mut().unwrap().push(json!({"id":job,"pool_id":"p","pool_name":"pool","wallet_id":"c","recipient":"0xc","phase":"ALLOCATED"}));
        guard.validate_state(&state).unwrap();
        state["treasury_status"]["pools"][0]["name"] = json!("other");
        assert!(guard.validate_state(&state).is_err());
        // A signed source attempt remains attributed to this run on restart.
        state["treasury_status"]["pools"][0]["name"] = json!("pool");
        guard
            .reserve_preparation("treasury", &job, "operation", 100)
            .unwrap();
        state["source_budget_entries"] = json!([{"id":"operation","requested":100,"original_day":1,"reserved":100,"consumed":0}]);
        state["pending_source_operations"] = json!(["operation"]);
        guard.validate_state(&state).unwrap();
        state["source_budget_entries"][0]["requested"] = json!(101);
        assert!(guard.validate_state(&state).is_err());
    }
    #[test]
    fn payment_resume_requires_exact_correlation_with_a_dispatched_case() {
        let (_dir, guard, db, mut state) = fixture();
        let attempt = json!({"id":"attempt","pool":"p","wallet":"a","generation":0,"amount":"1","state":"POSSIBLY_SUBMITTED"});
        state["payment_attempts"] = json!([attempt]);
        assert!(guard.validate_state(&state).is_err());
        let event = json!({"attempt_id":"attempt","case":"case","pool":"p","wallet":"a","generation":0,"amount":"1"});
        db.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES('run','application_payment',?1,0)",
            [event.to_string()],
        )
        .unwrap();
        guard.validate_state(&state).unwrap();
        state["payment_attempts"][0]["amount"] = json!("2");
        assert!(guard.validate_state(&state).is_err());
        state["payment_attempts"][0]["amount"] = json!("1");
        db.execute("UPDATE cases SET execution='UNATTEMPTED'", [])
            .unwrap();
        assert!(guard.validate_state(&state).is_err());
    }
    #[test]
    fn resolved_previous_run_payments_keep_original_charges_and_allow_fresh_runs() {
        let (_dir, guard, db, mut state) = fixture();
        db.execute_batch("ALTER TABLE cases ADD COLUMN reservation INTEGER; ALTER TABLE cases ADD COLUMN settlement TEXT; INSERT INTO cases(run,id,execution,reservation,settlement) VALUES('previous','paid','COMPLETED',20000,'USED');").unwrap();
        let payer = format!("0x{:040x}", 1);
        let attempt = json!({"id":"attempt","pool":"p","wallet":"a","generation":0,"amount":"7","requirements_hash":"hash","payer":payer,"state":"RESOLVED"});
        let event = json!({"attempt_id":"attempt","case":"paid","pool":"p","wallet":"a","generation":0,"amount":"7","requirements_hash":"hash","address":payer});
        state["payment_attempts"] = json!([attempt]);
        state["payment_resolutions"] = json!([{"attempt_id":"attempt","outcome":"USED","height":100,"block_time":1000,"hash":format!("0x{:064x}",1)}]);
        db.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES('previous','application_payment',?1,0)",
            [event.to_string()],
        )
        .unwrap();
        guard.validate_state(&state).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT reservation FROM cases WHERE run='previous'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            20000
        );
        for (field, value) in [
            ("state", json!("POSSIBLY_SUBMITTED")),
            ("amount", json!("8")),
            ("requirements_hash", json!("changed")),
            ("payer", json!(format!("0x{:040x}", 2))),
        ] {
            let mut wrong = state.clone();
            wrong["payment_attempts"][0][field] = value;
            assert!(guard.validate_state(&wrong).is_err(), "{field}");
        }
        let mut unknown = state.clone();
        unknown["payment_resolutions"] = json!([]);
        assert!(guard.validate_state(&unknown).is_err());
        db.execute(
            "UPDATE cases SET settlement='PENDING' WHERE run='previous'",
            [],
        )
        .unwrap();
        assert!(guard.validate_state(&state).is_err());
        db.execute(
            "UPDATE cases SET settlement='EXPIRED_UNUSED' WHERE run='previous'",
            [],
        )
        .unwrap();
        assert!(guard.validate_state(&state).is_err());
        state["payment_resolutions"][0]["outcome"] = json!("EXPIRED_UNUSED");
        guard.validate_state(&state).unwrap();
        db.execute("UPDATE cases SET reservation=0 WHERE run='previous'", [])
            .unwrap();
        assert!(guard.validate_state(&state).is_err());
        db.execute(
            "UPDATE cases SET reservation=20000 WHERE run='previous'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE cases SET execution='UNATTEMPTED' WHERE run='previous'",
            [],
        )
        .unwrap();
        assert!(guard.validate_state(&state).is_err());
        db.execute(
            "UPDATE cases SET execution='SKIPPED_TARGET_REACHED' WHERE run='previous'",
            [],
        )
        .unwrap();
        assert!(guard.validate_state(&state).is_err());
        db.execute(
            "UPDATE cases SET execution='COMPLETED' WHERE run='previous'",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES('another','application_payment',?1,0)",
            [event.to_string()],
        )
        .unwrap();
        assert!(guard.validate_state(&state).is_err());
    }
    #[test]
    fn unsigned_admission_cleanup_cannot_erase_signed_liabilities() {
        let (_dir, guard, db, mut state) = fixture();
        let clean = state.clone();
        state["payment_attempts"] = json!([{"id":"attempt","pool":"p","wallet":"a","generation":0,"amount":"1","state":"ADMITTED"}]);
        guard.validate_state(&state).unwrap();
        baseline(&db, &state);
        guard.validate_state(&clean).unwrap();
        state["payment_attempts"][0]["state"] = json!("POSSIBLY_SUBMITTED");
        baseline(&db, &state);
        assert!(guard.validate_state(&clean).is_err());
        assert!(guard.validate_state(&state).is_err());
    }
}
