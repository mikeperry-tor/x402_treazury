//! Source/session-scoped startup observations; they grant no HTTP/payment authority.
use super::*;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum PricingStage {
    Started,
    Completed {
        evidence: crate::pricing::Evidence,
        prices: Vec<crate::pricing::Price>,
    },
    Failed {
        failure: FailureCategory,
    },
}
pub async fn record(source: &str, stage: PricingStage) -> Result<()> {
    let Some(guard) = ACTIVE.get().cloned() else {
        return Ok(());
    };
    record_with(guard, source.to_owned(), stage).await
}
async fn record_with(guard: Arc<Guard>, source: String, stage: PricingStage) -> Result<()> {
    ensure!(identifier(&source), "invalid pricing observation source");
    if let PricingStage::Completed { evidence, prices } = &stage {
        evidence.validate()?;
        ensure!(
            crate::pricing::price_map(prices)?.len() == evidence.available_prices,
            "pricing lines differ from available-price count"
        );
    }
    tokio::task::spawn_blocking(move || {
        let mut db=connection(&guard.binding)?;
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw:String=tx.query_row("SELECT payload FROM pins WHERE run=?1 AND digest=?2 ORDER BY revision DESC LIMIT 1",params![guard.binding.run,guard.binding.pin_digest],|r|r.get(0))?;
        ensure!(format!("{:x}",Sha256::digest(raw.as_bytes()))==guard.binding.pin_digest,"pricing observation pin hash changed");
        let registered:String=tx.query_row("SELECT detail FROM events WHERE run=?1 AND kind='application_session' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2",params![guard.binding.run,guard.binding.session],|r|r.get(0))?;
        ensure!(bounded(registered)?==serde_json::to_value(&guard.binding)?,"pricing observation session binding changed");
        let pinned=bounded(raw)?;
        ensure!(pinned["resolved_config"]["sources"].as_object().is_some_and(|s|s.contains_key(&source)),"pricing observation source is not pinned");
        if let PricingStage::Completed{evidence,..}=&stage {
            ensure!(pinned["resolved_config"]["sources"][&source]["settings"]["probe_pricing"].as_bool()==Some(evidence.enabled),"pricing enabled state differs from pinned source");
        }
        let count:i64=tx.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='application_pricing' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.source')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?3",
            params![guard.binding.run,source,guard.binding.session],|r|r.get(0))?;
        ensure!(count==if matches!(stage,PricingStage::Started) {0}else{1},"pricing stage duplicated/out of order; at most one start and completion per source/session");
        let mut event=serde_json::to_value(stage)?;
        event["source"]=json!(source);event["session"]=json!(guard.binding.session);event["observed_micros"]=json!(guard.origin.elapsed().as_micros());
        let raw=encode(&event)?;
        tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_pricing',?2,?3)",params![guard.binding.run,raw,now()?])?;
        tx.commit()?;Ok(())
    }).await.context("pricing evidence worker stopped")?
}

fn encode(event: &Value) -> Result<String> {
    let raw = event.to_string();
    if raw.len() > LIMIT {
        tracing::warn!(
            limit_bytes = LIMIT,
            "pricing evidence exceeds private event limit; refusing incomplete publication"
        );
        anyhow::bail!(
            "pricing evidence exceeds {LIMIT}-byte limit; split qualification sources, no partial evidence"
        );
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_bound_has_explicit_error_and_log_without_private_text() {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let error = encode(&json!({"private":"x".repeat(LIMIT)})).unwrap_err();
        assert!(error.to_string().contains(&format!("{LIMIT}-byte limit")));
        assert!(error.to_string().contains("no partial evidence"));
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("limit_bytes=16777216"));
        assert!(!text.contains("xxxxxxxx"));
    }
    #[tokio::test]
    async fn pricing_evidence_is_pinned_session_scoped_and_single_completion() {
        let (_dir, mut guard, db) = crate::qualification::tests::fixture();
        let raw: String = db
            .query_row("SELECT payload FROM pins", [], |r| r.get(0))
            .unwrap();
        let mut pins: Value = serde_json::from_str(&raw).unwrap();
        pins["resolved_config"] = json!({"sources":{"api":{"settings":{"probe_pricing":false}}}});
        let digest = format!("{:x}", Sha256::digest(pins.to_string().as_bytes()));
        Arc::get_mut(&mut guard).unwrap().binding.pin_digest = digest.clone();
        db.execute(
            "UPDATE pins SET payload=?1,digest=?2",
            params![pins.to_string(), digest],
        )
        .unwrap();
        db.execute(
            "UPDATE events SET detail=?1 WHERE kind='application_session'",
            [serde_json::to_string(&guard.binding).unwrap()],
        )
        .unwrap();
        assert!(
            record_with(guard.clone(), "unknown".into(), PricingStage::Started)
                .await
                .is_err()
        );
        assert!(
            record_with(
                guard.clone(),
                "api".into(),
                PricingStage::Failed {
                    failure: FailureCategory::Other
                }
            )
            .await
            .is_err()
        );
        record_with(guard.clone(), "api".into(), PricingStage::Started)
            .await
            .unwrap();
        assert!(
            record_with(guard.clone(), "api".into(), PricingStage::Started)
                .await
                .is_err()
        );
        record_with(
            guard.clone(),
            "api".into(),
            PricingStage::Completed {
                evidence: Default::default(),
                prices: Vec::new(),
            },
        )
        .await
        .unwrap();
        assert!(
            record_with(
                guard,
                "api".into(),
                PricingStage::Failed {
                    failure: FailureCategory::Other
                }
            )
            .await
            .is_err()
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_pricing'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }
}
