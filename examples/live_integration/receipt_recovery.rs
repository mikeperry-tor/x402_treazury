//! Read-only recovery of already recorded payments. Never starts the application.
use crate::{
    files,
    process::{Process, StopSignals},
    registry::Registry,
    tor,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};
use x402_treazury::{
    deployment::MetaConfig,
    network::{self, IsolationId, NetworkContext, NetworkPolicy},
    qualification::receipt_expectation,
    rotation::{base::BaseRpc, store},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    version: u32,
    policy: NetworkPolicy,
    urls: Vec<String>,
    confirmations: u64,
    max_age: u64,
    snapshot: Value,
    candidates: Vec<Candidate>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    payment: Value,
    receipt: Value,
}

fn select(report: &Value, snapshot: &Value) -> Result<Vec<Candidate>> {
    let events = report["runtime_events"]
        .as_array()
        .context("runtime evidence missing")?;
    let mut candidates = Vec::new();
    for event in events.iter().filter(|e| e["kind"] == "application_payment") {
        let payment = &event["detail"];
        if events.iter().any(|e| {
            e["kind"] == "application_debit_verified"
                && e["detail"]["attempt_id"] == payment["attempt_id"]
        }) {
            continue;
        }
        let mut receipts = events.iter().filter(|e| {
            e["kind"] == "application_receipt" && e["detail"]["attempt_id"] == payment["attempt_id"]
        });
        let Some(receipt) = receipts.next() else {
            continue;
        };
        ensure!(receipts.next().is_none(), "duplicate receipt observation");
        let receipt = &receipt["detail"];
        if receipt["receipt"]["classification"] != "seller_success" {
            continue;
        }
        receipt_expectation(payment, receipt, snapshot)?;
        candidates.push(Candidate {
            payment: payment.clone(),
            receipt: receipt.clone(),
        });
    }
    ensure!(
        candidates.len() <= 10000,
        "receipt recovery exceeds 10000-candidate limit; no observations dispatched"
    );
    Ok(candidates)
}
fn identities(task: &Task) -> Result<tor::audit::Identities> {
    let context = NetworkContext::new(task.policy.clone())?;
    let targets: std::collections::BTreeSet<String> = task
        .urls
        .iter()
        .map(|url| {
            let url = reqwest::Url::parse(url)?;
            Ok(format!(
                "{}:{}",
                url.host_str().context("RPC host missing")?,
                url.port_or_known_default().context("RPC port missing")?
            )
            .to_ascii_lowercase())
        })
        .collect::<Result<_>>()?;
    let mut map = tor::audit::Identities::new();
    for candidate in &task.candidates {
        let expected = receipt_expectation(&candidate.payment, &candidate.receipt, &task.snapshot)?;
        let address = expected.payer.to_string();
        let (user, password) = context.credentials(&IsolationId::evm(&address)?);
        map.insert(
            format!("payer_{}", files::hash(&address)),
            tor::audit::Identity {
                user,
                password,
                kind: "evm".into(),
                targets: targets.clone(),
                required: true,
                required_targets: Default::default(),
                permitted: true,
            },
        );
    }
    Ok(map)
}
/// Hidden subprocess entry point: receives public journal facts, no wallet key or registry handle.
pub async fn observe(path: &Path, digest: &str) -> Result<u8> {
    files::regular(path)?;
    let bytes = files::read(path)?;
    ensure!(files::hash(&bytes) == digest, "receipt task hash mismatch");
    let task: Task = serde_json::from_slice(&bytes)?;
    ensure!(
        task.version == 1 && task.candidates.len() <= 10000,
        "invalid receipt task version or 10000-candidate limit exceeded"
    );
    ensure!(
        task.policy.mode == network::Mode::Tor,
        "receipt observer requires Tor"
    );
    let expected = task
        .candidates
        .iter()
        .map(|c| receipt_expectation(&c.payment, &c.receipt, &task.snapshot))
        .collect::<Result<Vec<_>>>()?;
    network::install(task.policy)?;
    let rpc = BaseRpc::with_fallbacks(&task.urls, task.confirmations, task.max_age)?;
    let mut results = Vec::new();
    for (index, expected) in expected.iter().enumerate() {
        eprintln!(
            "Read-only receipt observation {}/{}; no signing or paid requests",
            index + 1,
            task.candidates.len()
        );
        let result = match rpc.verify_transfer(expected).await {
            Ok(Some(proof)) => json!({"status":"verified","proof":proof}),
            Ok(None) => json!({"status":"pending"}),
            Err(error) => {
                json!({"status":"unavailable","reason":x402_treazury::rotation::base::safe_diagnostic(&error)})
            }
        };
        results.push(result);
    }
    println!(
        "{}",
        json!({"version":1,"task_sha256":digest,"results":results})
    );
    Ok(0)
}
fn validate_results(
    task: &Task,
    digest: &str,
    output: &Value,
    observation: &str,
) -> Result<Vec<Value>> {
    ensure!(
        output["version"] == 1 && output["task_sha256"] == digest,
        "receipt observation task mismatch"
    );
    let rows = output["results"]
        .as_array()
        .context("receipt results missing")?;
    ensure!(
        rows.len() == task.candidates.len(),
        "receipt observation incomplete"
    );
    let mut result = Vec::new();
    for (candidate, row) in task.candidates.iter().zip(rows) {
        let expected = receipt_expectation(&candidate.payment, &candidate.receipt, &task.snapshot)?;
        ensure!(
            matches!(
                row["status"].as_str(),
                Some("verified" | "pending" | "unavailable")
            ),
            "unknown receipt observation status"
        );
        if row["status"] == "verified" {
            let proof = &row["proof"];
            for (field, value) in [
                ("transaction", expected.transaction.to_string()),
                ("payer", expected.payer.to_string()),
                ("payee", expected.payee.to_string()),
                ("amount_atomic", expected.amount.to_string()),
                ("nonce", expected.nonce.to_string()),
            ] {
                ensure!(
                    proof[field] == value,
                    "receipt proof differs from journal: {field}"
                );
            }
            let _: alloy_primitives::B256 = proof["block_hash"]
                .as_str()
                .context("receipt proof block hash missing")?
                .parse()?;
            ensure!(
                proof["block_height"].as_u64().is_some(),
                "receipt proof block height missing"
            );
        } else {
            ensure!(
                row.get("proof").is_none(),
                "unverified result contains a proof"
            );
        }
        let mut event = row.clone();
        for field in ["case", "session", "attempt_id"] {
            event[field] = candidate.payment[field].clone();
        }
        event["producer"] = json!("read_only_receipt_observer");
        event["observation"] = json!(observation);
        result.push(event);
    }
    Ok(result)
}

