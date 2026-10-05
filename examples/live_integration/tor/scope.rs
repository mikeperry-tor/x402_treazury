//! Attribute network requirements to one supervision interval, never older payments.
use crate::{manifest::Manifest, manifest::identifier};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Sessions {
    prior: BTreeSet<String>,
}
pub struct Projection {
    pub manifest: Manifest,
    pub payments: Vec<Value>,
    pub sessions: BTreeSet<String>,
}
fn sessions(report: &Value) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for event in report["runtime_events"]
        .as_array()
        .context("session events missing")?
        .iter()
        .filter(|e| e["kind"] == "application_session")
    {
        let session = event["detail"]["session"]
            .as_str()
            .context("application session identity missing")?;
        ensure!(
            identifier(session) && result.insert(session.to_owned()),
            "invalid or duplicate application session"
        );
    }
    Ok(result)
}
impl Sessions {
    pub fn ids(&self) -> &BTreeSet<String> {
        &self.prior
    }
    pub fn including(report: &Value, included: &BTreeSet<String>) -> Result<Self> {
        let all = sessions(report)?;
        ensure!(
            !included.is_empty() && included.is_subset(&all),
            "Tor interval contains missing or unknown sessions"
        );
        Ok(Self {
            prior: all.difference(included).cloned().collect(),
        })
    }
    /// Capture before launch. Existing sessions must retain complete clean process
    /// evidence before a later paid supervisor may proceed.
    pub fn prior(report: &Value) -> Result<Self> {
        let prior = sessions(report)?;
        let mut finished = BTreeSet::new();
        for event in report["runtime_events"]
            .as_array()
            .context("session events missing")?
            .iter()
            .filter(|e| e["kind"] == "child_finished")
        {
            let detail = &event["detail"];
            let session = detail["session"]
                .as_str()
                .context("child completion session missing")?;
            ensure!(
                prior.contains(session) && finished.insert(session.to_owned()),
                "unknown or duplicate child completion"
            );
            ensure!(
                detail["success"] == true
                    && detail["forced_kill"] == false
                    && detail["valid_output"] == true,
                "prior application did not retain complete clean shutdown evidence"
            );
        }
        ensure!(
            prior == finished,
            "prior application shutdown is incomplete; no paid continuation"
        );
        Ok(Self { prior })
    }
    pub fn project(&self, manifest: &Manifest, report: &Value) -> Result<Projection> {
        let all = sessions(report)?;
        ensure!(
            self.prior.is_subset(&all),
            "prior application session disappeared"
        );
        let current: BTreeSet<_> = all.difference(&self.prior).cloned().collect();
        let known: BTreeMap<_, _> = manifest.cases.iter().map(|c| (c.id.as_str(), c)).collect();
        let events = report["runtime_events"]
            .as_array()
            .context("session events missing")?;
        let mut claims = BTreeMap::new();
        let mut payments = Vec::new();
        let mut paid = BTreeSet::new();
        let mut all_claimed = BTreeSet::new();
        for event in events.iter().filter(|e| {
            matches!(
                e["kind"].as_str(),
                Some("application_claim" | "application_payment")
            )
        }) {
            let detail = &event["detail"];
            let session = detail["session"]
                .as_str()
                .context("network event session missing")?;
            ensure!(
                all.contains(session),
                "network event names an unregistered session"
            );
            let id = detail["case"]
                .as_str()
                .context("network event case missing")?;
            let case = known
                .get(id)
                .context("network event names an undeclared case")?;
            if event["kind"] == "application_claim" {
                ensure!(
                    all_claimed.insert(id),
                    "case replayed across application sessions"
                );
            }
            if self.prior.contains(session) {
                continue;
            }
            if event["kind"] == "application_claim" {
                ensure!(
                    detail["server"] == case.server
                        && detail["source"] == case.source
                        && detail["tool"] == case.tool,
                    "network claim differs from reviewed case"
                );
                ensure!(
                    claims.insert(id, session).is_none(),
                    "case claimed twice during supervision"
                );
            } else {
                ensure!(
                    !case.unsigned && claims.get(id).copied() == Some(session) && paid.insert(id),
                    "payment lacks a unique same-session paid claim"
                );
                payments.push(detail.clone());
            }
        }
        let mut scoped = manifest.clone();
        scoped
            .cases
            .retain(|case| claims.contains_key(case.id.as_str()));
        Ok(Projection {
            manifest: scoped,
            payments,
            sessions: current,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(kind: &str, session: &str) -> Value {
        json!({"kind":kind,"detail":{"session":session,"case":"call","server":"main","source":"api","tool":"api_read","success":true,"forced_kill":false,"valid_output":true}})
    }
    #[test]
    fn old_wallet_payments_cannot_satisfy_new_session_requirements() {
        let mut m = crate::tests::manifest();
        let mut old_case = m.cases[0].clone();
        old_case.id = "old_call".into();
        m.cases.push(old_case);
        let mut old_claim = event("application_claim", "old");
        old_claim["detail"]["case"] = json!("old_call");
        let mut old_payment = event("application_payment", "old");
        old_payment["detail"]["case"] = json!("old_call");
        let mut r = json!({"runtime_events":[event("application_session","old"),old_claim,old_payment,event("child_finished","old")]});
        let scope = Sessions::prior(&r).unwrap();
        let result = scope.project(&m, &r).unwrap();
        assert!(result.manifest.cases.is_empty());
        assert!(result.payments.is_empty());
        r["runtime_events"].as_array_mut().unwrap().extend([
            event("application_session", "new"),
            event("application_claim", "new"),
            event("application_payment", "new"),
        ]);
        let result = scope.project(&m, &r).unwrap();
        assert_eq!(result.payments.len(), 1);
        assert_eq!(result.payments[0]["session"], "new");
        assert_eq!(result.manifest.cases.len(), 1);
        assert_eq!(result.sessions, BTreeSet::from(["new".into()]));
    }
    #[test]
    fn missing_failed_duplicate_or_unknown_process_evidence_refuses_continuation() {
        for field in ["success", "valid_output", "forced_kill"] {
            let mut end = event("child_finished", "old");
            end["detail"][field] = json!(field == "forced_kill");
            assert!(
                Sessions::prior(
                    &json!({"runtime_events":[event("application_session","old"),end]})
                )
                .is_err()
            );
        }
        assert!(
            Sessions::prior(&json!({"runtime_events":[event("application_session","old")]}))
                .is_err()
        );
        assert!(
            Sessions::prior(&json!({"runtime_events":[event("child_finished","unknown")]}))
                .is_err()
        );
    }
    #[test]
    fn scope_cannot_hide_replays_or_claim_unregistered_sessions() {
        let m = crate::tests::manifest();
        let r = json!({"runtime_events":[event("application_session","old"),event("application_claim","old"),event("child_finished","old"),event("application_session","new"),event("application_claim","new")]});
        let selected = BTreeSet::from(["new".into()]);
        assert!(
            Sessions::including(&r, &selected)
                .unwrap()
                .project(&m, &r)
                .is_err()
        );
        assert!(Sessions::including(&r, &BTreeSet::from(["unknown".into()])).is_err());
        assert!(Sessions::including(&r, &BTreeSet::new()).is_err());
    }
    #[test]
    fn changed_binding_unknown_session_and_cross_session_payment_refuse_qualification() {
        let m = crate::tests::manifest();
        let mut r = json!({"runtime_events":[event("application_session","new"),event("application_claim","new"),event("application_payment","new")]});
        assert_eq!(
            Sessions::default().project(&m, &r).unwrap().payments.len(),
            1
        );
        r["runtime_events"][2]["detail"]["session"] = json!("unknown");
        assert!(Sessions::default().project(&m, &r).is_err());
        r["runtime_events"][2]["detail"]["session"] = json!("new");
        r["runtime_events"][1]["detail"]["source"] = json!("changed");
        assert!(Sessions::default().project(&m, &r).is_err());
    }
}
