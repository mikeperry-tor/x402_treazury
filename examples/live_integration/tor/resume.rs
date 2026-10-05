//! Read-only verification of retained Tor intervals, including historical continuations.
use super::{audit, identities, scope};
use crate::{files, manifest::Manifest};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

fn document(path: &Path) -> Result<Value> {
    files::regular(path)?;
    Ok(serde_json::from_slice(&files::read(path)?)?)
}
fn validate_identity(old: &audit::Identities, derived: &audit::Identities) -> Result<()> {
    for (name, entry) in old {
        let expected = derived
            .get(name)
            .context("prior Tor identity is not derived from this deployment")?;
        ensure!(
            entry.user == expected.user
                && entry.password == expected.password
                && entry.kind == expected.kind
                && entry.targets == expected.targets
                && entry.permitted == expected.permitted
                && entry.required == expected.required
                && entry.required_targets == expected.required_targets,
            "prior Tor credentials or destinations differ from production derivation"
        );
    }
    ensure!(
        derived
            .iter()
            .filter(|(_, v)| v.required)
            .all(|(name, _)| old.contains_key(name)),
        "prior Tor map omitted a required identity"
    );
    Ok(())
}
fn interval(
    path: &Path,
    state: &Path,
    m: &Manifest,
    report: &Value,
    snapshot: &Value,
    fetched: bool,
) -> Result<BTreeSet<String>> {
    let scope = document(&path.join("session-scope.json"))?;
    ensure!(
        scope["version"] == 1,
        "prior Tor interval lacks supported session scope; observation only"
    );
    ensure!(
        scope["catalogs_fetched"] == fetched,
        "Tor interval catalog scope differs from its preparation kind"
    );
    let raw_ids: Vec<String> = serde_json::from_value(scope["sessions"].clone())?;
    let ids: BTreeSet<String> = raw_ids.iter().cloned().collect();
    ensure!(ids.len() == raw_ids.len(), "duplicate Tor interval session");
    let projected = scope::Sessions::including(report, &ids)?.project(m, report)?;
    let derived = if fetched {
        identities::managed_map(
            &projected.manifest,
            snapshot,
            state,
            |n| std::env::var(n).ok(),
            &projected.payments,
        )?
    } else {
        identities::managed_map_frozen(
            &projected.manifest,
            snapshot,
            state,
            |n| std::env::var(n).ok(),
            &projected.payments,
        )?
    };
    files::directory(path)?;
    let process = document(&path.join("tor.process.json"))?;
    ensure!(
        process["success"] == true
            && process["valid_output"] == true
            && process["forced_kill"] == false,
        "prior Tor process lacks complete clean shutdown evidence"
    );
    ensure!(
        document(&path.join("confinement.json"))?["status"] == "qualified",
        "prior confinement was not qualified"
    );
    let map_path = path.join("identities-final.json");
    files::regular(&map_path)?;
    let map: audit::Identities = serde_json::from_slice(&files::read(&map_path)?)?;
    validate_identity(&map, &derived)?;
    let bytes = files::read(&path.join("tor-events.log"))?;
    let text = std::str::from_utf8(&bytes)?;
    let events: Vec<_> = text.lines().map(str::to_owned).collect();
    let complete = document(&path.join("control-completion.json"))?;
    ensure!(
        complete["version"] == 1
            && complete["complete"] == true
            && complete["event_count"].as_u64() == Some(events.len() as u64)
            && complete["events_sha256"] == files::hash(&bytes),
        "prior Tor control evidence is incomplete or changed"
    );
    audit::verify(&map, &events)?;
    Ok(ids)
}
fn interval_kind(run: &str, name: &str) -> Option<bool> {
    if name == format!("{run}-tor-run") {
        return Some(false);
    }
    if name == format!("{run}-tor") {
        return Some(true);
    }
    name.strip_prefix(&format!("{run}-tor-resume_"))
        .filter(|suffix| uuid::Uuid::parse_str(suffix).is_ok_and(|id| id.to_string() == *suffix))
        .map(|_| false)
}
pub fn verify_intervals(
    state: &Path,
    m: &Manifest,
    report: &Value,
    snapshot: &Value,
) -> Result<usize> {
    let sessions = scope::Sessions::prior(report)?;
    let mut count = 0usize;
    let mut covered = BTreeSet::new();
    for entry in std::fs::read_dir(&m.evidence_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().context("non-UTF8 Tor evidence path")?;
        if let Some(fetched) = interval_kind(&m.run_id, name) {
            count += 1;
            ensure!(
                count <= 1000,
                "resume exceeds 1000 prior Tor intervals; no evidence omitted"
            );
            let ids = interval(&entry.path(), state, m, report, snapshot, fetched)?;
            for id in ids {
                ensure!(
                    covered.insert(id),
                    "application session claimed by two Tor intervals"
                );
            }
        }
    }
    ensure!(
        count > 0,
        "paid Tor resume has no prior supervision evidence"
    );
    ensure!(
        &covered == sessions.ids(),
        "prior application sessions lack complete Tor interval coverage"
    );
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn other_runs_with_similar_names_are_not_adopted_as_intervals() {
        let suffix = "11111111-1111-4111-8111-111111111111";
        assert_eq!(interval_kind("run", "run-tor"), Some(true));
        assert_eq!(
            interval_kind("run", &format!("run-tor-resume_{suffix}")),
            Some(false)
        );
        assert_eq!(
            interval_kind("run", &format!("run-tor-resume_{suffix}-tor")),
            None
        );
        assert_eq!(interval_kind("run", "run-tor-resume_not_a_uuid"), None);
        assert_eq!(interval_kind("run", "other-tor"), None);
    }
    #[test]
    fn retained_maps_cannot_widen_destinations_or_change_credentials() {
        let identity = audit::Identity {
            user: "user".into(),
            password: "password".into(),
            kind: "evm".into(),
            targets: BTreeSet::from(["api.test:443".into()]),
            required: true,
            required_targets: BTreeSet::new(),
            permitted: true,
        };
        let derived = audit::Identities::from([("payer".into(), identity)]);
        validate_identity(&derived, &derived).unwrap();
        for field in ["host", "password", "permission", "required"] {
            let mut changed = derived.clone();
            let payer = changed.get_mut("payer").unwrap();
            match field {
                "host" => {
                    payer.targets.insert("other.test:443".into());
                }
                "password" => payer.password = "changed".into(),
                "required" => payer.required = false,
                _ => payer.permitted = false,
            }
            assert!(validate_identity(&changed, &derived).is_err());
        }
        assert!(validate_identity(&derived, &audit::Identities::new()).is_err());
        assert!(validate_identity(&audit::Identities::new(), &derived).is_err());
    }
}
