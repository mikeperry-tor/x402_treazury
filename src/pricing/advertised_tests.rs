use super::*;
use serde_json::json;
fn config() -> PricingConfig {
    PricingConfig {
        source: "catalog".into(),
        format: PricingFormat::PaymentInfo,
        key: None,
        unit: Some("call".into()),
        usd_as_usdc: true,
    }
}
#[test]
fn exact_selection_requires_explicit_currency_unit_and_fixed_amount() {
    let cfg = config();
    let normalized = NormalizedPrice::parse(
        &cfg,
        &json!({"price":{"amount":"0.010000","currency":"USD"}}),
    );
    assert_eq!(normalized.selection_amount, Some(U256::from(10_000)));
    assert_eq!(normalized.description(), "Listed: ~$0.01/call [spec].");
    for value in [
        json!({"amount":"0.01"}),
        json!({"amount":"0.01","currency":"USD","unit":"record"}),
        json!({"amount":"0.01","currency":"USD","unit":null}),
        json!({"unit":"record","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
        json!({"amount":"0.01","currency":"USD","mode":"dynamic"}),
        json!({"amount":"0.01","currency":"USD","mode":"quote"}),
        json!({"amount":"0.0000001","currency":"USD"}),
        json!({"min":"0.01","max":"0.02","currency":"USD"}),
        json!({"amount":"0.01","currency":"USDC","network":"solana:mainnet"}),
        json!({"price":{"amount":"0.01","currency":"USD"},"protocols":[{"x402":{"network":"solana:mainnet"}}]}),
    ] {
        assert_eq!(
            NormalizedPrice::parse(&cfg, &value).selection_amount,
            None,
            "{value}"
        );
    }
    let legacy = PricingConfig::legacy("x-price");
    assert_eq!(
        NormalizedPrice::parse(&legacy, &json!({"amount":"0.01","currency":"USD"}))
            .selection_amount,
        None
    );
    assert_eq!(
        NormalizedPrice::parse(&cfg, &json!({"amount":"0","currency":"USDC"})).selection_amount,
        Some(U256::ZERO)
    );
    assert_eq!(
        NormalizedPrice::parse(&cfg, &json!({"authMode":"free"})).kind,
        TariffKind::Free
    );
}

#[test]
fn declared_x402_networks_require_an_exact_base_usdc_alternative() {
    let base = json!({"network":"eip155:8453","asset":crate::payment::USDC,"amount":"2000"});
    let solana = json!({"network":"solana:mainnet","asset":"USDC","amount":"2000"});
    let different_asset = json!({"network":"eip155:8453","asset":"USDT","amount":"2000"});
    for (protocol, eligible) in [
        (json!({"scheme":"exact","networks":[solana, base]}), true),
        (
            json!({"scheme":"exact","networks":[solana, different_asset]}),
            false,
        ),
        (json!({"scheme":"upto","networks":[base]}), false),
        (
            json!({"scheme":"exact","networks":[{"network":"eip155:8453"}]}),
            false,
        ),
        (
            json!({"scheme":"exact","networks":[{"network":"eip155:8453","asset":"USDC","scheme":"upto"}]}),
            false,
        ),
        (json!({"scheme":"exact","networks":[]}), false),
        (json!({"scheme":"exact","networks":null}), false),
        (json!({"network":"eip155:8453"}), true),
    ] {
        let value = json!({
            "price":{"amount":"0.002","currency":"USD","mode":"fixed"},
            "protocols":["x402",{"mpp":{"network":"4217"}},{"x402":protocol}]
        });
        let price = NormalizedPrice::parse(&config(), &value);
        assert_eq!(
            price.selection_amount,
            eligible.then(|| U256::from(2000)),
            "{value}"
        );
        assert_eq!(price.description(), "Listed: ~$0.002/call [spec].");
    }
}

#[test]
fn nested_effective_modes_render_consistent_dynamic_labels() {
    for value in [
        json!({"mode":"dynamic","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
        json!({"pricingMode":"quote","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
        json!({"price":{"mode":"dynamic","fixed":{"amount":"0.01","currency":"USD"}}}),
        json!({"price":{"mode":"fixed","fixed":{"amount":"0.01","currency":"USD","mode":"quote"}}}),
    ] {
        let normalized = NormalizedPrice::parse(&config(), &value);
        assert_eq!(normalized.kind, TariffKind::Dynamic);
        assert_eq!(normalized.selection_amount, None);
        assert_eq!(
            normalized.description(),
            "Listed: ~$0.01/call [spec, dynamic]."
        );
    }
    let fixed = NormalizedPrice::parse(
        &config(),
        &json!({"mode":"dynamic","price":{"mode":"fixed","fixed":{"amount":"0.01","currency":"USD"}}}),
    );
    assert_eq!(fixed.kind, TariffKind::Fixed);
    assert_eq!(fixed.selection_amount, None);
    assert_eq!(fixed.description(), "Listed: ~$0.01/call [spec].");
}

fn range_wrappers(range: Value) -> Vec<Value> {
    vec![
        range.clone(),
        json!({"price":range.clone()}),
        json!({"price":{"fixed":range.clone()}}),
        json!({"fixed":range.clone()}),
        json!({"min":range["min"],"max":range["max"],"currency":"USD",
            "price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
        json!({"price":{"min":range["min"],"max":range["max"],"currency":"USD",
            "fixed":{"amount":"0.01","currency":"USD"}}}),
    ]
}

#[test]
fn a_fixed_alias_cannot_mask_metering_in_the_same_tariff_block() {
    for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
        let mut cfg = config();
        cfg.format = format;
        let price = NormalizedPrice::parse(
            &cfg,
            &json!({
                "price":{"mode":"fixed","pricingMode":"metered","amount":"0.01",
                    "currency":"USDC","unit":"call"},
                "accepts":[{"scheme":"exact","network":"eip155:8453",
                    "asset":crate::payment::USDC,"amount":"1000"}]
            }),
        );
        assert_eq!(price.kind, TariffKind::Dynamic);
        assert_eq!(price.selection_amount, None);
        assert!(price.description().contains(", metered]."));
    }
}

#[test]
fn explicit_ranges_override_fixed_samples_across_catalog_and_external_formats() {
    let range = json!({"amount":"0.01","min":"0.01","max":"1","currency":"USD"});
    for format in [
        PricingFormat::PaymentInfo,
        PricingFormat::X402,
        PricingFormat::X402Resources,
        PricingFormat::X402Discovery,
    ] {
        let mut cfg = config();
        cfg.format = format;
        for mut value in range_wrappers(range.clone()) {
            value["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"}]);
            let normalized = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(normalized.kind, TariffKind::Range, "{value}");
            assert_eq!(normalized.selection_amount, None, "{value}");
            assert_eq!(
                normalized.description(),
                if matches!(
                    cfg.format,
                    PricingFormat::X402Resources | PricingFormat::X402Discovery
                ) {
                    "Listed: ~$0.01–$1/call [external pricing]. Advertised sample offer: ~$0.001/request [external pricing]."
                } else {
                    "Listed: ~$0.01–$1/call [spec]."
                },
                "{value}"
            );
        }
    }
}

#[test]
fn range_metadata_follows_selected_block_and_enclosing_wrappers() {
    for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
        let mut cfg = config();
        cfg.format = format;
        let (origin, offer) = if cfg.format == PricingFormat::X402Resources {
            (
                "external pricing",
                " Advertised sample offer: ~$0.001/request [external pricing].",
            )
        } else {
            ("spec", "")
        };
        let mut outer = json!({"min":"0.01","max":"1","currency":"TOK","unit":"token",
            "network":"networkA","asset":"assetA",
            "price":{"fixed":{"amount":"0.01","currency":"USD","unit":"call",
                "network":"networkB","asset":"assetB"}},
            "accepts":[{"scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"}]});
        let normalized = NormalizedPrice::parse(&cfg, &outer);
        assert_eq!(
            normalized.description(),
            format!("Listed: ~0.01 TOK–1 TOK/token [{origin}].{offer}")
        );
        assert_eq!(normalized.currency.as_deref(), Some("TOK"));
        assert_eq!(normalized.unit.as_deref(), Some("token"));
        assert_eq!(normalized.network.as_deref(), Some("networkA"));
        assert_eq!(normalized.asset.as_deref(), Some("assetA"));
        assert_eq!(normalized.selection_amount, None);
        for key in ["currency", "unit", "network", "asset"] {
            outer.as_object_mut().unwrap().remove(key);
        }
        let sparse = NormalizedPrice::parse(&cfg, &outer);
        assert_eq!(
            sparse.description(),
            format!("Listed: ~0.01–1/call [{origin}].{offer}")
        );
        assert_eq!(sparse.currency, None);
        assert_eq!(sparse.network, None);
        assert_eq!(sparse.asset, None);
        assert_eq!(sparse.selection_amount, None);
        let inner = NormalizedPrice::parse(
            &cfg,
            &json!({
                "currency":"ROOT","unit":"root-unit","network":"root-network","asset":"root-asset",
                "price":{"currency":"TOK","unit":"token","network":"parent-network","asset":"parent-asset",
                    "fixed":{"min":"0.01","max":"1"}}
            }),
        );
        assert_eq!(
            inner.description(),
            format!("Listed: ~0.01 TOK–1 TOK/token [{origin}].")
        );
        assert_eq!(inner.currency.as_deref(), Some("TOK"));
        assert_eq!(inner.unit.as_deref(), Some("token"));
        assert_eq!(inner.network.as_deref(), Some("parent-network"));
        assert_eq!(inner.asset.as_deref(), Some("parent-asset"));
        assert_eq!(inner.selection_amount, None);
    }
    let fixed = NormalizedPrice::parse(
        &config(),
        &json!({
            "currency":"USD","price":{"fixed":{"amount":"0.01"}}
        }),
    );
    assert_eq!(fixed.currency, None);
    assert_eq!(fixed.selection_amount, None);
}

#[test]
fn malformed_range_units_preserve_details_without_inventing_call_unit() {
    for unit in [json!(null), json!(false), json!({}), json!("")] {
        let value = json!({"min":"0.01","max":"1","currency":"USD","unit":unit,
            "price":{"fixed":{"amount":"0.01","currency":"USD","unit":"call"}}});
        let normalized = NormalizedPrice::parse(&config(), &value);
        assert_eq!(normalized.kind, TariffKind::Range);
        assert_eq!(normalized.selection_amount, None);
        assert_eq!(
            normalized.description(),
            "Listed: ~$0.01–$1 [spec, no unit]."
        );
    }
}

#[test]
fn external_atomic_samples_obey_nested_tariff_units_and_modes() {
    for format in [PricingFormat::X402Resources, PricingFormat::X402Discovery] {
        let mut cfg = config();
        cfg.format = format;
        let accepts = serde_json::json!([{"scheme":"exact","network":"eip155:8453",
            "asset":crate::payment::USDC,"amount":"1000"}]);
        let metered = NormalizedPrice::parse(
            &cfg,
            &json!({"accepts":accepts,
            "price":{"fixed":{"unit":"record","amount":"0.01","currency":"USD"}}}),
        );
        assert_eq!(metered.selection_amount, None);
        assert_eq!(metered.unit.as_deref(), Some("record"));
        assert_eq!(
            metered.description(),
            "Listed: ~$0.01/record [external pricing]. Advertised sample offer: ~$0.001/request [external pricing]."
        );
        let dynamic = NormalizedPrice::parse(
            &cfg,
            &json!({"accepts":accepts,
            "price":{"fixed":{"mode":"dynamic","amount":"0.01","currency":"USD"}}}),
        );
        assert_eq!(dynamic.selection_amount, None);
        assert_eq!(dynamic.kind, TariffKind::Dynamic);
        assert_eq!(
            dynamic.description(),
            "Listed: ~$0.01/call [external pricing, dynamic]. Advertised sample offer: ~$0.001/request [external pricing]."
        );
        let conflicting = NormalizedPrice::parse(
            &cfg,
            &json!({"accepts":accepts,
            "mode":"dynamic","price":{"unit":"record","fixed":{
                "mode":"fixed","unit":"call","amount":"0.01","currency":"USD"}}}),
        );
        assert_eq!(conflicting.selection_amount, None);
        assert_eq!(
            conflicting.description(),
            "Listed: ~$0.01/call [external pricing]. Advertised sample offer: ~$0.001/request [external pricing]."
        );
        let malformed = NormalizedPrice::parse(
            &cfg,
            &json!({"accepts":accepts,
            "price":{"fixed":{"unit":null}}}),
        );
        assert_eq!(malformed.selection_amount, None);
    }
}

#[test]
fn enclosing_tariff_terms_control_estimates_independently_of_display() {
    let terms = [
        (json!({"mode":"metered"}), false),
        (json!({"pricingMode":"metered"}), false),
        (json!({"mode":"fixed","pricingMode":"metered"}), false),
        (json!({"pricingMode":null}), false),
        (json!({"unit":"record"}), false),
        (json!({"unit":null}), false),
        (json!({"unit":12}), false),
        (json!({"mode":"FIXED","unit":"request"}), true),
        (json!({}), true),
    ];
    let fixed = json!({"mode":"fixed","unit":"call","amount":"0.01","currency":"USDC"});
    let accepts = json!([{"scheme":"exact","network":"eip155:8453",
        "asset":crate::payment::USDC,"amount":"1000"}]);
    for (terms, eligible) in terms {
        let mut outer = terms.clone();
        outer["price"] = json!({"fixed":fixed});
        let mut parent = terms;
        parent["fixed"] = fixed.clone();
        for mut record in [outer, json!({"price":parent})] {
            record["accepts"] = accepts.clone();
            for format in [
                PricingFormat::PaymentInfo,
                PricingFormat::X402,
                PricingFormat::X402Resources,
                PricingFormat::X402Discovery,
            ] {
                let mut cfg = config();
                cfg.format = format;
                let external = matches!(
                    cfg.format,
                    PricingFormat::X402Resources | PricingFormat::X402Discovery
                );
                let price = NormalizedPrice::parse(&cfg, &record);
                let amount = if external { 1000 } else { 10_000 };
                assert_eq!(
                    price.selection_amount,
                    eligible.then_some(U256::from(amount)),
                    "{:?}: {record}",
                    cfg.format
                );
                assert_eq!(price.kind, TariffKind::Fixed);
                assert!(price.description().starts_with("Listed: ~$0.01/call"));
                if external {
                    assert!(
                        price
                            .description()
                            .contains(&external_offer(U256::from(amount), eligible))
                    );
                }
            }
        }
    }
}

#[test]
fn external_tariff_evidence_excludes_transport_envelope_fields() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    let mut record = json!({
        "resource":"https://api.example.com/read",
        "method":"GET",
        "type":"http",
        "x402Version":2,
        "metadata":{"description":"transport annotation"}
    });
    let absent = NormalizedPrice::parse(&cfg, &record);
    assert_eq!(absent.description(), "Cost: unknown.");
    assert_eq!(absent.selection_amount, None);

    record["price"] = json!({"percentage":"2.5","minimum":"0.001"});
    record["pricingTerms"] = json!("Minimum fee applies.");
    record["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
        "asset":crate::payment::USDC,"amount":"1000"}]);
    let priced = NormalizedPrice::parse(&cfg, &record);
    assert_eq!(priced.selection_amount, None);
    assert!(priced.description().contains("Minimum fee applies."));
    assert!(
        priced
            .description()
            .contains("Advertised sample offer: ~$0.001/request")
    );
    assert!(!priced.description().contains("transport annotation"));
    assert!(!priced.description().contains("api.example.com"));

    for variable in [
        json!({"price":{"amount":"0.001","currency":"USD","percentage":"2.5"}}),
        json!({"percentage":"2.5","minimum":"0.001"}),
    ] {
        let mut record = variable;
        record["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
            "asset":crate::payment::USDC,"amount":"1000"}]);
        let price = NormalizedPrice::parse(&cfg, &record);
        assert_eq!(price.selection_amount, None);
        assert!(price.description().contains("percentage"));
        assert!(
            price
                .description()
                .contains("Advertised sample offer: ~$0.001/request")
        );
    }
}