pub async fn reconcile(state: &Path, run: &str, evidence: &Path) -> Result<u8> {
    let mut signals = StopSignals::install()?;
    let registry = Registry::open(state, false)?;
    let _owner = files::lock(&state.join("owner.lock"))?;
    let manifest = registry.manifest(run)?;
    ensure!(
        manifest.network.tor_mode == crate::manifest::TorMode::Owned,
        "receipt recovery requires an owned-Tor run"
    );
    let pins = registry.pins(run)?;
    let catalogs = pins.catalogs.as_ref().context("catalog pins missing")?;
    let archive = catalogs.verify_archive(&manifest)?;
    registry.application_evidence(run)?;
    let snapshot = store::qualification_state(state)?;
    ensure!(
        snapshot["treasury_status"]["treasury_id"] == manifest.treasury_id,
        "receipt recovery treasury mismatch"
    );
    let config: MetaConfig = serde_json::from_value(archive["deployment"].clone())?;
    let funding = config.funding.context("funding config missing")?;
    let candidates = select(&registry.report(Some(run), now()?)?, &snapshot)?;
    if candidates.is_empty() {
        println!("No unverified successful seller receipts to observe; no requests sent");
        return Ok(0);
    }
    let task = Task {
        version: 1,
        policy: config.network,
        urls: funding.base_rpc_urls(|name| std::env::var(name).ok())?,
        confirmations: funding.base_confirmations,
        max_age: funding.base_max_block_age_seconds,
        snapshot,
        candidates,
    };
    ensure!(
        task.policy.mode == network::Mode::Tor,
        "receipt recovery requires Tor policy"
    );
    let expected = identities(&task)?;
    files::create_dir(evidence)?;
    let evidence = evidence.canonicalize()?;
    let bytes = serde_json::to_vec(&task)?;
    let digest = files::hash(&bytes);
    let task_path = evidence.join("task.json");
    files::publish(&task_path, &bytes)?;
    files::publish(
        &evidence.join("identities.json"),
        &serde_json::to_vec_pretty(&expected)?,
    )?;
    let runner = std::env::current_exe()?.canonicalize()?;
    files::publish(
        &evidence.join("observer.json"),
        &serde_json::to_vec_pretty(
            &json!({"version":1,"run":run,"binary_sha256":files::hash_file(&runner)?,"catalog_snapshot_sha256":catalogs.files["snapshot.json"],"task_sha256":digest,"read_only":true}),
        )?,
    )?;
    let profile = evidence.join("client.sb");
    files::publish(
        &profile,
        tor::confinement::profile(
            task.policy
                .socks_endpoint
                .context("SOCKS endpoint missing")?
                .port(),
            &[],
        )?
        .as_bytes(),
    )?;
    tor::confinement::qualify_with_cancel(&runner, &profile, &evidence, signals.stop.clone())
        .await?;
    let tor = tor::owned::Tor::start_with_cancel(
        manifest
            .network
            .tor_binary
            .as_deref()
            .context("Tor binary missing")?,
        &state.join("live-integration/tor-state"),
        &evidence,
        &task.policy,
        signals.stop.clone(),
    )
    .await?;
    signals.observe_faults(tor.faults());
    let outcome = async {
        let child = Process::launch_confined(
            &runner,
            &[
                "observe-receipts".into(),
                "--task".into(),
                task_path.to_str().context("task path must be UTF8")?.into(),
                "--digest".into(),
                digest.clone(),
            ],
            &BTreeMap::new(),
            &evidence,
            files::DOCUMENT_BYTES,
            Some(&profile),
        )?;
        let result = child
            .wait(
                Duration::from_secs(900),
                Duration::from_secs(10),
                signals.stop.clone(),
            )
            .await?;
        files::publish(&evidence.join("observer.stdout"), &result.stdout.bytes)?;
        files::publish(&evidence.join("observer.stderr"), &result.stderr.bytes)?;
        files::publish(
            &evidence.join("observer.process.json"),
            &serde_json::to_vec_pretty(&result)?,
        )?;
        ensure!(
            result.success
                && result.valid_output()
                && !result.forced_kill
                && !signals.stop.is_cancelled(),
            "receipt observer incomplete; no proofs imported, paid cases never replayed"
        );
        validate_results(
            &task,
            &digest,
            &serde_json::from_slice(&result.stdout.bytes)?,
            &digest,
        )
    }
    .await;
    // Always reap the owned Tor, including launch/parse/cancellation failures.
    let events = tor.stop(&evidence).await?;
    files::publish(
        &evidence.join("control-completion.json"),
        &serde_json::to_vec_pretty(
            &json!({"version":1,"complete":true,"event_count":events.len(),"events_sha256":files::hash_file(&evidence.join("tor-events.log"))?}),
        )?,
    )?;
    let observations = outcome?;
    let audit = tor::audit::verify(&expected, &events)?;
    files::publish(
        &evidence.join("stream-audit.json"),
        &serde_json::to_vec_pretty(&audit)?,
    )?;
    ensure!(
        !signals.stop.is_cancelled(),
        "receipt observation cancelled; no proofs imported"
    );
    registry.record_receipt_observations(run, &observations, now()?)?;
    let verified = observations
        .iter()
        .filter(|e| e["status"] == "verified")
        .count();
    println!(
        "Read-only Tor receipt recovery: {verified}/{} canonical debits verified; no payments replayed or reservations released",
        observations.len()
    );
    Ok(if verified == observations.len() { 0 } else { 3 })
}
fn now() -> Result<i64> {
    Ok(i64::try_from(x402_treazury::rotation::base::now()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Value) {
        let payment = json!({"case":"case","session":"session","attempt_id":"attempt","pool":"pool","wallet":"wallet","generation":0,"amount":"7","requirements_hash":"hash","address":format!("0x{:040x}",1)});
        let receipt = json!({"case":"case","session":"session","attempt_id":"attempt","receipt":{"classification":"seller_success","network":"eip155:8453","transaction":format!("0x{:064x}",2)}});
        let snapshot = json!({"payment_attempts":[{"id":"attempt","pool":"pool","wallet":"wallet","generation":0,"amount":"7","requirements_hash":"hash","state":"RESOLVED","payer":format!("0x{:040x}",1),"payee":format!("0x{:040x}",3),"nonce":format!("0x{:064x}",4)}]});
        (
            json!({"runtime_events":[{"kind":"application_payment","detail":payment},{"kind":"application_receipt","detail":receipt}]}),
            snapshot,
        )
    }
    #[test]
    fn selection_never_replays_verified_or_unsuccessful_receipts() {
        let (mut report, snapshot) = fixture();
        assert_eq!(select(&report, &snapshot).unwrap().len(), 1);
        report["runtime_events"][1]["detail"]["receipt"]["classification"] =
            json!("seller_failure");
        assert!(select(&report, &snapshot).unwrap().is_empty());
        let (mut report, snapshot) = fixture();
        report["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"application_debit_verified","detail":{"attempt_id":"attempt"}}));
        assert!(select(&report, &snapshot).unwrap().is_empty());
        let (mut report, snapshot) = fixture();
        let duplicate = report["runtime_events"][1].clone();
        report["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(select(&report, &snapshot).is_err());
    }
    #[test]
    fn recovered_proofs_must_match_task_and_journal_in_full() {
        let (report, snapshot) = fixture();
        let task = Task {
            version: 1,
            policy: NetworkPolicy::default(),
            urls: vec![],
            confirmations: 12,
            max_age: 120,
            candidates: select(&report, &snapshot).unwrap(),
            snapshot,
        };
        let expected = receipt_expectation(
            &task.candidates[0].payment,
            &task.candidates[0].receipt,
            &task.snapshot,
        )
        .unwrap();
        let output = json!({"version":1,"task_sha256":"digest","results":[{"status":"verified","proof":{"transaction":expected.transaction.to_string(),"payer":expected.payer.to_string(),"payee":expected.payee.to_string(),"amount_atomic":"7","nonce":expected.nonce.to_string(),"block_height":1,"block_hash":format!("0x{:064x}",5)}}]});
        let result = validate_results(&task, "digest", &output, "observation").unwrap();
        assert_eq!(result[0]["session"], "session");
        assert_eq!(result[0]["attempt_id"], "attempt");
        assert!(validate_results(&task, "other", &output, "observation").is_err());
        for field in [
            "transaction",
            "payer",
            "payee",
            "amount_atomic",
            "nonce",
            "block_hash",
            "block_height",
        ] {
            let mut wrong = output.clone();
            wrong["results"][0]["proof"][field] = Value::Null;
            assert!(
                validate_results(&task, "digest", &wrong, "observation").is_err(),
                "{field}"
            );
        }
        let mut wrong = output.clone();
        wrong["results"][0]["status"] = json!("pending");
        assert!(validate_results(&task, "digest", &wrong, "observation").is_err());
        wrong["results"][0].as_object_mut().unwrap().remove("proof");
        validate_results(&task, "digest", &wrong, "observation").unwrap();
        wrong["results"] = json!([]);
        assert!(validate_results(&task, "digest", &wrong, "observation").is_err());
    }
}
