use super::*;
use std::collections::BTreeSet;
#[allow(dead_code, reason = "M2 evidence boundary; supervisor wiring follows")]
#[derive(Clone, Copy)]
pub enum Outcome {
    ResponseSaved,
    TransportUncertain,
}
impl Registry {
    // Kept private to the supervisor modules; no CLI exposes synthetic reservations.
    // M3 must qualify pins/identity before using this durable pre-dispatch boundary.
    #[allow(
        dead_code,
        reason = "M2 durable boundary; execution wiring belongs to M3"
    )]
    pub fn reserve_batch(
        &mut self,
        run: &str,
        batch: &str,
        ids: &[String],
        now: i64,
    ) -> Result<()> {
        ensure!(
            identifier(batch) && !ids.is_empty(),
            "invalid/empty reservation batch"
        );
        let m = self.manifest(run)?;
        let p = m
            .phases
            .iter()
            .find(|p| p.cases.contains(&ids[0]))
            .context("unknown batch case")?;
        ensure!(
            ids.iter().all(|id| p.cases.contains(id)),
            "batch crosses phase boundary"
        );
        match &p.scenario {
            super::super::manifest::Scenario::Concurrency { batches } => ensure!(
                batches.contains(&ids.to_vec()),
                "batch does not match reviewed concurrency members/order"
            ),
            _ => ensure!(
                ids.len() == 1,
                "non-concurrent phase permits one case per batch"
            ),
        }
        ensure!(
            ids.len() <= m.limits.max_in_flight,
            "batch exceeds max_in_flight"
        );
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let authority: String = tx.query_row(
            "SELECT id FROM authorizations ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            authority == m.registry_authorization,
            "run authority has changed; no new dispatch allowed"
        );
        let ceiling: i64 = tx.query_row(
            "SELECT api FROM authorizations ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        for dependency in &p.depends_on {
            let blocked:i64=tx.query_row("SELECT COUNT(*) FROM cases WHERE run=?1 AND phase=?2 AND (execution!='COMPLETED' OR semantic!='PASSED' OR settlement NOT IN ('NOT_SIGNED','USED','EXPIRED_UNUSED'))",params![run,dependency],|r|r.get(0))?;
            ensure!(
                blocked == 0,
                "dependency {dependency} has incomplete/unqualified cases"
            );
        }
        let mut sum = 0i64;
        let mut unique = BTreeSet::new();
        for id in ids {
            ensure!(unique.insert(id), "duplicate case in reservation batch");
            let (cost,start,end,state):(i64,i64,i64,String)=tx.query_row("SELECT reservation,window_start,window_end,execution FROM cases WHERE run=?1 AND id=?2",params![run,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            ensure!(
                state == "UNATTEMPTED",
                "case {id} already reserved: observe only, never replay"
            );
            ensure!(
                now >= start && now < end,
                "case {id} outside absolute execution window"
            );
            sum = sum
                .checked_add(cost)
                .context("batch reservation overflow")?;
        }
        let (global,local):(i64,i64)=tx.query_row("SELECT COALESCE(SUM(reservation),0),COALESCE(SUM(CASE WHEN run=?1 THEN reservation ELSE 0 END),0) FROM cases WHERE execution!='UNATTEMPTED'",[run],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            global.checked_add(sum).is_some_and(|n| n <= ceiling)
                && local
                    .checked_add(sum)
                    .is_some_and(|n| n <= usdc(&m.limits.api_reservation_usdc).unwrap_or(0)),
            "cumulative/run API reservation ceiling exceeded"
        );
        tx.execute(
            "INSERT INTO batches VALUES(?1,?2,?3,?4)",
            params![run, batch, bounded_json(&ids)?, now],
        )?;
        for id in ids {
            tx.execute(
                "UPDATE cases SET execution='RESERVED' WHERE run=?1 AND id=?2",
                params![run, id],
            )?;
        }
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'batch_reserved',?2,?3)",
            params![
                run,
                bounded_json(&json!({"batch":batch,"cases":ids,"reservation":sum}))?,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    #[allow(
        dead_code,
        reason = "M2 durable boundary; execution wiring belongs to M3"
    )]
    pub fn dispatching(&mut self, run: &str, id: &str) -> Result<()> {
        ensure!(self.db.execute("UPDATE cases SET execution='DISPATCHING' WHERE run=?1 AND id=?2 AND execution='RESERVED'",params![run,id])?==1,"case must be durably reserved before dispatch");
        Ok(())
    }
    #[allow(
        dead_code,
        reason = "M2 durable boundary; execution wiring belongs to M3"
    )]
    pub fn finish(
        &mut self,
        run: &str,
        id: &str,
        outcome: Outcome,
        body: Option<&[u8]>,
    ) -> Result<()> {
        let execution = match outcome {
            Outcome::ResponseSaved => "RESPONSE_SAVED",
            Outcome::TransportUncertain => "TRANSPORT_UNCERTAIN",
        };
        ensure!(
            matches!(outcome, Outcome::ResponseSaved) == body.is_some(),
            "result evidence does not match outcome"
        );
        let m = self.manifest(run)?;
        if let Some(bytes) = body {
            ensure!(
                bytes.len() <= m.limits.result_bytes,
                "result exceeds configured result_bytes={}",
                m.limits.result_bytes
            );
            let path = self.root.join(format!(
                "response-{}.bin",
                files::hash(format!("{run}:{id}"))
            ));
            files::publish(&path, bytes)?;
        }
        let tx = self.db.transaction()?;
        ensure!(tx.execute("UPDATE cases SET execution=?3,result_hash=?4,settlement='PENDING' WHERE run=?1 AND id=?2 AND execution='DISPATCHING'",params![run,id,execution,body.map(files::hash)])?==1,"only dispatched cases can finish; result is immutable");
        // Settlement is deliberately unknown. A body/error cannot prove debit or unused authorization.
        tx.commit()?;
        Ok(())
    }
}

impl Registry {
    pub fn uncertain_unsigned(&mut self, run: &str, id: &str) -> Result<()> {
        let manifest = self.manifest(run)?;
        ensure!(
            manifest
                .cases
                .iter()
                .find(|c| c.id == id)
                .is_some_and(|c| c.unsigned),
            "unsigned result requires an unsigned case"
        );
        self.finish(run, id, Outcome::TransportUncertain, None)?;
        ensure!(self.db.execute("UPDATE cases SET settlement='NOT_SIGNED' WHERE run=?1 AND id=?2 AND execution='TRANSPORT_UNCERTAIN'", params![run,id])? == 1, "unsigned uncertainty state changed");
        Ok(())
    }
    pub fn finish_unsigned(
        &mut self,
        run: &str,
        id: &str,
        body: &[u8],
        passed: bool,
    ) -> Result<()> {
        let manifest = self.manifest(run)?;
        ensure!(
            manifest
                .cases
                .iter()
                .find(|c| c.id == id)
                .is_some_and(|c| c.unsigned),
            "unsigned result requires an unsigned case"
        );
        self.finish(run, id, Outcome::ResponseSaved, Some(body))?;
        // The keyless executable contract establishes NOT_SIGNED, not the seller body.
        ensure!(self.db.execute("UPDATE cases SET execution='COMPLETED',semantic=?3,settlement='NOT_SIGNED' WHERE run=?1 AND id=?2 AND execution='RESPONSE_SAVED'",
            params![run,id,if passed { "PASSED" } else { "FAILED" }])? == 1, "unsigned completion state changed");
        Ok(())
    }
}