#[test]
fn external_range_preserves_variable_billing_terms_and_request_sample() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    let record = json!({
        "price":{"min":"0.01","max":"0.02","currency":"USD","percentage":"2.5"},
        "minimum":"0.001",
        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]
    });
    let range = NormalizedPrice::parse(&cfg, &record);
    assert_eq!(range.selection_amount, None);
    assert!(range.description().contains("Listed: ~$0.01–$0.02/call"));
    assert!(
        range
            .description()
            .contains("Percentage fee [external pricing] 2.5")
    );
    assert!(
        range
            .description()
            .contains("Minimum charge [external pricing] 0.001")
    );
    assert!(
        range
            .description()
            .contains("Advertised sample offer: ~$0.001/request")
    );
}

#[test]
fn unsupported_external_offers_remain_display_evidence_only() {
    const NO_ESTIMATE: &str =
        "Advertised offers supply no exact Base USDC price estimate [external pricing].";
    for format in [PricingFormat::X402Resources, PricingFormat::X402Discovery] {
        let mut cfg = config();
        cfg.format = format;
        for (scheme, network) in [("upto", "eip155:8453"), ("exact", "eip155:1")] {
            let mut record = json!({"accepts":[{"scheme":scheme,"network":network,
                "asset":crate::payment::USDC,"amount":"1000","payTo":"fixture-payee"}]});
            let unsupported = NormalizedPrice::parse(&cfg, &record);
            assert_eq!(unsupported.kind, TariffKind::Other);
            assert_eq!(unsupported.description(), NO_ESTIMATE);
            assert_eq!(unsupported.selection_amount, None);

            record["price"] = json!({"amount":"0.01","currency":"USD"});
            let listed = NormalizedPrice::parse(&cfg, &record);
            assert_eq!(
                listed.description(),
                format!("Listed: ~$0.01/call [external pricing]. {NO_ESTIMATE}")
            );
            assert_eq!(listed.selection_amount, None);

            record["accepts"].as_array_mut().unwrap().push(json!({
                "scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"
            }));
            let supported = NormalizedPrice::parse(&cfg, &record);
            assert_eq!(supported.selection_amount, Some(U256::from(1000)));
            assert!(!supported.description().contains(NO_ESTIMATE));
        }
        let malformed = NormalizedPrice::parse(&cfg, &json!({"accepts":[{}, false]}));
        assert_eq!(malformed.kind, TariffKind::Unknown);
    }
}

