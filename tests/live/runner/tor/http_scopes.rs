//! Project application-recorded HTTP scopes into one registered Tor interval.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use x402_treazury::network::ScopeObservation;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    session: String,
    observation: ScopeObservation,
}
pub fn from_report(report: &Value, sessions: &BTreeSet<String>) -> Result<Vec<ScopeObservation>> {
    let events = report["runtime_events"]
        .as_array()
        .context("application runtime events missing")?;
    let mut registered = BTreeSet::new();
    for event in events
        .iter()
        .filter(|event| event["kind"] == "application_session")
    {
        let session = event["detail"]["session"]
            .as_str()
            .context("application session identity missing")?;
        ensure!(
            crate::manifest::identifier(session) && registered.insert(session.to_owned()),
            "invalid or duplicate application session"
        );
    }
    ensure!(
        sessions.is_subset(&registered),
        "HTTP scope interval contains unknown sessions"
    );
    let mut observations = Vec::new();
    for event in events
        .iter()
        .filter(|event| event["kind"] == "application_network_scope")
    {
        let detail: Detail = serde_json::from_value(event["detail"].clone())?;
        ensure!(
            registered.contains(&detail.session),
            "HTTP scope belongs to an unregistered application session"
        );
        if sessions.contains(&detail.session) {
            observations.push(detail.observation);
        }
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn observation(scope: &str) -> Value {
        json!({"canonical":["owner-user","owner-password"],"scope":["scope-user",scope],"origin":"https://provider.example"})
    }
    fn report() -> Value {
        json!({"runtime_events":[
            {"kind":"application_session","detail":{"session":"earlier"}},
            {"kind":"application_session","detail":{"session":"current"}},
            {"kind":"application_network_scope","detail":{"session":"earlier","observation":observation("old-scope")}},
            {"kind":"application_network_scope","detail":{"session":"current","observation":observation("new-scope")}}
        ]})
    }
    #[test]
    fn only_selected_registered_interval_scopes_are_projected() {
        let document = report();
        let observations = from_report(&document, &BTreeSet::from(["current".into()])).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].scope.1, "new-scope");
        assert!(from_report(&document, &BTreeSet::new()).unwrap().is_empty());
        assert!(from_report(&document, &BTreeSet::from(["missing".into()])).is_err());
    }
    #[test]
    fn unknown_sessions_and_malformed_scope_records_fail_even_outside_interval() {
        let selected = BTreeSet::from(["current".into()]);
        for field in ["session", "observation"] {
            let mut document = report();
            document["runtime_events"][2]["detail"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(from_report(&document, &selected).is_err());
        }
        let mut document = report();
        document["runtime_events"][2]["detail"]["session"] = json!("unregistered");
        assert!(from_report(&document, &selected).is_err());
        let mut document = report();
        document["runtime_events"][2]["detail"]["extra"] = json!(true);
        assert!(from_report(&document, &selected).is_err());
        let mut document = report();
        document["runtime_events"][1]["detail"]["session"] = json!("earlier");
        assert!(from_report(&document, &BTreeSet::new()).is_err());
    }
}
