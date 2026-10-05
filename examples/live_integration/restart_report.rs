//! Authenticate the supervisor's restart transcript against registered sessions,
//! process completion, round baselines and durable funding continuity.
use crate::manifest::{Manifest, Phase, Restart, Scenario};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use x402_treazury::qualification::{
    Binding,
    lifecycle::restart::{self, Checkpoint},
};

fn unique<'a>(
    events: &'a [Value],
    kind: &str,
    field: &str,
    value: &str,
) -> Result<Option<(usize, &'a Value)>> {
    let mut found = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e["kind"] == kind && e["detail"][field] == value);
    let result = found.next().map(|(i, e)| (i, &e["detail"]));
    ensure!(found.next().is_none(), "duplicate restart evidence: {kind}");
    Ok(result)
}
pub(crate) fn report(events: &[Value], manifest: &Manifest, phase: &Phase) -> Result<Value> {
    let Scenario::Lifecycle {
        restart,
        ref rounds,
        ..
    } = phase.scenario
    else {
        anyhow::bail!("restart report requires lifecycle")
    };
    let Some((requested_at, requested)) = unique(events, "restart_requested", "phase", &phase.id)?
    else {
        return Ok(json!({"status":"incomplete","reason":"restart checkpoint not observed"}));
    };
    let Some((drained_at, drained)) = unique(events, "restart_drained", "phase", &phase.id)? else {
        return Ok(json!({"status":"incomplete","reason":"graceful drain not verified"}));
    };
    let Some((completed_at, completed)) = unique(events, "restart_completed", "phase", &phase.id)?
    else {
        return Ok(json!({"status":"incomplete","reason":"application reopening not verified"}));
    };
    for detail in [requested, drained, completed] {
        ensure!(detail["round"] == 0, "restart belongs to another round");
    }
    ensure!(
        requested["checkpoint"] == serde_json::to_value(restart)?,
        "restart checkpoint differs from manifest"
    );
    let previous: Binding = serde_json::from_value(requested["binding"].clone())?;
    let next: Binding = serde_json::from_value(completed["binding"].clone())?;
    ensure!(
        previous.version == 1
            && next.version == 1
            && previous.run == manifest.run_id
            && next.run == previous.run
            && next.session != previous.session
            && !previous.session.is_empty()
            && !next.session.is_empty()
            && previous.registry == next.registry
            && previous.pin_digest == next.pin_digest
            && previous.config_sha256 == next.config_sha256
            && previous.expires_at == next.expires_at,
        "restart changed authority/configuration or reused session identity"
    );
    ensure!(
        drained["binding"] == requested["binding"]
            && completed["previous_binding"] == requested["binding"],
        "restart transcript previous-session mismatch"
    );
    let (old_at, old) = unique(events, "application_session", "session", &previous.session)?
        .context("old session is not registered")?;
    let (new_at, new) = unique(events, "application_session", "session", &next.session)?
        .context("new session is not registered")?;
    ensure!(
        *old == requested["binding"] && *new == completed["binding"],
        "restart binding differs from registered session"
    );
    let (closed_at, closed) = unique(events, "child_finished", "session", &previous.session)?
        .context("restart lacks owned-child completion")?;
    ensure!(
        closed["success"] == true
            && closed["exit_code"] == 0
            && closed["forced_kill"] == false
            && closed["valid_output"] == true
            && closed["reason"] == "lifecycle_restart"
            && closed["pid"].as_u64().is_some_and(|pid| pid > 0)
            && drained["closed"]["session"] == previous.session
            && drained["closed"]["complete"] == true
            && drained["closed"]["process"] == *closed,
        "restart did not cleanly drain its owned child"
    );
    ensure!(
        old_at < requested_at
            && requested_at < closed_at
            && closed_at < drained_at
            && drained_at < new_at
            && new_at < completed_at,
        "restart session/process events are out of order"
    );
    let pool = requested["pool"].as_str().context("restart pool missing")?;
    let job = requested["job"].as_str().context("restart job missing")?;
    let selected = requested["state"]["pools"]
        .as_array()
        .context("restart pools missing")?
        .iter()
        .find(|p| p["id"] == pool)
        .context("restart selected pool missing")?;
    let mut promotions = events.iter().filter(|e| {
        e["kind"] == "rotation_promotion"
            && e["detail"]["phase"] == phase.id
            && e["detail"]["round"] == 0
    });
    let promotion = &promotions
        .next()
        .context("restart first promotion missing")?["detail"];
    ensure!(
        promotions.next().is_none(),
        "duplicate restart first promotion"
    );
    let promoted = promotion["after"]["pools"]
        .as_array()
        .context("promoted pools missing")?
        .iter()
        .find(|p| p["id"] == pool)
        .context("promoted pool missing")?;
    ensure!(
        rounds.len() == 2
            && selected["name"] == rounds[0].pool
            && requested["state"]["treasury_id"] == manifest.treasury_id
            && promotion["replacement_job"] == job
            && promoted["generation"] == selected["generation"]
            && x402_treazury::qualification::lifecycle::promotion(
                &promotion["before"],
                &promotion["after"],
                pool
            )? == job,
        "restart does not belong to the first verified replacement"
    );
    let checkpoint = match restart {
        Restart::QueuedRefill => Checkpoint::QueuedRefill,
        Restart::SubmittedDeposit => Checkpoint::SubmittedDeposit,
    };
    let continuity = restart::continuity(
        &requested["state"],
        &drained["state"],
        &completed["state"],
        pool,
        job,
        checkpoint,
    )?;
    ensure!(
        completed["continuity"] == continuity,
        "recorded restart continuity differs from durable evidence"
    );
    for (index, expected_session) in [(0, &previous.session), (1, &next.session)] {
        let mut starts = events.iter().enumerate().filter(|(_, e)| {
            e["kind"] == "rotation_started"
                && e["detail"]["phase"] == phase.id
                && e["detail"]["round"] == index
        });
        let Some((at, start)) = starts.next() else {
            return Ok(
                json!({"status":"incomplete","reason":"rotation round lacks a session baseline"}),
            );
        };
        ensure!(starts.next().is_none(), "duplicate restart round baseline");
        let request = start["detail"]["request"]
            .as_str()
            .context("round observation request missing")?;
        let (_, observation) = unique(events, "application_pool_observation", "request", request)?
            .context("restart round observation missing")?;
        ensure!(
            observation["session"] == *expected_session,
            "rotation used another restart session"
        );
        ensure!(
            if index == 0 {
                at < requested_at
            } else {
                at > completed_at
            },
            "rotation baseline lies outside its restart interval"
        );
    }
    Ok(
        json!({"status":"passed","checkpoint":restart,"from_session":previous.session,"to_session":next.session,"continuity":continuity}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Phase, Vec<Value>) {
        let mut m = crate::tests::manifest();
        m.treasury_id = "t".into();
        let mut phase = m.phases[0].clone();
        phase.id = "lifecycle".into();
        phase.scenario = Scenario::Lifecycle {
            refill_slots: 2,
            restart: Restart::SubmittedDeposit,
            rounds: (0..2)
                .map(|i| crate::manifest::RotationRound {
                    pool: "pool".into(),
                    expected_price_usdc: "0.01".into(),
                    depletion_cases: vec![format!("d{i}")],
                    service_cases: vec![format!("s{i}")],
                })
                .collect(),
        };
        let wallet = |id: &str, n: u64, role: &str| json!({"id":id,"address":format!("0x{n:040x}"),"role":role,"target":"100"});
        let before = json!({"treasury_id":"t","pools":[{"id":"p","name":"pool","generation":0,
            "addresses":[wallet("a",1,"ACTIVE"),wallet("b",2,"READY")]}],"funding_jobs":[],"source_operations":[]});
        let state = json!({"treasury_id":"t","pools":[{"id":"p","name":"pool","generation":1,
            "addresses":[wallet("a",1,"RETIRED"),wallet("b",2,"ACTIVE"),wallet("c",3,"ALLOCATED")]}],
            "funding_jobs":[{"id":"j","pool_id":"p","wallet_id":"c","recipient":format!("0x{:040x}",3),"target":"100","operation_id":"op","phase":"DEPOSIT_PENDING"}],
            "source_operations":[{"operation_id":"op","submission":"BROADCAST","attempts":1}]});
        let binding = |session: &str| {
            json!({"version":1,"run":m.run_id,"session":session,"registry":"registry.sqlite",
            "pin_digest":"pins","config_sha256":"config","expires_at":1000})
        };
        let old = binding("old");
        let new = binding("new");
        let process = json!({"session":"old","pid":123,"success":true,"exit_code":0,"forced_kill":false,"valid_output":true,"reason":"lifecycle_restart"});
        let continuity = restart::continuity(
            &state,
            &state,
            &state,
            "p",
            "j",
            Checkpoint::SubmittedDeposit,
        )
        .unwrap();
        let event = |kind: &str, detail: Value| json!({"kind":kind,"detail":detail});
        let events = vec![
            event("application_session", old.clone()),
            event(
                "application_pool_observation",
                json!({"request":"q0","session":"old"}),
            ),
            event(
                "rotation_started",
                json!({"phase":"lifecycle","round":0,"request":"q0"}),
            ),
            event(
                "rotation_promotion",
                json!({"phase":"lifecycle","round":0,"before":before,"after":state,"replacement_job":"j"}),
            ),
            event(
                "restart_requested",
                json!({"phase":"lifecycle","round":0,"pool":"p","job":"j","checkpoint":"submitted_deposit","binding":old,"state":state}),
            ),
            event("child_finished", process.clone()),
            event(
                "restart_drained",
                json!({"phase":"lifecycle","round":0,"binding":old,"closed":{"session":"old","complete":true,"process":process},"state":state}),
            ),
            event("application_session", new.clone()),
            event(
                "restart_completed",
                json!({"phase":"lifecycle","round":0,"previous_binding":old,"binding":new,"state":state,"continuity":continuity}),
            ),
            event(
                "application_pool_observation",
                json!({"request":"q1","session":"new"}),
            ),
            event(
                "rotation_started",
                json!({"phase":"lifecycle","round":1,"request":"q1"}),
            ),
        ];
        (m, phase, events)
    }
    #[test]
    fn authenticated_ordered_sessions_and_clean_drain_prove_restart_continuity() {
        let (m, phase, events) = fixture();
        assert_eq!(report(&events, &m, &phase).unwrap()["status"], "passed");
        for len in [0, 5, 8, 9] {
            assert_eq!(
                report(&events[..len], &m, &phase).unwrap()["status"],
                "incomplete"
            );
        }
    }
    #[test]
    fn restart_rejects_extended_authority_forced_exit_mixed_baselines_and_extra_submissions() {
        let (m, phase, events) = fixture();
        for (index, pointer, value) in [
            (8, "/detail/binding/expires_at", json!(1001)),
            (8, "/detail/binding/session", json!("old")),
            (8, "/detail/binding/pin_digest", json!("changed")),
            (5, "/detail/forced_kill", json!(true)),
            (5, "/detail/valid_output", json!(false)),
            (9, "/detail/session", json!("old")),
            (8, "/detail/state/source_operations/0/attempts", json!(2)),
            (
                8,
                "/detail/state/funding_jobs/0/operation_id",
                json!("different"),
            ),
            (4, "/detail/checkpoint", json!("queued_refill")),
            (4, "/detail/pool", json!("other")),
        ] {
            let mut wrong = events.clone();
            *wrong[index].pointer_mut(pointer).unwrap() = value;
            assert!(report(&wrong, &m, &phase).is_err(), "{pointer}");
        }
        let mut wrong = events.clone();
        wrong.swap(5, 7);
        assert!(report(&wrong, &m, &phase).is_err());
        let mut wrong = events.clone();
        wrong.push(events[8].clone());
        assert!(report(&wrong, &m, &phase).is_err());
        let mut wrong = events.clone();
        wrong.remove(7);
        assert!(report(&wrong, &m, &phase).is_err());
    }
}