#[test]
fn native_tariffs_keep_explicit_billing_terms_and_conditions() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    for (price, estimate) in [
        (
            json!({"amount":"0.01","currency":"USDC","unit":"call"}),
            Some(U256::from(1000)),
        ),
        (
            json!({"min":"0.01","max":"1","currency":"USDC","unit":"call"}),
            None,
        ),
    ] {
        let mut record = json!({
            "price": price,
            "pricingTerms": "An additional fee applies to delivered rows.",
            "conditions": ["Unused quota expires after the call."],
            "accepts":[{"scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"}]
        });
        record["price"]["conditions"] = json!(["Successful requests only."]);
        record["price"]["pricingTerms"] = Value::Null;
        let normalized = NormalizedPrice::parse(&cfg, &record);
        assert_eq!(normalized.selection_amount, estimate);
        assert!(normalized.description().contains("Listed: ~$0.01"));
        assert!(
            normalized
                .description()
                .contains("An additional fee applies to delivered rows.")
        );
        assert!(
            normalized
                .description()
                .contains("Successful requests only.")
        );
        assert!(
            normalized
                .description()
                .contains("Unused quota expires after the call.")
        );

        record["price"]["conditions"] = record["conditions"].clone();
        record["price"]["pricingTerms"] = json!("");
        let repeated = NormalizedPrice::parse(&cfg, &record);
        assert_eq!(
            repeated
                .description()
                .matches("Unused quota expires after the call.")
                .count(),
            1
        );
        assert!(
            repeated
                .description()
                .contains("An additional fee applies to delivered rows.")
        );
    }
}

