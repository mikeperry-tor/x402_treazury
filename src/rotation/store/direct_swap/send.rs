//! Accepted direct payouts reuse the treasury's input, byte and recovery journals.
use super::*;
use crate::rotation::transaction::PrepareRequest;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectSendAuthority {
    pub max_fee_zatoshis: u64,
    pub max_debit_zatoshis: Option<u64>,
}
#[derive(Clone)]
pub struct DirectSendBinding {
    pub recipient: String,
    pub principal: u64,
    pub deadline: Option<u64>,
    pub authority: DirectSendAuthority,
}
pub enum DirectSourceCondition {
    CostLimit,
    InsufficientFunds,
    Unavailable,
}
impl DirectSourceCondition {
    fn message(self) -> &'static str {
        match self {
            Self::CostLimit => {
                "Transaction cost exceeds accepted total debit or existing network fee limit"
            }
            Self::InsufficientFunds => "Waiting for spendable treasury funds",
            Self::Unavailable => {
                "Source work is waiting for treasury availability; original accepted terms remain"
            }
        }
    }
}
pub enum DirectSendAction {
    Prepare(PrepareRequest),
    Submit,
    NoAutomaticAction,
}

impl Store {
    fn optional_direct(&self, id: &str) -> Result<Option<DirectOperation>> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM direct_operations WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        exists.then(|| self.direct_by_id(id)).transpose()
    }
    pub fn accept_direct_send(
        &mut self,
        id: &str,
        quote: Option<Value>,
        authority: DirectSendAuthority,
    ) -> Result<DirectOperation> {
        let mut record = self.direct_by_id(id)?;
        ensure!(record.kind == "send", "operation is not a direct send");
        if record.authority.is_some() {
            return Ok(record);
        }
        self.require_direct_spending_policy()?;
        ensure!(
            record.quote.is_none(),
            "direct send quote acceptance is immutable"
        );
        record.quote = quote;
        record.authority = Some(authority);
        if record.route == DirectRoute::Near {
            let quote: crate::direct_swap::route::DirectQuote = serde_json::from_value(
                record
                    .quote
                    .clone()
                    .context("direct source quote missing")?,
            )?;
            let validated = crate::direct_swap::route::validate_quote_on(
                &self.network.chain(),
                quote.request,
                quote.response,
                crate::rotation::base::now()?,
            )?;
            record.quote = Some(serde_json::to_value(validated)?);
        }
        let binding = self.binding_for_record(&record)?;
        ensure!(
            binding
                .authority
                .max_debit_zatoshis
                .is_none_or(|cap| binding.principal <= cap),
            "direct source principal exceeds accepted debit authority"
        );
        record.state = "accepted".into();
        self.save_direct(&record)?;
        self.direct_changes.notify_one();
        self.treasury_changes.notify_one();
        Ok(record)
    }
    fn binding_for_record(&self, record: &DirectOperation) -> Result<DirectSendBinding> {
        ensure!(record.kind == "send", "operation is not a direct send");
        let authority = record
            .authority
            .clone()
            .context("direct send authority is not accepted")?;
        let destination = record
            .terms
            .get("recipient")
            .and_then(Value::as_str)
            .context("direct destination is missing")?;
        let (recipient, principal, deadline) = match record.route {
            DirectRoute::Native => {
                ensure!(
                    record.quote.is_none(),
                    "native send cannot use a provider quote"
                );
                ensure!(
                    record.terms["asset_id"] == crate::direct_swap::route::ZEC
                        && record.terms["chain"] == "zec",
                    "native send asset binding mismatch"
                );
                crate::treasury::native_recipient(&self.network.chain(), destination)?;
                (destination.to_owned(), native_amount(&record.terms)?, None)
            }
            DirectRoute::Near => {
                let quote: crate::direct_swap::route::DirectQuote = serde_json::from_value(
                    record
                        .quote
                        .clone()
                        .context("direct source quote missing")?,
                )?;
                ensure!(
                    quote.request["dry"] == false
                        && quote.request["swapType"] == "EXACT_OUTPUT"
                        && quote.request["originAsset"] == crate::direct_swap::route::ZEC
                        && quote.request["destinationAsset"] == record.terms["asset_id"]
                        && quote.request["recipient"] == destination
                        && quote.request["amount"] == record.terms["amount"]
                        && quote.request["refundTo"].as_str() == record.address.as_deref(),
                    "direct quote destination or refund binding mismatch"
                );
                let principal = quote
                    .input
                    .parse::<u64>()
                    .context("direct source exceeds SDK monetary range")?;
                zcash_protocol::value::Zatoshis::from_u64(principal)?;
                ensure!(principal > 0, "direct source principal must be positive");
                let deposit = quote.deposit.context("direct source deposit missing")?;
                ensure!(
                    matches!(
                        zcash_keys::address::Address::decode(&self.network.chain(), &deposit),
                        Some(zcash_keys::address::Address::Transparent(_))
                    ),
                    "direct source requires a transparent ZEC deposit"
                );
                ensure!(
                    quote.deposit_memo.is_none(),
                    "ZEC source deposit memo is unsupported"
                );
                (deposit, principal, Some(quote.deadline))
            }
        };
        if let Some(maximum) = record
            .terms
            .get("max_source_zatoshis")
            .filter(|value| !value.is_null())
        {
            let maximum: u64 = maximum
                .as_str()
                .context("invalid caller source authority")?
                .parse()
                .context("invalid caller source authority")?;
            ensure!(
                principal <= maximum,
                "direct source principal exceeds caller authority"
            );
        } else {
            ensure!(
                record.route == DirectRoute::Native,
                "foreign sends require explicit caller source authority"
            );
        }
        Ok(DirectSendBinding {
            recipient,
            principal,
            deadline,
            authority,
        })
    }
    pub fn direct_send_binding(&self, id: &str) -> Result<Option<DirectSendBinding>> {
        self.optional_direct(id)?
            .map(|record| self.binding_for_record(&record))
            .transpose()
    }
    pub fn direct_operation_identity(
        &self,
        id: &str,
    ) -> Result<Option<crate::network::IsolationId>> {
        Ok(self
            .optional_direct(id)?
            .map(|record| crate::network::IsolationId::direct_swap(&self.id, &record.id)))
    }
    fn require_direct_spending_policy(&self) -> Result<()> {
        ensure!(
            self.funding_permits.is_none(),
            "qualification_funding_denied: direct sends are not authorized by funding permits"
        );
        if self.funding_restriction
            == super::super::super::restriction::FundingRestriction::DenyNewFunding
        {
            self.funding_restriction
                .require_new_funding("direct_source_preparation")?;
        }
        Ok(())
    }
    pub(crate) fn require_operation_source_policy(
        &self,
        id: &str,
        action: &'static str,
    ) -> Result<()> {
        match self.optional_direct(id)? {
            Some(record) if record.kind == "send" && record.authority.is_some() => {
                self.binding_for_record(&record)?;
                self.require_direct_spending_policy()
            }
            _ => self.funding_restriction.require_new_funding(action),
        }
    }
    pub(crate) fn require_operation_preparation_available(&self, id: &str) -> Result<()> {
        self.require_operation_source_policy(id, "treasury_preparation")?;
        ensure!(!self.preparation_pending()?, "treasury_send_pending");
        Ok(())
    }
    pub(crate) fn require_direct_source_cost(
        &self,
        id: &str,
        principal: u64,
        fee: u64,
    ) -> Result<()> {
        let Some(binding) = self.direct_send_binding(id)? else {
            return Ok(());
        };
        ensure!(
            principal == binding.principal,
            "direct source principal binding mismatch"
        );
        ensure!(
            fee <= binding.authority.max_fee_zatoshis,
            "direct source fee exceeds accepted authority"
        );
        let cost = principal
            .checked_add(fee)
            .context("direct source cost overflow")?;
        ensure!(
            binding
                .authority
                .max_debit_zatoshis
                .is_none_or(|maximum| cost <= maximum),
            "direct source cost exceeds accepted debit authority"
        );
        Ok(())
    }
    pub(crate) fn require_direct_preparation_binding(
        &self,
        id: &str,
        pool: Option<&str>,
        recipient: &str,
        principal: u64,
        deadline: Option<u64>,
    ) -> Result<bool> {
        let Some(record) = self.optional_direct(id)? else {
            return Ok(false);
        };
        let binding = self.binding_for_record(&record)?;
        ensure!(
            pool.is_none()
                && binding.recipient == recipient
                && binding.principal == principal
                && binding.deadline == deadline,
            "persisted direct source binding mismatch"
        );
        ensure!(
            !matches!(
                provider_state(&record),
                Some("SUCCESS" | "FAILED" | "REFUNDED")
            ),
            "direct provider outcome closes new source preparation"
        );
        Ok(true)
    }
    fn direct_source_started(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)", [id], |row| row.get(0))?)
    }
    pub(crate) fn require_direct_first_submission(&self, id: &str) -> Result<()> {
        if let Some(record) = self.optional_direct(id)? {
            ensure!(
                !matches!(
                    provider_state(&record),
                    Some("SUCCESS" | "FAILED" | "REFUNDED")
                ),
                "direct provider outcome closes first source disclosure"
            );
        }
        Ok(())
    }
    pub fn direct_send_action(
        &self,
        id: &str,
        current_max_fee: u64,
        now: Option<u64>,
    ) -> Result<DirectSendAction> {
        let record = self.direct_by_id(id)?;
        if record.kind != "send"
            || record.authority.is_none()
            || matches!(
                provider_state(&record),
                Some("SUCCESS" | "FAILED" | "REFUNDED")
            )
        {
            return Ok(DirectSendAction::NoAutomaticAction);
        }
        let binding = self.binding_for_record(&record)?;
        let outgoing: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if outgoing {
            let operation = self.operation(id)?;
            return Ok(
                if self.operation_pending(id)?
                    && operation.attempts == 0
                    && operation.submission == "PREPARED"
                {
                    DirectSendAction::Submit
                } else {
                    DirectSendAction::NoAutomaticAction
                },
            );
        }
        if self.direct_source_started(id)?
            || binding
                .deadline
                .is_some_and(|deadline| now.is_none_or(|now| now >= deadline))
        {
            return Ok(DirectSendAction::NoAutomaticAction);
        }
        self.require_operation_source_policy(id, "direct_source_preparation")?;
        let fee = current_max_fee.min(binding.authority.max_fee_zatoshis);
        let cost = binding
            .principal
            .checked_add(fee)
            .context("direct source cost overflow")?;
        let max_input = binding
            .authority
            .max_debit_zatoshis
            .map_or(cost, |cap| cost.min(cap));
        Ok(DirectSendAction::Prepare(PrepareRequest {
            managed_funding: None,
            operation_id: id.into(),
            pool_id: None,
            recipient: binding.recipient,
            amount_zatoshis: binding.principal,
            deadline: binding.deadline,
            max_fee_zatoshis: fee,
            max_input_zatoshis: max_input,
        }))
    }
    /// A fixed derived diagnostic cannot change accepted authority or work ownership.
    pub fn direct_source_error(
        &mut self,
        id: &str,
        condition: Option<DirectSourceCondition>,
    ) -> Result<()> {
        let mut record = self.direct_by_id(id)?;
        ensure!(
            record.kind == "send" && record.authority.is_some(),
            "direct source authority is not accepted"
        );
        record.source_error = condition.map(|condition| condition.message().to_owned());
        let payload = seal(
            &self.key,
            &self.direct_aad(id),
            &serde_json::to_vec(&record)?,
        )?;
        self.db.execute(
            "UPDATE direct_operations SET payload=?2 WHERE id=?1",
            params![id, payload],
        )?;
        Ok(())
    }
    pub fn direct_source_status(&self, id: &str) -> Result<Value> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        let preparation: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)", [id], |row| row.get(0))?;
        Ok(
            serde_json::json!({"operation":exists.then(|| self.operation(id)).transpose()?,"preparation_accepted":preparation,"source_error":self.direct_by_id(id)?.source_error}),
        )
    }
    pub(crate) fn direct_source_succeeded(&self, id: &str) -> Result<bool> {
        Ok(self.optional_direct(id)?.is_some_and(|record| {
            record.kind == "send"
                && record.route == DirectRoute::Near
                && provider_state(&record) == Some("SUCCESS")
        }))
    }
    pub(crate) fn direct_source_needed(&self) -> Result<bool> {
        let ids = self
            .db
            .prepare("SELECT id FROM direct_operations WHERE observe_receive=1 AND kind='send'")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            let Some(record) = self.direct_inventory_record(&id)? else {
                continue;
            };
            if record.authority.is_none()
                || matches!(
                    provider_state(&record),
                    Some("SUCCESS" | "FAILED" | "REFUNDED")
                )
            {
                continue;
            }
            let binding = self.binding_for_record(&record)?;
            if !self.direct_source_started(&id)?
                && self.funding_restriction
                    != super::super::super::restriction::FundingRestriction::DenyNewFunding
                && self.funding_permits.is_none()
                && !binding.deadline.is_some_and(|deadline| {
                    crate::rotation::base::now()
                        .ok()
                        .is_none_or(|now| now >= deadline)
                })
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::{
        store::preparation::{CalculationInputs, InputPool, OwnedInput},
        transaction::{PreparedTransaction, TransactionFacts},
    };
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )
        .unwrap();
        (dir, store)
    }
    fn native(store: &mut Store, request: &str, cap: Option<u64>) -> DirectOperation {
        let address = crate::rotation::near::test_shielded_refund(&store.network.chain());
        let (operation, _) = store.reserve_native("owner", "send", request, json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec","amount":"50","recipient":address})).unwrap();
        store
            .accept_direct_send(
                &operation.id,
                None,
                DirectSendAuthority {
                    max_fee_zatoshis: 50,
                    max_debit_zatoshis: cap,
                },
            )
            .unwrap()
    }
    fn inputs(recipient: &str) -> CalculationInputs {
        CalculationInputs {
            destination: recipient.into(),
            inputs: vec![OwnedInput {
                pool: InputPool::Orchard,
                txid: format!("{:064x}", 1),
                output_index: 0,
            }],
        }
    }
    fn facts(fee: u64, deadline: Option<u64>) -> TransactionFacts {
        TransactionFacts {
            txid: crate::rotation::store::test_support::transaction_txid(100),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: fee,
            deadline,
        }
    }
    #[test]
    fn native_authority_binds_actual_cost_and_survives_pause_and_config_changes() {
        let (_dir, mut store) = fixture();
        store.pause_new_funding();
        let operation = native(&mut store, "native", Some(55));
        let binding = store.direct_send_binding(&operation.id).unwrap().unwrap();
        assert!(store.require_preparation_available().is_err());
        store
            .require_operation_preparation_available(&operation.id)
            .unwrap();
        store
            .require_direct_source_cost(&operation.id, 50, 5)
            .unwrap();
        assert!(
            store
                .require_direct_source_cost(&operation.id, 50, 6)
                .is_err()
        );
        assert!(
            store
                .require_direct_source_cost(&operation.id, 49, 5)
                .is_err()
        );
        store
            .require_preparation_binding(&operation.id, None, &binding.recipient, 50, None)
            .unwrap();
        assert!(
            store
                .require_preparation_binding(&operation.id, None, "replacement", 50, None)
                .is_err()
        );
        assert!(
            store
                .require_preparation_binding(&operation.id, None, &binding.recipient, 51, None)
                .is_err()
        );
        assert!(
            store
                .require_preparation_binding(&operation.id, None, &binding.recipient, 50, Some(1))
                .is_err()
        );
        let retried = store
            .accept_direct_send(
                &operation.id,
                None,
                DirectSendAuthority {
                    max_fee_zatoshis: 1,
                    max_debit_zatoshis: Some(1),
                },
            )
            .unwrap();
        assert_eq!(retried.authority, operation.authority);
        let DirectSendAction::Prepare(request) =
            store.direct_send_action(&operation.id, 10, None).unwrap()
        else {
            panic!("native send must be preparable without UTC");
        };
        assert_eq!(request.max_input_zatoshis, 55);
        assert_eq!(request.max_fee_zatoshis, 10);
        assert!(request.deadline.is_none());
        store.deny_new_funding();
        assert!(
            store
                .require_operation_preparation_available(&operation.id)
                .is_err()
        );
        assert!(
            store
                .accept_preparation(&operation.id, None, 55, Some(inputs(&binding.recipient)))
                .is_err()
        );
    }
    #[test]
    fn native_input_intent_handoff_publication_and_first_broadcast_keep_one_owner_after_restart() {
        use futures_util::FutureExt;
        let (dir, mut store) = fixture();
        let treasury = store.id().to_owned();
        let operation = native(&mut store, "handoff", Some(60));
        let binding = store.direct_send_binding(&operation.id).unwrap().unwrap();
        assert_eq!(store.direct_pending().unwrap().len(), 1);
        assert!(store.treasury_observation_needed(&[]).unwrap());
        store
            .accept_preparation(&operation.id, None, 60, Some(inputs(&binding.recipient)))
            .unwrap();
        store
            .direct_source_error(&operation.id, Some(DirectSourceCondition::Unavailable))
            .unwrap();
        assert!(
            store
                .direct_operation("owner", &operation.id)
                .unwrap()
                .source_error
                .is_some()
        );
        store.direct_source_error(&operation.id, None).unwrap();
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(!store.direct_source_needed().unwrap());
        assert!(matches!(
            store.direct_send_action(&operation.id, 50, None).unwrap(),
            DirectSendAction::NoAutomaticAction
        ));
        drop(store);
        let mut store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )
        .unwrap();
        assert!(matches!(
            store.direct_send_action(&operation.id, 50, None).unwrap(),
            DirectSendAction::NoAutomaticAction
        ));
        let revision = store.snapshot().unwrap().0;
        let _ = store.direct_changes.notified().now_or_never();
        assert!(
            store
                .prepare_with_facts(
                    &operation.id,
                    revision,
                    b"bad publication",
                    crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                    Some(facts(11, None))
                )
                .is_err()
        );
        assert_eq!(store.snapshot().unwrap().0, revision);
        store
            .prepare_with_facts(
                &operation.id,
                revision,
                b"published",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(facts(10, None)),
            )
            .unwrap();
        assert!(store.direct_changes.notified().now_or_never().is_some());
        assert_eq!(store.direct_pending().unwrap().len(), 1);
        assert!(matches!(
            store.direct_send_action(&operation.id, 1, None).unwrap(),
            DirectSendAction::Submit
        ));
        let loaded = PreparedTransaction::load(&store, &operation.id).unwrap();
        assert_eq!(
            loaded.network_identity().unwrap(),
            crate::network::IsolationId::direct_swap(&treasury, &operation.id)
        );
        store.pause_new_funding();
        store.request_broadcast(&operation.id, 1, 1, false).unwrap();
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(
            store
                .unowned_pending_sources(None)
                .unwrap()
                .contains(&operation.id)
        );
        assert!(store.require_unstarted_preparation(&operation.id).is_err());
        assert!(store.abandon_unprepared(&operation.id).is_err());
        let public = store
            .direct_source_status(&operation.id)
            .unwrap()
            .to_string();
        assert!(!public.contains("published") && !public.contains("inputs"));
    }
    fn near(store: &mut Store, request_id: &str) -> DirectOperation {
        use crate::direct_swap::route::{Asset, SwapType, ZEC, request_on, validate_quote_on};
        let now = crate::rotation::base::now().unwrap();
        let refund = crate::rotation::near::test_shielded_refund(&store.network.chain());
        let (operation,_)=store.reserve_direct("owner","send",request_id,json!({"asset_id":"foreign:usd","chain":"foreign","amount":"1000","recipient":"destination","max_source_zatoshis":"60"})).unwrap();
        store
            .save_direct_address(
                &operation.id,
                &refund,
                store.snapshot().unwrap().0,
                b"refund",
            )
            .unwrap();
        let origin = Asset {
            asset_id: ZEC.into(),
            blockchain: "zec".into(),
            decimals: 8,
            symbol: "ZEC".into(),
            metadata: Default::default(),
        };
        let destination = Asset {
            asset_id: "foreign:usd".into(),
            blockchain: "foreign".into(),
            decimals: 6,
            symbol: "USD".into(),
            metadata: Default::default(),
        };
        let request = request_on(
            &store.network.chain(),
            &origin,
            &destination,
            "destination",
            &refund,
            "1000",
            SwapType::ExactOutput,
            500,
            now + 3600,
            false,
        )
        .unwrap();
        let response = json!({"quoteRequest":request,"quote":{"amountIn":"50","minAmountIn":"45","amountOut":"1000","minAmountOut":"1000","depositAddress":"t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F","deadline":request["deadline"]}});
        let mut quote = validate_quote_on(&store.network.chain(), request, response, now).unwrap();
        quote.input = "1".into(); // Rebuild derived fields from the validated response at acceptance.
        store
            .accept_direct_send(
                &operation.id,
                Some(serde_json::to_value(quote).unwrap()),
                DirectSendAuthority {
                    max_fee_zatoshis: 50,
                    max_debit_zatoshis: Some(60),
                },
            )
            .unwrap()
    }
    #[test]
    fn near_source_acceptance_binds_original_quote_and_success_closes_exact_pending_bytes() {
        let (_dir, mut store) = fixture();
        let operation = near(&mut store, "near");
        let binding = store.direct_send_binding(&operation.id).unwrap().unwrap();
        assert_eq!(binding.principal, 50);
        assert!(matches!(
            store.direct_send_action(&operation.id, 50, None).unwrap(),
            DirectSendAction::NoAutomaticAction
        ));
        store
            .require_preparation_binding(
                &operation.id,
                None,
                &binding.recipient,
                50,
                binding.deadline,
            )
            .unwrap();
        assert!(
            store
                .require_preparation_binding(&operation.id, None, &binding.recipient, 50, None)
                .is_err()
        );
        store
            .accept_preparation(&operation.id, None, 60, Some(inputs(&binding.recipient)))
            .unwrap();
        assert_eq!(
            store.direct_pending().unwrap().len(),
            1,
            "provider status keeps its lane after input acceptance"
        );
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &operation.id,
                revision,
                b"near signed",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(facts(10, binding.deadline)),
            )
            .unwrap();
        store
            .save_direct_status(
                &operation.id,
                json!({"provider":{"status":"SUCCESS"},"observed_at":1}),
            )
            .unwrap();
        assert!(store.funding_operation_fulfilled(&operation.id).unwrap());
        assert!(!store.operation_pending(&operation.id).unwrap());
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(store.request_broadcast(&operation.id, 1, 1, false).is_err());
        assert!(!store.prepared_bytes(&operation.id).unwrap().is_empty());
    }
}
