//! Offline interpretation of retained authenticated control evidence. No execution authority.
use super::{audit, identities};
use crate::{files, registry::Registry};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;

fn document(path: &Path) -> Result<Value> {
    files::regular(path)?;
    Ok(serde_json::from_slice(&files::read(path)?)?)
}
fn complete(marker: Option<&Value>, bytes: &[u8], count: usize) -> Result<bool> {
    let Some(marker) = marker else {
        return Ok(false);
    };
    ensure!(
        marker["version"] == 1
            && marker["complete"] == true
            && marker["event_count"].as_u64() == Some(count as u64)
            && marker["events_sha256"] == files::hash(bytes),
        "Tor control completion marker disagrees with retained events"
    );
    Ok(true)
}
fn initial_identities(
    initial: &audit::Identities,
    current: &audit::Identities,
) -> Result<Vec<String>> {
    let mut corrections = Vec::new();
    for (label, identity) in current
        .iter()
        .filter(|(label, id)| id.kind != "evm" && !initial.contains_key(*label))
    {
        // Older managed exports omitted the public NEAR token-catalog identity.
        // Permit only this fixed production derivation, and only when the same
        // destination was already authorized for an EVM identity. Never infer
        // credentials or destinations from observed STREAM events.
        let origin = x402_treazury::rotation::near::ORIGIN;
        let url = reqwest::Url::parse(origin)?;
        let target = format!("{}:443", url.host_str().context("NEAR host missing")?);
        ensure!(
            label == &format!("discovery_{}", files::hash(origin))
                && identity.kind == "discovery"
                && identity.permitted
                && !identity.required
                && identity.required_targets.is_empty()
                && identity.targets == [target.clone()].into()
                && initial
                    .values()
                    .any(|old| old.kind == "evm" && old.permitted && old.targets.contains(&target)),
            "discovery/treasury identity set differs from original observation"
        );
        corrections.push("near_token_catalog_discovery_omitted_by_original_export".into());
    }
    for (label, old) in initial {
        let new = current
            .get(label)
            .context("original identity disappeared")?;
        ensure!(
            old.user == new.user && old.password == new.password && old.kind == new.kind,
            "retained identity credentials differ from production derivation"
        );
        if old.kind != "evm" {
            ensure!(
                old.targets == new.targets,
                "configured observation endpoints differ from original identity export"
            );
        }
    }
    Ok(corrections)
}
pub fn audit_run(state: &Path, run: &str, output: &Path) -> Result<u8> {
    let registry = Registry::open(state, true)?;
    // Do not race a serving owner or read its incomplete wallet transitions.
    let _treasury = files::lock(&state.join("owner.lock"))?;
    let manifest = registry.manifest(run)?;
    let pins = registry.pins(run)?;
    let catalogs = pins
        .catalogs
        .as_ref()
        .context("run has no frozen catalog evidence")?;
    let snapshot = catalogs.verify_archive(&manifest)?;
    ensure!(
        manifest.network.tor_mode == crate::manifest::TorMode::Owned,
        "offline Tor audit requires an originally owned-Tor run"
    );
    let current = manifest.evidence_dir.join(format!("{run}-tor-run"));
    let directory = if current.try_exists()? {
        current
    } else {
        manifest.evidence_dir.join(format!("{run}-tor"))
    };
    if directory.join("session-scope.json").try_exists()? {
        let report = registry.report(
            Some(run),
            i64::try_from(x402_treazury::rotation::base::now()?)?,
        )?;
        let count = super::resume::verify_intervals(state, &manifest, &report, &snapshot)?;
        let result = json!({"version":1,"run":run,"qualification":"qualified","tor_intervals":count,
            "scope":"offline session-scoped Tor stream and confinement evidence only; no payment, cover or complete unlinkability claim"});
        files::publish(output, &serde_json::to_vec_pretty(&result)?)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(0);
    }

    files::directory(&directory)?;
    let process = document(&directory.join("tor.process.json"))?;
    ensure!(
        process["success"] == true
            && process["valid_output"] == true
            && process["forced_kill"] == false,
        "owned Tor process did not retain complete clean shutdown evidence"
    );
    let confinement = document(&directory.join("confinement.json"))?;
    ensure!(
        confinement["status"] == "qualified",
        "original confinement checks incomplete"
    );
    let report = registry.report(
        Some(run),
        i64::try_from(x402_treazury::rotation::base::now()?)?,
    )?;
    let payments: Vec<_> = report["runtime_events"]
        .as_array()
        .context("runtime events missing")?
        .iter()
        .filter(|e| e["kind"] == "application_payment")
        .map(|e| e["detail"].clone())
        .collect();
    let expected = if manifest.start.pools.is_empty() {
        if directory.ends_with(format!("{run}-tor-run")) {
            identities::unsigned_map_frozen(&manifest, &snapshot, state)?
        } else {
            identities::unsigned_map(&manifest, &snapshot, state)?
        }
    } else {
        identities::managed_map(
            &manifest,
            &snapshot,
            state,
            |name| std::env::var(name).ok(),
            &payments,
        )?
    };
    let initial: audit::Identities =
        serde_json::from_value(document(&directory.join("identities.json"))?)?;
    let corrections = initial_identities(&initial, &expected)?;
    let event_path = directory.join("tor-events.log");
    files::regular(&event_path)?;
    let bytes = files::read(&event_path)?;
    ensure!(
        bytes.is_empty() || bytes.ends_with(b"\n"),
        "incomplete trailing Tor event line"
    );
    let events: Vec<_> = std::str::from_utf8(&bytes)?
        .lines()
        .map(str::to_owned)
        .collect();
    let marker_path = directory.join("control-completion.json");
    let marker = if marker_path.try_exists()? || marker_path.is_symlink() {
        Some(document(&marker_path)?)
    } else {
        None
    };
    let complete = complete(marker.as_ref(), &bytes, events.len())?;
    let result = json!({"version":1,"run":run,"mode":"offline_reaudit",
        "events_sha256":files::hash(&bytes),"event_count":events.len(),
        "catalog_snapshot_sha256":catalogs.files["snapshot.json"],
        "initial_identities_sha256":files::hash_file(&directory.join("identities.json"))?,
        "identity_export_corrections":corrections,
        "control_evidence_complete":complete,"stream_checks":audit::verify(&expected,&events)?,
        "status":if complete {"stream_evidence_verified"} else {"observed_streams_only"},
        "scope":"Later read-only Tor stream audit; no execution, payment, scenario-completion or complete-unlinkability claim"});
    files::publish(output, &serde_json::to_vec_pretty(&result)?)?;
    if !complete {
        eprintln!(
            "Tor re-audit: recorded streams pass, but the original run lacks a typed control-completion marker; whole-session isolation remains unqualified"
        );
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(if complete { 0 } else { 3 })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_near_discovery_correction_cannot_add_destinations() {
        let origin = x402_treazury::rotation::near::ORIGIN;
        let label = format!("discovery_{}", files::hash(origin));
        let target = "1click.chaindefuser.com:443".to_owned();
        let wallet = audit::Identity {
            user: "user".into(),
            password: "wallet".into(),
            kind: "evm".into(),
            targets: [target.clone()].into(),
            permitted: true,
            required: false,
            required_targets: Default::default(),
        };
        let initial = audit::Identities::from([("evm_pool_wallet".into(), wallet.clone())]);
        let mut current = initial.clone();
        current.insert(
            label.clone(),
            audit::Identity {
                password: "production-derived-discovery".into(),
                kind: "discovery".into(),
                ..wallet
            },
        );
        assert_eq!(
            initial_identities(&initial, &current).unwrap(),
            ["near_token_catalog_discovery_omitted_by_original_export"]
        );
        assert!(initial_identities(&current, &current).unwrap().is_empty());
        assert!(initial_identities(&audit::Identities::new(), &current).is_err());
        let mut unpermitted = initial.clone();
        unpermitted.values_mut().next().unwrap().permitted = false;
        assert!(initial_identities(&unpermitted, &current).is_err());
        let mut changed = current.clone();
        changed
            .get_mut(&label)
            .unwrap()
            .targets
            .insert("other.example:443".into());
        assert!(initial_identities(&initial, &changed).is_err());
        let mut changed = current.clone();
        changed.get_mut(&label).unwrap().required = true;
        assert!(initial_identities(&initial, &changed).is_err());
        let mut changed = current.clone();
        let identity = changed.remove(&label).unwrap();
        changed.insert("other_discovery".into(), identity);
        assert!(initial_identities(&initial, &changed).is_err());
    }
    #[test]
    fn reaudit_preserves_original_tokens_and_endpoint_constraints() {
        let identity = audit::Identity {
            user: "user".into(),
            password: "token".into(),
            kind: "treasury".into(),
            targets: ["indexer.example:443".into()].into(),
            required: true,
            required_targets: Default::default(),
            permitted: true,
        };
        let initial = audit::Identities::from([("treasury".into(), identity)]);
        initial_identities(&initial, &initial).unwrap();
        let mut changed = initial.clone();
        changed.get_mut("treasury").unwrap().password = "different".into();
        assert!(initial_identities(&initial, &changed).is_err());
        let mut changed = initial.clone();
        changed
            .get_mut("treasury")
            .unwrap()
            .targets
            .insert("new.example:443".into());
        assert!(initial_identities(&initial, &changed).is_err());
        assert!(initial_identities(&initial, &audit::Identities::new()).is_err());
    }
    #[test]
    fn missing_or_changed_completion_evidence_cannot_qualify_a_session() {
        let bytes = b"650 CIRC 1 BUILT\n";
        assert!(!complete(None, bytes, 1).unwrap());
        let marker =
            json!({"version":1,"complete":true,"event_count":1,"events_sha256":files::hash(bytes)});
        assert!(complete(Some(&marker), bytes, 1).unwrap());
        assert!(complete(Some(&marker), b"650 CIRC 2 BUILT\n", 1).is_err());
        assert!(complete(Some(&marker), bytes, 2).is_err());
    }
}