#[test]
fn fractional_json_range_bounds_remain_visible_as_display_evidence() {
    for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
        let mut cfg = config();
        cfg.format = format;
        for (min, max, expected) in [
            (json!(0.01), json!(0.1), "~$0.01–$0.10/call"),
            (json!("0.01"), json!(0.1), "~$0.01–$0.10/call"),
            (Value::Null, json!(0.1), "up to ~$0.10/call"),
        ] {
            let price = NormalizedPrice::parse(
                &cfg,
                &json!({"price":{
                    "min":min,"max":max,"currency":"USD","unit":"call",
                }}),
            );
            assert_eq!(price.kind, TariffKind::Range);
            assert_eq!(price.selection_amount, None);
            assert!(
                price.description().contains(expected),
                "{}",
                price.description()
            );
            assert!(price.legacy_listing.is_none());
        }
    }
}

#[test]
fn partial_and_malformed_ranges_never_become_fixed_estimates() {
    for range in [
        json!({"min":"0.01"}),
        json!({"max":"1"}),
        json!({"min":null,"max":"1"}),
        json!({"min":"bad","max":"1"}),
        json!({"min":"0.01","max":{}}),
        json!({"min":false,"max":[]}),
    ] {
        let mut range = range;
        range["amount"] = json!("0.01");
        range["currency"] = json!("USD");
        for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
            let mut cfg = config();
            cfg.format = format;
            for mut value in range_wrappers(range.clone()) {
                value["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
                    "asset":crate::payment::USDC,"amount":"1000"}]);
                let normalized = NormalizedPrice::parse(&cfg, &value);
                assert_eq!(normalized.kind, TariffKind::Range, "{value}");
                assert_eq!(normalized.selection_amount, None, "{value}");
                assert!(
                    [
                        "Listed: from ",
                        "Listed: up to ",
                        "Published price range unavailable"
                    ]
                    .iter()
                    .any(|prefix| normalized.description().starts_with(prefix)),
                    "{value}"
                );
            }
        }
    }
    let mut free = json!({"authMode":"free","min":null});
    assert_eq!(
        NormalizedPrice::parse(&config(), &free).selection_amount,
        None
    );
    free.as_object_mut().unwrap().remove("min");
    assert_eq!(
        NormalizedPrice::parse(&config(), &free).selection_amount,
        Some(U256::ZERO)
    );
}

