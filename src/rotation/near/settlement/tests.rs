use super::*;

fn fixture() -> (Quote, Value) {
    let request = json!({"swapType":"EXACT_OUTPUT","originAsset":ZEC,"destinationAsset":USDC,"depositType":"ORIGIN_CHAIN","recipientType":"DESTINATION_CHAIN","amount":"7000","recipient":format!("0x{:040x}",1),"deadline":"2030-01-01T00:00:00Z"});
    let response = json!({"quoteRequest":request,"quote":{"depositAddress":"original-deposit","amountIn":"100000","amountOut":"7000","deadline":"2030-01-01T00:00:00Z"}});
    let quote = Quote {
        request,
        response: response.clone(),
        input: 100000,
        deadline: 1893456000,
        deposit: Some("original-deposit".into()),
    };
    let value = json!({"status":"SUCCESS","quoteResponse":response,"swapDetails":{
        "originChainTxHashes":[{"hash":format!("{:064X}",170),"explorerUrl":"https://private.invalid/origin"}],
        "destinationChainTxHashes":[{"hash":format!("0x{:064X}",187),"explorerUrl":"https://private.invalid/destination"}],
        "amountIn":"100000","amountOut":"7000","depositedAmount":"100500","refundedAmount":"500",
        "refundReason":"private upstream prose","unknown":{"arbitrary":true}
    }});
    (quote, value)
}

#[test]
fn original_quote_bound_candidates_are_unverified_and_retain_atomic_amounts() {
    let (quote, value) = fixture();
    let result = parse(&quote, &value).unwrap();
    assert_eq!(result.status(), &SwapStatus::Success);
    assert!(result.issues().is_empty());
    let data = result.settlement().unwrap();
    assert_eq!(
        data.origin_transactions,
        vec![B256::from(U256::from(170).to_be_bytes::<32>())]
    );
    assert_eq!(
        data.destination_transactions,
        vec![B256::from(U256::from(187).to_be_bytes::<32>())]
    );
    assert_eq!(data.amount_in, Some(U256::from(100000)));
    assert_eq!(data.amount_out, Some(U256::from(7000)));
    assert_eq!(data.deposited_amount, Some(U256::from(100500)));
    assert_eq!(data.refunded_amount, Some(U256::from(500)));
    assert_eq!(
        result.quote_digest(),
        &<[u8; 32]>::from(Sha256::digest(serde_json::to_vec(&quote).unwrap()))
    );
}

#[test]
fn status_only_and_unknown_fields_never_invent_settlement() {
    let (quote, mut value) = fixture();
    value.as_object_mut().unwrap().remove("swapDetails");
    value["unknown"] =
        json!({"destinationChainTxHashes":[{"hash":format!("0x{:064x}",1)}],"amountOut":"7000"});
    let result = parse(&quote, &value).unwrap();
    assert_eq!(result.status(), &SwapStatus::Success);
    assert!(result.settlement().is_none());
    assert_eq!(result.issues(), &[SettlementIssue::MissingDetails]);
    value["status"] = json!("FUTURE_STATUS");
    assert_eq!(
        parse(&quote, &value).unwrap().into_status(),
        SwapStatus::Unknown
    );
}

#[test]
fn missing_amounts_remain_unknown_and_partial_amounts_do_not_become_delivery_proof() {
    let (quote, mut value) = fixture();
    value["swapDetails"]
        .as_object_mut()
        .unwrap()
        .remove("refundedAmount");
    value["swapDetails"]["depositedAmount"] = Value::Null;
    value["swapDetails"]["amountOut"] = json!("1");
    value["status"] = json!("INCOMPLETE_DEPOSIT");
    let result = parse(&quote, &value).unwrap();
    let data = result.settlement().unwrap();
    assert_eq!(data.amount_out, Some(U256::from(1)));
    assert_eq!(data.refunded_amount, None);
    assert_eq!(data.deposited_amount, None);
    assert_eq!(result.status(), &SwapStatus::IncompleteDeposit);
}

#[test]
fn malformed_known_evidence_invalidates_all_candidates_without_losing_status() {
    let (quote, original) = fixture();
    for (field, invalid, issue) in [
        (
            "originChainTxHashes",
            json!(["not-object"]),
            SettlementIssue::InvalidOriginTransactions,
        ),
        (
            "originChainTxHashes",
            json!([{"hash":format!("0x{:064x}",1)}]),
            SettlementIssue::InvalidOriginTransactions,
        ),
        (
            "destinationChainTxHashes",
            json!([{"hash":"private arbitrary prose"}]),
            SettlementIssue::InvalidDestinationTransactions,
        ),
        (
            "destinationChainTxHashes",
            json!([{"hash":format!("0x{:064x}",1)},{"hash":format!("0x{:064x}",1)}]),
            SettlementIssue::InvalidDestinationTransactions,
        ),
        ("amountIn", json!(-1), SettlementIssue::InvalidAmountIn),
        ("amountOut", json!("1.5"), SettlementIssue::InvalidAmountOut),
        (
            "depositedAmount",
            json!("-1"),
            SettlementIssue::InvalidDepositedAmount,
        ),
        (
            "refundedAmount",
            json!(format!("1{}", "0".repeat(100))),
            SettlementIssue::InvalidRefundedAmount,
        ),
    ] {
        let mut value = original.clone();
        value["swapDetails"][field] = invalid;
        let result = parse(&quote, &value).unwrap();
        assert_eq!(result.status(), &SwapStatus::Success);
        assert!(result.settlement().is_none(), "{field}");
        assert!(result.issues().contains(&issue));
    }
}

#[test]
fn missing_correlations_and_malformed_details_stay_incomplete() {
    let (quote, original) = fixture();
    for details in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"originChainTxHashes":[],"destinationChainTxHashes":[]}),
    ] {
        let mut value = original.clone();
        value["swapDetails"] = details;
        let result = parse(&quote, &value).unwrap();
        assert!(result.settlement().is_none());
        assert!(!result.issues().is_empty());
        assert_eq!(result.status(), &SwapStatus::Success);
    }
}

#[test]
fn original_deposit_and_request_mismatches_are_rejected_without_private_prose() {
    let (quote, original) = fixture();
    for pointer in [
        "/quoteResponse/quote/depositAddress",
        "/quoteResponse/quoteRequest/recipient",
        "/quoteResponse/quoteRequest/amount",
        "/quoteResponse/quoteRequest/destinationAsset",
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = json!("private mismatched value");
        let error = parse(&quote, &value).err().unwrap().to_string();
        assert!(!error.contains("private mismatched value"));
    }
}

#[test]
fn changed_echoed_quote_terms_or_route_do_not_supply_candidates() {
    let (quote, original) = fixture();
    for field in ["amountIn", "amountOut", "deadline", "depositMemo"] {
        let mut value = original.clone();
        value["quoteResponse"]["quote"][field] = json!("different");
        let result = parse(&quote, &value).unwrap();
        assert!(result.settlement().is_none());
        assert!(
            result
                .issues()
                .contains(&SettlementIssue::QuoteTermsMismatch)
        );
    }
    let mut quote = quote;
    quote.request["originAsset"] = json!("unsupported");
    let mut value = original;
    value["quoteResponse"]["quoteRequest"] = quote.request.clone();
    let result = parse(&quote, &value).unwrap();
    assert_eq!(result.issues(), &[SettlementIssue::UnsupportedRoute]);
    assert!(result.settlement().is_none());
}
