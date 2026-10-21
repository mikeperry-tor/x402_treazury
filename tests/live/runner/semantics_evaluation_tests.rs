// Full evaluator contracts, shared with the original unit-test response builder.
fn expected_outcome(status: &str, category: &str) -> Value {
    json!({"status": status, "category": category})
}
fn expected_checks(status: &str, checks: &[&str]) -> Value {
    json!({"status":status,"category":"declared_assertions","checks":checks.iter().enumerate().map(|(index,status)|json!({"index":index,"status":status})).collect::<Vec<_>>()})
}
#[test]
fn evaluator_outer_and_content_precedence_preserves_complete_outcomes() {
    let cases = [
        (json!({}), "manual_review_required", "missing_mcp_result"),
        (
            json!({"result":[]}),
            "manual_review_required",
            "missing_mcp_result",
        ),
        (
            json!({"result":{"isError":true,"content":null}}),
            "failed",
            "mcp_is_error",
        ),
        (
            json!({"result":{"isError":null,"content":null}}),
            "manual_review_required",
            "malformed_mcp_error_flag",
        ),
        (
            json!({"result":{"isError":false,"content":{}}}),
            "manual_review_required",
            "unsupported_mcp_content",
        ),
        (
            json!({"result":{"content":[{"type":"image","text":null},{}]}}),
            "manual_review_required",
            "no_declared_provider_assertions",
        ),
        (
            json!({"result":{"structuredContent":{"isError":true},"content":[{"type":"text","text":null}]}}),
            "manual_review_required",
            "malformed_text_content",
        ),
        (body(json!({"isError":true})), "failed", "nested_is_error"),
        (
            body(json!({"isError":"true","error":"ordinary data"})),
            "manual_review_required",
            "no_declared_provider_assertions",
        ),
    ];
    for (response, status, category) in cases {
        assert_eq!(
            evaluate(&response, &[]).unwrap(),
            expected_outcome(status, category),
            "{response}"
        );
    }
}
#[test]
fn evaluator_deduplicates_json_and_preserves_mixed_assertion_indices() {
    let value = json!({"a/b":{"~":null},"error":"ordinary data"});
    let response = json!({"result":{"structuredContent":value,"content":[
        {"type":"text","text":value.to_string()},
        {"type":"text","text":serde_json::to_string_pretty(&value).unwrap()},
        {"type":"text","text":"Usage: search"},
        {"type":"image","text":null}
    ]}});
    let checks = [
        Check::JsonExists {
            pointer: "/a~1b/~0".into(),
        },
        Check::JsonEquals {
            pointer: "/a~1b/~0".into(),
            value: Value::Null,
        },
        Check::TextContains {
            text: "Usage:".into(),
        },
        Check::JsonEquals {
            pointer: "/error".into(),
            value: json!("ordinary data"),
        },
        Check::JsonExists {
            pointer: "/missing".into(),
        },
        Check::JsonEquals {
            pointer: "/missing".into(),
            value: Value::Null,
        },
    ];
    assert_eq!(
        evaluate(&response, &checks).unwrap(),
        expected_checks(
            "failed",
            &["passed", "passed", "passed", "passed", "failed", "failed"]
        )
    );
    assert_eq!(
        evaluate(&response, &checks[..4]).unwrap(),
        expected_checks("passed", &["passed"; 4])
    );
    let null = json!({"result":{"structuredContent":null,"content":[]}});
    assert_eq!(
        evaluate(
            &null,
            &[Check::JsonEquals {
                pointer: String::new(),
                value: Value::Null
            }]
        )
        .unwrap(),
        expected_checks("passed", &["passed"])
    );
}
#[test]
fn evaluator_ambiguous_and_absent_evidence_retains_failed_over_unknown_priority() {
    let checks = [
        Check::JsonExists {
            pointer: "/ok".into(),
        },
        Check::TextContains {
            text: "needle".into(),
        },
    ];
    for (response, expected) in [
        (
            json!({"result":{"content":[]}}),
            expected_checks(
                "manual_review_required",
                &["manual_review_required", "manual_review_required"],
            ),
        ),
        (
            json!({"result":{"content":[{"type":"text","text":"needle"}]}}),
            expected_checks(
                "manual_review_required",
                &["manual_review_required", "passed"],
            ),
        ),
        (
            json!({"result":{"structuredContent":{"ok":true},"content":[]}}),
            expected_checks(
                "manual_review_required",
                &["passed", "manual_review_required"],
            ),
        ),
        (
            json!({"result":{"structuredContent":{"ok":true},"content":[{"type":"text","text":"{}"},{"type":"text","text":"needle"}]}}),
            expected_checks(
                "manual_review_required",
                &["manual_review_required", "passed"],
            ),
        ),
        (
            json!({"result":{"structuredContent":{"ok":true},"content":[{"type":"text","text":"{}"}]}}),
            expected_checks("failed", &["manual_review_required", "failed"]),
        ),
        (
            json!({"result":{"structuredContent":{},"content":[]}}),
            expected_checks("failed", &["failed", "manual_review_required"]),
        ),
    ] {
        assert_eq!(
            evaluate(&response, &checks).unwrap(),
            expected,
            "{response}"
        );
    }
}
#[test]
fn evaluator_check_validation_precedes_even_outer_failure() {
    let response = json!({"result":{"isError":true}});
    let cases = [
        (
            vec![
                Check::TextContains {
                    text: String::new()
                };
                33
            ],
            "provider checks exceed 32-assertion limit; no checks dropped",
        ),
        (
            vec![Check::JsonExists {
                pointer: format!("bad{}", "x".repeat(4096)),
            }],
            "provider check exceeds 4096-byte limit; no truncation permitted",
        ),
        (
            vec![Check::JsonExists {
                pointer: "/bad~2".into(),
            }],
            "invalid provider-check JSON pointer",
        ),
        (
            vec![
                Check::TextContains {
                    text: String::new(),
                },
                Check::JsonExists {
                    pointer: "invalid".into(),
                },
            ],
            "text assertion must not be empty",
        ),
    ];
    for (checks, expected) in cases {
        assert_eq!(
            evaluate(&response, &checks).unwrap_err().to_string(),
            expected
        );
    }
    let check = Check::TextContains { text: "x".into() };
    assert_eq!(
        evaluate(&response, &vec![check; 32]).unwrap(),
        expected_outcome("failed", "mcp_is_error")
    );
    let empty_size = serde_json::to_vec(&Check::TextContains {
        text: String::new(),
    })
    .unwrap()
    .len();
    let exact = Check::TextContains {
        text: "x".repeat(4096 - empty_size),
    };
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), 4096);
    assert_eq!(
        evaluate(&response, &[exact]).unwrap(),
        expected_outcome("failed", "mcp_is_error")
    );
}
#[test]
fn evaluator_content_limit_is_checked_before_collection_and_nested_scan() {
    for count in [1000, 1001] {
        let response = json!({"result":{"structuredContent":{"isError":true},"content":vec![json!({"type":"image"});count]}});
        let expected = if count == 1000 {
            expected_outcome("failed", "nested_is_error")
        } else {
            expected_outcome(
                "manual_review_required",
                "content_block_limit_1000_exceeded",
            )
        };
        assert_eq!(evaluate(&response, &[]).unwrap(), expected);
    }
}
#[test]
fn evaluator_scan_limits_preserve_boundaries_and_override_observed_failure() {
    let limit = expected_outcome(
        "manual_review_required",
        "semantic_scan_limit_100000_nodes_64_levels_exceeded",
    );
    for depth in [64, 65] {
        let mut value = Value::Null;
        for _ in 0..depth {
            value = json!([value]);
        }
        let response = json!({"result":{"structuredContent":value,"content":[]}});
        assert_eq!(
            evaluate(&response, &[]).unwrap(),
            if depth == 64 {
                expected_outcome("manual_review_required", "no_declared_provider_assertions")
            } else {
                limit.clone()
            }
        );
    }
    for children in [99999, 100000] {
        let response =
            json!({"result":{"structuredContent":vec![Value::Null;children],"content":[]}});
        assert_eq!(
            evaluate(&response, &[]).unwrap(),
            if children == 99999 {
                expected_outcome("manual_review_required", "no_declared_provider_assertions")
            } else {
                limit.clone()
            }
        );
    }
    // The later text JSON is scanned first and reports failure, but the earlier
    // structured value still exhausts the scan: partial evidence cannot classify.
    let response = json!({"result":{"structuredContent":vec![Value::Null;100000],"content":[{"type":"text","text":"{\"isError\":true}"}]}});
    assert_eq!(evaluate(&response, &[]).unwrap(), limit);
}