#[test]
fn x402_dollar_forwarding_preserves_explicit_range_restrictions() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402;
    for range in [
        json!({"min":"0.01","max":"1"}),
        json!({"min":"0.01"}),
        json!({"max":null}),
    ] {
        let mut value = range;
        value["price"] = json!("$0.01");
        let normalized = NormalizedPrice::parse(&cfg, &value);
        assert_eq!(normalized.selection_amount, None, "{value}");
        if value.get("max").is_some_and(Value::is_string) {
            assert_eq!(normalized.kind, TariffKind::Range);
            assert_eq!(normalized.description(), "Listed: ~$0.01–$1/call [spec].");
        } else {
            assert_eq!(normalized.kind, TariffKind::Range);
        }
    }
}

#[test]
fn atomic_alternatives_require_supported_asset_and_choose_one_offer() {
    let base = json!({"scheme":"exact","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"4000"});
    let mut lower = base.clone();
    lower["amount"] = json!("1000");
    assert_eq!(
        NormalizedPrice::payment_requirements(&json!([base.clone(), lower])).selection_amount,
        Some(U256::from(1000))
    );
    for (key, value) in [
        ("asset", json!("USDC")),
        ("network", json!("base")),
        ("amount", json!("0.001")),
        ("scheme", json!("upto")),
        ("extra", json!({"decimals":18})),
    ] {
        let mut altered = base.clone();
        altered[key] = value;
        assert_eq!(
            NormalizedPrice::payment_requirements(&json!([altered])).selection_amount,
            None
        );
    }
}
#[test]
fn fixed_zero_agrees_with_an_explicit_free_declaration() {
    for format in [
        PricingFormat::PaymentInfo,
        PricingFormat::X402,
        PricingFormat::X402Resources,
        PricingFormat::X402Discovery,
    ] {
        let mut cfg = config();
        cfg.format = format;
        let value = json!({"authMode":"free", "protocols":[],
            "conditions":"Public records only.",
            "price":{"amount":"0.000000","currency":"USD","mode":"fixed"}});
        let mut numeric_zero = value.clone();
        numeric_zero["price"]["amount"] = json!(0.0);
        for evidence in [&value, &numeric_zero] {
            let free = NormalizedPrice::parse(&cfg, evidence);
            assert_eq!(free.kind, TariffKind::Free, "{evidence}");
            assert_eq!(free.selection_amount, Some(U256::ZERO));
            assert!(free.description().contains("Public records only."));
        }

        for (key, competing) in [
            ("amount", json!("0.01")),
            ("amount", Value::Null),
            ("pricingTerms", json!("Extra charges apply")),
            ("mode", json!("metered")),
            ("min", Value::Null),
            ("price", json!(false)),
        ] {
            let mut value = value.clone();
            value[key] = competing;
            let conflict = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(conflict.selection_amount, None, "{value}");
            assert_ne!(conflict.kind, TariffKind::Free, "{value}");
        }
    }

    let mut cfg = config();
    cfg.format = PricingFormat::X402;
    let mut value = json!({"authMode":"free","price":"$0.00"});
    assert_eq!(NormalizedPrice::parse(&cfg, &value).kind, TariffKind::Free);
    value["amount"] = json!("0.01");
    assert_eq!(NormalizedPrice::parse(&cfg, &value).selection_amount, None);
}

#[test]
fn conflicting_free_and_paid_fields_never_select_a_zero_hint() {
    for format in [
        PricingFormat::PaymentInfo,
        PricingFormat::X402,
        PricingFormat::X402Resources,
        PricingFormat::X402Discovery,
    ] {
        let mut cfg = config();
        cfg.format = format;
        for mut value in [
            json!({"authMode":"free","amount":"0.01","currency":"USD"}),
            json!({"authMode":"free","fixed":{"amount":"0.01","currency":"USD"}}),
            json!({"authMode":"free","price":"$0.01"}),
            json!({"authMode":"free","paymentRequired":true}),
            json!({"authMode":"free","accepts":[{"scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"}]}),
        ] {
            value["conditions"] = json!("Public records only.");
            let price = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(price.selection_amount, None, "{value}");
            assert_ne!(price.kind, TariffKind::Free, "{value}");
            assert!(
                price
                    .description()
                    .contains("Conflicting free and paid metadata.")
            );
            assert!(price.description().contains("Public records only."));
            assert!(!price.description().contains("Advertised exact offer:"));
        }
    }
}

#[test]
fn free_declarations_check_terms_inside_and_outside_tariff_wrappers() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    let mixed = NormalizedPrice::parse(
        &cfg,
        &json!({"authMode":"free","pricingTerms":"0.003 USDC per delivered row"}),
    );
    assert_eq!(mixed.selection_amount, None);
    assert!(mixed.description().contains("0.003 USDC per delivered row"));
    assert!(
        mixed
            .description()
            .contains("Conflicting free and paid metadata.")
    );

    for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
        cfg.format = format;
        for value in [
            json!({"price":{"authMode":"free","accepts":[{"scheme":"exact",
                "network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]}}),
            json!({"authMode":"free","price":[{"amount":"0.003","unit":"row"}]}),
            json!({"price":{"authMode":"free","pricingMode":"metered"}}),
            json!({"price":{"authMode":"free","mode":"fixed","pricingMode":"metered"}}),
            json!({"authMode":"free","mode":"fixed","price":{"pricingMode":"metered"}}),
        ] {
            let mixed = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(mixed.selection_amount, None, "{value}");
            assert!(
                mixed
                    .description()
                    .contains("Conflicting free and paid metadata.")
            );
        }
        for value in [
            json!({"price":{"authMode":"free"}}),
            json!({"price":{"fixed":{"authMode":"free"}}}),
            json!({"authMode":"free","conditions":"Public records only."}),
            json!({"authMode":"free","pricingTerms":"  "}),
            json!({"price":{"authMode":"free","pricingTerms":[]}}),
            json!({"price":{"fixed":{"authMode":"free","conditions":"Public records only."}}}),
        ] {
            let free = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(free.kind, TariffKind::Free);
            assert_eq!(free.selection_amount, Some(U256::ZERO));
            if value.get("conditions").is_some()
                || value.pointer("/price/fixed/conditions").is_some()
            {
                assert!(free.description().contains("Public records only."));
            }
        }
    }
}

#[test]
fn external_decimal_evidence_is_retained_without_a_selection_hint() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    let price = NormalizedPrice::parse(
        &cfg,
        &json!({"amount":0.01,"currency":"USD","unit":"record","mode":"metered"}),
    );
    assert_eq!(price.selection_amount, None);
    assert!(price.description().contains("\"amount\":0.01"));
    assert!(price.description().contains("\"unit\":\"record\""));
}

#[test]
fn normalized_dollars_follow_the_configured_adapter_shape() {
    let mut cfg = config();
    let value = json!({"price":"$0.05"});
    let authored = NormalizedPrice::parse(&cfg, &value);
    assert_eq!(authored.description(), "Pricing: \"$0.05\" [spec].");
    assert_eq!(authored.selection_amount, None);
    let scalar = NormalizedPrice::parse(&cfg, &json!(0.05));
    assert_eq!(scalar.description(), "Pricing: 0.05 [spec].");
    assert_eq!(scalar.selection_amount, None);
    let opaque = NormalizedPrice::parse(
        &cfg,
        &json!({
            "price":{"percentage":"2.5","unit":"transaction"},
            "pricingTerms":"Minimum fee applies",
            "authMode":"paid"
        }),
    );
    assert_eq!(opaque.selection_amount, None);
    assert!(opaque.description().contains("percentage"));
    assert!(opaque.description().contains("Minimum fee applies"));
    assert!(!opaque.description().contains("authMode"));

    cfg.format = PricingFormat::X402;
    assert_eq!(
        NormalizedPrice::parse(&cfg, &json!("$0.05")).selection_amount,
        None
    );
    assert_eq!(
        NormalizedPrice::parse(&cfg, &value).selection_amount,
        Some(U256::from(50_000))
    );
}

#[test]
fn unit_is_resolved_without_inventing_a_catalog_call() {
    let cfg = PricingConfig::legacy("x-payment-info");
    let legacy = NormalizedPrice::parse(&cfg, &json!({"amount":"0.05","currency":"USD"}));
    assert_eq!(legacy.description(), "Listed: ~$0.05 [spec, no unit].");
    assert_eq!(legacy.selection_amount, None);
    let mut external = cfg;
    external.format = PricingFormat::X402Resources;
    let mut record = json!({"accepts":[{"scheme":"exact","network":"eip155:8453",
        "asset":crate::payment::USDC,"amount":"1000"}]});
    let implicit = NormalizedPrice::parse(&external, &record);
    assert_eq!(implicit.unit.as_deref(), Some("request"));
    assert_eq!(implicit.selection_amount, Some(U256::from(1000)));

    external.unit = Some("call".into());
    record["unit"] = json!("request");
    let explicit = NormalizedPrice::parse(&external, &record);
    assert!(explicit.description().contains("/request"));
    assert_eq!(explicit.selection_amount, Some(U256::from(1000)));
    record["network"] = json!("solana:mainnet");
    assert_eq!(
        NormalizedPrice::parse(&external, &record).selection_amount,
        None
    );
}

#[test]
fn handle_clones_share_fresh_advertisements_independently_of_observed_cache() {
    let price = NormalizedPrice::parse(&config(), &json!({"amount":"0.01","currency":"USD"}));
    let handle = AdvertisedPricing::new(price.clone(), "local.json".into(), None);
    let clone = handle.clone();
    assert_eq!(clone.amount(), Some(U256::from(10_000)));
    handle.set_expires(0);
    assert_eq!(clone.amount(), None);
    handle.replace(price, "metadata".into(), None);
    assert_eq!(clone.amount(), Some(U256::from(10_000)));
    assert_eq!(AdvertisedPricing::default().amount(), None);
}
#[test]
fn x402_dollar_format_and_decimal_parsing_are_exact() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402;
    assert_eq!(
        NormalizedPrice::parse(&cfg, &json!({"price":"$0.015","network":"eip155:8453"}))
            .selection_amount,
        Some(U256::from(15_000))
    );
    for text in ["-1", "1e-3", "1.", ".1", " 1", "1.0000000"] {
        assert_eq!(decimal(text), None);
    }
    assert_eq!(decimal("0.000001"), Some(U256::from(1)));
    assert_eq!(decimal(&U256::MAX.to_string()), None);
}

#[test]
fn external_exact_samples_do_not_override_explicit_metering_or_dynamic_tariffs() {
    let mut cfg = config();
    cfg.format = PricingFormat::X402Resources;
    let record = json!({"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]});
    assert_eq!(
        NormalizedPrice::parse(&cfg, &record).selection_amount,
        Some(U256::from(1000))
    );
    for patch in [
        json!({"unit":"record"}),
        json!({"unit":null}),
        json!({"mode":"dynamic"}),
        json!({"price":{"min":"0.001","max":"1"}}),
        json!({"unit":"call","price":{"unit":"record"}}),
        json!({"unit":"call","price":{"unit":null}}),
        json!({"mode":"fixed","price":{"mode":"dynamic"}}),
    ] {
        let mut modified = record.clone();
        modified
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(
            NormalizedPrice::parse(&cfg, &modified).selection_amount,
            None
        );
    }
    for (key, value) in [("unit", "record"), ("mode", "metered")] {
        let mut metered = record.clone();
        metered[key] = json!(value);
        assert!(
            NormalizedPrice::parse(&cfg, &metered)
                .description()
                .contains("Advertised sample offer: ~$0.001/request [external pricing].")
        );
    }
    for mode in [None, Some("metered")] {
        let mut metered = record.clone();
        metered["unit"] = json!("record");
        if let Some(mode) = mode {
            metered["mode"] = json!(mode);
        }
        let price = NormalizedPrice::parse(&cfg, &metered);
        assert_eq!(price.unit.as_deref(), Some("record"));
        assert_eq!(price.selection_amount, None);
        assert!(price.description().contains("$0.001/request"));
        assert!(price.description().contains("Billing unit: record."));
    }
    let mut ranged = record;
    ranged["price"] =
        json!({"min":"0.001","max":"1","currency":"USDC","unit":"call","mode":"dynamic"});
    assert!(
        NormalizedPrice::parse(&cfg, &ranged)
            .description()
            .contains("$0.001–$1/call")
    );
    ranged
        .as_object_mut()
        .unwrap()
        .insert("unit".into(), json!("record"));
    ranged["price"].as_object_mut().unwrap().remove("unit");
    let range = NormalizedPrice::parse(&cfg, &ranged);
    assert_eq!(range.selection_amount, None);
    assert!(range.description().contains("$0.001–$1/record"));
    assert!(
        range
            .description()
            .contains("Advertised sample offer: ~$0.001/request")
    );
}

#[test]
fn amountless_dynamic_and_metered_modes_do_not_repeat_mode_tag() {
    let cfg = config();
    let dynamic = NormalizedPrice::parse(&cfg, &json!({"mode": "dynamic"}));
    assert_eq!(dynamic.kind, TariffKind::Dynamic);
    assert_eq!(dynamic.selection_amount, None);
    assert_eq!(dynamic.description(), "Pricing: dynamic [spec].");

    let metered = NormalizedPrice::parse(&cfg, &json!({"mode": "metered"}));
    assert_eq!(metered.kind, TariffKind::Dynamic);
    assert_eq!(metered.selection_amount, None);
    assert_eq!(metered.description(), "Pricing: metered [spec].");
}
