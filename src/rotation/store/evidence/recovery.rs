//! Rebuildable chain projections never own authority. Original attempts do.
use super::*;

pub(in crate::rotation::store) fn initialize_history(db: &Connection) -> Result<()> {
    // Additive audit evidence, independent of encrypted record formats. Older
    // binaries still see the conservatively reopened original liabilities.
    db.execute_batch("CREATE TABLE IF NOT EXISTS payment_resolution_history(
        attempt_id TEXT NOT NULL REFERENCES payment_attempts(id),
        outcome TEXT NOT NULL,height INTEGER NOT NULL,hash TEXT NOT NULL,
        block_time INTEGER NOT NULL,support TEXT);
        CREATE INDEX IF NOT EXISTS resolution_history_attempt ON payment_resolution_history(attempt_id);")?;
    Ok(())
}

/// One atomic correction, with no new work queue. Existing pending attempts own
/// the next proof pass across cancellation and restart. Copy exact prior support
/// before removing current dispositions; never remove the accepted attempt.
pub(super) fn reopen_authorizations(tx: &Connection, wallet: &str) -> Result<()> {
    tx.execute("INSERT INTO payment_resolution_history(attempt_id,outcome,height,hash,block_time,support)
        SELECT r.attempt_id,r.outcome,r.height,r.hash,r.block_time,r.support
        FROM payment_resolutions r JOIN payment_attempts a ON a.id=r.attempt_id WHERE a.wallet_id=?1", [wallet])?;
    tx.execute("DELETE FROM payment_resolutions WHERE attempt_id IN (SELECT id FROM payment_attempts WHERE wallet_id=?1)", [wallet])?;
    tx.execute("UPDATE payment_attempts SET state='POSSIBLY_SUBMITTED' WHERE wallet_id=?1 AND state='RESOLVED'", [wallet])?;
    tx.execute("UPDATE wallets SET financial_height=NULL,financial_hash=NULL,debit_height=NULL,debit_hash=NULL,evidence_repair=NULL WHERE id=?1",[wallet])?;
    // Historical observations remain exact audit records, but no longer assert
    // current canonicality. In particular they cannot recreate a capture latch.
    tx.execute(
        "UPDATE wallet_retirements SET disputed='canonical_evidence_replaced' WHERE wallet_id=?1",
        [wallet],
    )?;
    tx.execute("UPDATE funding_jobs SET readiness_disputed='canonical_evidence_replaced' WHERE wallet_id=?1 AND readiness_support IS NOT NULL", [wallet])?;
    Ok(())
}

impl Store {
    /// Recover legacy repair latches and malformed derived checkpoint/debit
    /// projections. These fields are reconstructible from the original nonce
    /// journals; malformed original identities/amounts still fail at their owner.
    pub(super) fn recover_derived_evidence(&self, wallet: &str) -> Result<()> {
        let projection = (|| -> Result<bool> {
            let checkpoint = checkpoint(&self.db, wallet)?;
            let (height, hash, repair): (Option<i64>, Option<String>, Option<String>) =
                self.db.query_row(
                    "SELECT debit_height,debit_hash,evidence_repair FROM wallets WHERE id=?1",
                    [wallet],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
            let debit_valid = match (height, hash) {
                (None, None) => true,
                (Some(height), Some(hash)) => {
                    height >= 0
                        && hash.parse::<alloy_primitives::B256>().is_ok()
                        && checkpoint.as_ref().is_some_and(|c| {
                            c.height >= height as u64
                                && (c.height != height as u64 || c.hash == hash)
                        })
                }
                _ => false,
            };
            let missing_checkpoint = checkpoint.is_none() && self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM payment_attempts WHERE wallet_id=?1 AND state='RESOLVED')", [wallet], |r|r.get::<_,bool>(0))?;
            Ok(repair.is_some() || !debit_valid || missing_checkpoint)
        })();
        let reset = match projection {
            Ok(reset) => reset,
            Err(error) if reconstructible_projection(&error) => true,
            Err(error) => return Err(error),
        };
        if reset {
            // Store serializes all access, including captures. This transaction
            // needs no mutable Rust cache access: generation checks obsolete it.
            let tx = self.db.unchecked_transaction()?;
            reopen_authorizations(&tx, wallet)?;
            bump_generation(&tx, "wallet", wallet)?;
            tx.commit()?;
            tracing::info!(
                wallet,
                "Wallet chain records are being rebuilt from its original authorizations; payments are not replayed"
            );
        }
        Ok(())
    }
}

fn reconstructible_projection(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<EvidenceFailure>(),
        Some(EvidenceFailure::MalformedSupport)
    ) || matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(
            rusqlite::Error::InvalidColumnType(..)
                | rusqlite::Error::IntegralValueOutOfRange(..)
                | rusqlite::Error::FromSqlConversionFailure(..)
        )
    )
}
