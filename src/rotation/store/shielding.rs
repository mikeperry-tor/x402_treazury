//! Immutable shielding provenance; the ordinary outgoing journal owns its lifecycle.
use super::*;

#[derive(Clone)]
pub(crate) struct Policy {
    pool: String,
    wallet: String,
    address: String,
    max_fee: u64,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Input {
    pub txid: String,
    pub output_index: u32,
    pub amount: u64,
    pub script: Vec<u8>,
}
#[derive(Clone)]
pub(crate) struct Binding {
    policy: Policy,
    inputs: Vec<Input>,
    fee: u64,
}
impl Binding {
    pub(crate) fn new(policy: Policy, mut inputs: Vec<Input>, fee: u64) -> Result<Self> {
        ensure!(
            !inputs.is_empty() && fee > 0 && fee <= policy.max_fee,
            "shielding fee/input mismatch"
        );
        let script = inputs[0].script.clone();
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for input in &mut inputs {
            let txid = input
                .txid
                .parse::<alloy_primitives::B256>()
                .map_err(|_| anyhow::anyhow!("shielding input identity malformed"))?;
            input.txid = txid.to_string().trim_start_matches("0x").to_owned();
            ensure!(
                input.amount > 0
                    && !input.script.is_empty()
                    && input.script == script
                    && seen.insert((input.txid.clone(), input.output_index)),
                "shielding input mismatch"
            );
            total = total
                .checked_add(input.amount)
                .context("shielding input value overflow")?;
        }
        ensure!(
            total > fee,
            "shielding inputs do not cover internal output and fee"
        );
        Ok(Self {
            policy,
            inputs,
            fee,
        })
    }
    #[cfg(feature = "zcash")]
    pub(crate) fn check_transaction(&self, raw: &[u8], facts: &TransactionFacts) -> Result<()> {
        let transaction = crate::treasury::submission::decode(raw)?;
        ensure!(
            transaction.txid().to_string() == facts.txid
                && u32::from(transaction.expiry_height()) == facts.expiry_height,
            "shielding transaction facts mismatch"
        );
        let bundle = transaction
            .transparent_bundle()
            .context("shielding inputs missing")?;
        ensure!(
            bundle.vout.is_empty() && bundle.vin.len() == self.inputs.len(),
            "shielding raw input/output mismatch"
        );
        let mut actual = std::collections::BTreeSet::new();
        for input in &bundle.vin {
            let id = (
                zcash_primitives::transaction::TxId::from_bytes(*input.prevout().hash())
                    .to_string(),
                input.prevout().n(),
            );
            ensure!(actual.insert(id), "duplicate shielding raw input");
        }
        let selected = self
            .inputs
            .iter()
            .map(|i| (i.txid.clone(), i.output_index))
            .collect();
        ensure!(actual == selected, "shielding raw input mismatch");
        let input = self
            .inputs
            .iter()
            .try_fold(0u64, |sum, i| sum.checked_add(i.amount))
            .context("shielding input value overflow")?;
        let shielded = [
            transaction
                .sapling_bundle()
                .map(|b| i64::from(*b.value_balance())),
            transaction
                .orchard_bundle()
                .map(|b| i64::from(*b.value_balance())),
            transaction
                .ironwood_bundle()
                .map(|b| i64::from(*b.value_balance())),
        ]
        .into_iter()
        .flatten()
        .try_fold(0i64, |sum, v| sum.checked_add(v))
        .context("shielding value overflow")?;
        ensure!(
            i64::try_from(input)?.checked_add(shielded) == Some(i64::try_from(facts.fee_zatoshis)?),
            "shielding raw fee mismatch"
        );
        Ok(())
    }
    pub(super) fn check(
        &self,
        db: &Connection,
        operation: &str,
        facts: &TransactionFacts,
    ) -> Result<()> {
        ensure!(
            facts.amount_zatoshis == 0
                && facts.fee_zatoshis == self.fee
                && facts.fee_zatoshis <= self.policy.max_fee,
            "shielding cost mismatch"
        );
        let bound: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM wallets WHERE id=?1 AND pool_id=?2)",
            params![self.policy.wallet, self.policy.pool],
            |r| r.get(0),
        )?;
        ensure!(bound, "shielding policy binding changed");
        self.check_claims(db, operation)
    }
    pub(crate) fn check_claims(&self, db: &Connection, operation: &str) -> Result<()> {
        for input in &self.inputs {
            let claimed: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM shielding_inputs i WHERE i.txid=?1 AND i.output_index=?2 AND i.operation_id!=?3 AND NOT EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=i.operation_id))",params![input.txid,input.output_index,operation],|r|r.get(0))?;
            // Confirmation does not release an outpoint claim: a reorg can make
            // an old input unspent while its original signed bytes remain live.
            ensure!(!claimed, "shielding input already owned");
        }
        Ok(())
    }
    pub(super) fn insert(
        &self,
        tx: &Connection,
        key: &[u8; 32],
        treasury: &str,
        network: &str,
        operation: &str,
    ) -> Result<()> {
        let address = seal(
            key,
            &format!("v1:{treasury}:{network}:shielding:{operation}"),
            self.policy.address.as_bytes(),
        )?;
        tx.execute("INSERT INTO shielding_operations(operation_id,policy_pool_id,policy_wallet_id,address,max_fee,automatic) VALUES(?1,?2,?3,?4,?5,0)",params![operation,self.policy.pool,self.policy.wallet,address,i64::try_from(self.policy.max_fee)?])?;
        for input in &self.inputs {
            tx.execute("INSERT INTO shielding_inputs(operation_id,txid,output_index,amount,script) VALUES(?1,?2,?3,?4,?5)",params![operation,input.txid,input.output_index,i64::try_from(input.amount)?,input.script])?;
        }
        Ok(())
    }
    pub(super) fn check_saved(
        &self,
        db: &Connection,
        key: &[u8; 32],
        treasury: &str,
        network: &str,
        operation: &str,
    ) -> Result<()> {
        let (pool,wallet,address,max_fee,automatic):(String,String,Vec<u8>,i64,bool)=db.query_row("SELECT policy_pool_id,policy_wallet_id,address,max_fee,automatic FROM shielding_operations WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            pool == self.policy.pool
                && wallet == self.policy.wallet
                && max_fee == i64::try_from(self.policy.max_fee)?
                && !automatic
                && unseal(
                    key,
                    &format!("v1:{treasury}:{network}:shielding:{operation}"),
                    &address
                )?
                .as_slice()
                    == self.policy.address.as_bytes(),
            "shielding operation conflict"
        );
        let saved=db.prepare("SELECT txid,output_index,amount,script FROM shielding_inputs WHERE operation_id=?1 ORDER BY txid,output_index")?.query_map([operation],|r|Ok(Input {txid:r.get(0)?,output_index:r.get(1)?,amount:sql_u64(r,2)?,script:r.get(3)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut expected = self.inputs.clone();
        expected.sort_by(|a, b| (&a.txid, a.output_index).cmp(&(&b.txid, b.output_index)));
        ensure!(saved == expected, "shielding input conflict");
        Ok(())
    }
}
impl Store {
    /// Reconstruct the immutable internal transfer contract before granting
    /// expiry-only submission. An absent deadline or zero amount alone is never authority.
    pub(crate) fn validated_internal_shielding(
        &self,
        operation: &str,
        facts: &TransactionFacts,
    ) -> Result<bool> {
        let header: Option<(String, String, Vec<u8>, i64)> = self.db.query_row(
            "SELECT policy_pool_id,policy_wallet_id,address,max_fee FROM shielding_operations WHERE operation_id=?1",
            [operation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        ).optional()?;
        let Some((pool, wallet, address, max_fee)) = header else {
            return Ok(false);
        };
        let address = unseal(
            &self.key,
            &format!(
                "v1:{}:{}:shielding:{operation}",
                self.id,
                self.network.name()
            ),
            &address,
        )?;
        let policy = Policy {
            pool,
            wallet,
            address: std::str::from_utf8(&address)?.into(),
            max_fee: u64::try_from(max_fee)?,
        };
        let inputs = self.db.prepare("SELECT txid,output_index,amount,script FROM shielding_inputs WHERE operation_id=?1")?
            .query_map([operation], |r| Ok(Input { txid: r.get(0)?, output_index: r.get(1)?, amount: sql_u64(r,2)?, script: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let binding = Binding::new(policy, inputs, facts.fee_zatoshis)?;
        binding.check(&self.db, operation, facts)?;
        #[cfg(feature = "zcash")]
        binding.check_transaction(&self.prepared_bytes(operation)?, facts)?;
        Ok(true)
    }
    /// Explicit CLI preparation only. Automatic policy is a separate capability;
    /// neither generic sync nor a positive source-funding permit grants it.
    pub(crate) fn begin_manual_shielding(
        &mut self,
        operation: &str,
        revision: i64,
        job: &str,
        address: &str,
        max_fee: u64,
    ) -> Result<(preparation::Preparation, Policy)> {
        ensure!(
            self.funding_permits.is_none(),
            AdmissionError::FundingPermitDenied
        );
        self.funding_restriction
            .require_new_funding("shielding_preparation")?;
        ensure!(
            max_fee > 0 && i64::try_from(max_fee).is_ok(),
            "invalid shielding fee cap"
        );
        ensure!(
            self.refund_address(job)?.as_deref() == Some(address),
            "shielding address binding changed"
        );
        let (pool, wallet): (String, String) = self.db.query_row(
            "SELECT pool_id,wallet_id FROM funding_jobs WHERE id=?1",
            [job],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let pending: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
            [],
            |r| r.get(0),
        )?;
        ensure!(!pending, "treasury_send_pending");
        let owner = self.begin_owned_preparation(operation, revision, None)?;
        Ok((
            owner,
            Policy {
                pool,
                wallet,
                address: address.into(),
                max_fee,
            },
        ))
    }
    pub(crate) fn check_shielding_inputs(
        &self,
        owner: &preparation::Preparation,
        binding: &Binding,
    ) -> Result<()> {
        self.require_owned_reservation(owner)?;
        binding.check_claims(&self.db, owner.operation())
    }
    pub(crate) fn prepare_owned_shielding(
        &mut self,
        owner: &preparation::Preparation,
        snapshot: &[u8],
        raw: &[u8],
        facts: TransactionFacts,
        binding: &Binding,
    ) -> Result<i64> {
        self.prepare_bound(
            owner.operation(),
            owner.revision(),
            snapshot,
            raw,
            Some(facts),
            Some((owner, Some(binding))),
        )
    }
}

#[cfg(test)]
mod tests;
