//! Shareable projection of the same private registry report. No raw identifiers or prose.
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use serde_json::Value;
use std::fmt::Write;

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Json,
    Markdown,
}
fn atomics(value: &Value) -> Result<u64> {
    match value {
        Value::String(s) => Ok(s.parse()?),
        _ => value.as_u64().context("report amount missing or invalid"),
    }
}
fn signed_atomics(value: &Value) -> Result<&str> {
    let text = value.as_str().context("missing signed accounting change")?;
    let digits = text.strip_prefix('-').unwrap_or(text);
    ensure!(
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        "invalid signed accounting change"
    );
    digits.parse::<alloy_primitives::U256>()?;
    Ok(text)
}
fn dollars(value: u64) -> String {
    format!("${}.{:06}", value / 1_000_000, value % 1_000_000)
}
fn status(value: &Value) -> &'static str {
    match value.as_str() {
        Some("UNATTEMPTED") => "unattempted",
        Some("RESERVED") => "reserved",
        Some("DISPATCHING") => "dispatching",
        Some("COMPLETED") => "completed",
        Some("TRANSPORT_UNCERTAIN") => "transport uncertain",
        Some("SKIPPED_TARGET_REACHED") => "skipped: target reached",
        Some("NOT_EXECUTED" | "not_executed") => "not executed",
        Some("PASSED" | "passed") => "passed",
        Some("FAILED" | "failed") => "failed",
        Some("UNOBSERVED" | "unobserved") => "unobserved",
        Some("PENDING") => "pending",
        Some("NOT_SIGNED") => "not signed",
        Some("USED") => "used",
        Some("EXPIRED_UNUSED") => "expired unused",
        Some("incomplete") => "incomplete",
        Some("manual_review_required") => "manual review required",
        Some("invalid" | "invalid_or_incomplete") => "invalid or incomplete",
        _ => "unknown (inspect private JSON)",
    }
}
fn observed(value: &Value) -> &'static str {
    match value.as_bool() {
        Some(true) => "observed",
        Some(false) => "not observed",
        None => "unavailable",
    }
}
fn semantic_category(value: &Value) -> &'static str {
    match value.as_str() {
        Some("mcp_is_error") => "MCP error",
        Some("nested_is_error") => "nested JSON isError",
        Some("declared_assertions") => "declared assertions",
        Some("no_declared_provider_assertions") => "no declared provider assertions",
        Some("content_block_limit_1000_exceeded") => "1000-content-block limit exceeded",
        Some("semantic_scan_limit_100000_nodes_64_levels_exceeded") => {
            "100000-node / 64-level scan limit exceeded"
        }
        Some("missing_or_changed_response_evidence") => "missing or changed response evidence",
        Some("response_not_completed") => "response not completed",
        Some("validated_rotation_target_reached") => "validated rotation target reached",
        Some(
            "missing_mcp_result"
            | "malformed_mcp_error_flag"
            | "malformed_text_content"
            | "unsupported_mcp_content",
        ) => "unsupported or malformed content",
        _ => "unclassified (inspect private JSON)",
    }
}
pub fn markdown(report: &Value) -> Result<String> {
    ensure!(report["version"] == 1, "unsupported report version");
    let cases = report["cases"].as_array().context("report cases missing")?;
    ensure!(
        cases.len() <= 10000,
        "Markdown report exceeds 10000 cases; no partial report emitted"
    );
    let debit = &report["canonical_api_debits"];
    let valid = debit["status"] == "validated";
    let proofs = debit["cases"]
        .as_object()
        .context("report debit map missing")?;
    ensure!(
        valid || proofs.is_empty(),
        "invalid debit report contains amounts"
    );
    let mut charged = 0u64;
    let mut total = 0u64;
    let mut proof_count = 0usize;
    let mut known = std::collections::BTreeSet::new();
    let mut out = String::from(
        "# Live integration report\n\nThis is a sanitized projection of private registry evidence, not a new qualification run.\nCase and assertion numbers follow the private JSON order. Identifiers, addresses,\ntransaction references, credentials, paths and provider content are omitted.\n\n## API accounting\n\n| Case | Execution | MCP result | Settlement | Charged reservation | Verified debit |\n| --- | --- | --- | --- | ---: | ---: |\n",
    );
    if let Some(summary) = report.get("summary") {
        let view: crate::report_model::Report = serde_json::from_value(summary.clone())?;
        ensure!(view.version == 1, "unsupported summary version");
        ensure!(
            view.providers.len() <= 10000,
            "provider summary exceeds 10000 rows; no partial report"
        );
        let mut overview = String::from(
            "## Provider stages\n\nStages are independent observations. Saved source accounting is not a fresh balance.\nOwned Tor requires its separate control audit; Tor-only failures do not establish Tor causation.\nProviders follow private JSON order; unselected calls remain not attempted.\n\n| Provider | Catalog | Help | Pricing | Execution | Provider semantics | Canonical payment |\n| --- | --- | --- | --- | --- | --- | --- |\n",
        );
        fn labels(values: impl Iterator<Item = crate::report_model::State>) -> String {
            let values: std::collections::BTreeSet<_> = values.map(|v| v.label()).collect();
            if values.is_empty() {
                "not attempted".into()
            } else {
                values.into_iter().collect::<Vec<_>>().join(", ")
            }
        }
        for (i, provider) in view.providers.iter().enumerate() {
            writeln!(
                overview,
                "| {} | {} | {} | {} | {} | {} | {} |",
                i + 1,
                labels(provider.catalog.iter().map(|s| s.state)),
                labels(provider.help.iter().map(|s| s.state)),
                labels(provider.pricing.iter().map(|s| s.state)),
                labels(provider.cases.iter().map(|c| c.execution)),
                labels(provider.cases.iter().map(|c| c.semantics)),
                labels(provider.cases.iter().map(|c| c.payment))
            )?;
        }
        if !view.source_inventory_complete {
            overview.push_str("\nHistorical source inventory is incomplete; this table includes known case sources only.\n");
        }
        if view.providers.iter().any(|p| {
            p.pricing.iter().any(|s| {
                s.pricing_evidence.as_ref().is_some_and(|e| {
                    e.outcomes.iter().any(|(kind, count)| {
                        *kind != x402_treazury::pricing::Outcome::Discovered && *count > 0
                    })
                })
            })
        }) {
            overview.push_str("\nPricing probe failures were observed even where the discovery stage completed; private JSON retains all outcome and HTTP-status counts.\n");
        }
        overview.push('\n');
        let index = out
            .find("## API accounting")
            .context("report accounting section missing")?;
        out.insert_str(index, &overview);
    }
    for (i, case) in cases.iter().enumerate() {
        let id = case["case"].as_str().context("case identity missing")?;
        ensure!(known.insert(id), "duplicate case in single-run report");
        let reservation = atomics(&case["charged_reservation_atomic"])?;
        charged = charged
            .checked_add(reservation)
            .context("reservation total overflow")?;
        let amount = proofs.get(id).map(atomics).transpose()?;
        let fee = if let Some(amount) = amount {
            total = total
                .checked_add(amount)
                .context("verified debit total overflow")?;
            proof_count += 1;
            dollars(amount)
        } else {
            "not established".into()
        };
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            i + 1,
            status(&case["execution"]),
            status(&case["semantic"]),
            status(&case["settlement"]),
            dollars(reservation),
            fee
        )?;
    }
    ensure!(
        proofs.keys().all(|id| known.contains(id.as_str())),
        "debit proof has no report case"
    );
    writeln!(
        out,
        "\nSelected-run charged API reservations: **{}**. These remain charged after failed calls.",
        dollars(charged)
    )?;
    if valid {
        writeln!(
            out,
            "Verified API debits: **{}** across **{proof_count}** cases. Missing proofs are not zero-cost claims.",
            dollars(total)
        )?;
    } else {
        out.push_str(
            "Verified API debit accounting is **invalid or incomplete**; no total is asserted.\n",
        );
    }
    writeln!(
        out,
        "\nRegistry-wide charged API reservations: **{}**; source reservations: **{} zatoshis**; reserved funding jobs: **{}**. Source reservations are limits, not measured Zcash fees.",
        dollars(atomics(&report["api_reserved_atomic"])?),
        atomics(&report["source_reserved_zatoshis"])?,
        atomics(&report["funding_jobs_reserved"])?
    )?;
    if let Some(observations) = report["treasury_accounting"]["observations"].as_array() {
        out.push_str("\n## Persisted treasury accounting\n\nTreasury-wide saved observations include history outside this run. These are not fresh chain reads or new spending authority. Refund outputs are reported separately, not subtracted again from costs. Missing observations do not establish zero balances.\n");
        if observations.is_empty() {
            out.push_str("No clean-shutdown accounting snapshot was recorded.\n");
        }
        for (index, observation) in observations.iter().enumerate() {
            let summary = &observation["summary"];
            let comparison = &observation["baseline_comparison"];
            if comparison["scope"] == "registry_authorization_to_snapshot" {
                out.push_str("\nChanges since registry authorization (may span multiple runs; not payment or fresh-chain proofs):\n\n| Source bookkeeping (zatoshis) | Baseline | Saved observation | Change |\n| --- | ---: | ---: | ---: |\n");
                for (key, label) in [
                    ("recorded_principal_zatoshis", "Principal"),
                    ("recorded_fees_zatoshis", "Fees"),
                    ("recorded_consumed_zatoshis", "Consumed cost"),
                    ("source_reserved_zatoshis", "Treasury reservations"),
                    ("recorded_refund_outputs_zatoshis", "Refund outputs"),
                ] {
                    let row = &comparison["source"][key];
                    writeln!(
                        out,
                        "| {label} | {} | {} | {} |",
                        atomics(&row["baseline"])?,
                        atomics(&row["observed"])?,
                        signed_atomics(&row["change"])?
                    )?;
                }
                if comparison["wallets"]["aggregate_change_atomic"].is_null() {
                    out.push_str(
                        "Aggregate USDC balance change: unobserved (missing wallet anchors).\n",
                    );
                } else {
                    writeln!(
                        out,
                        "Aggregate saved USDC balance change: **{} atomic**; role changes and external deposits are not API debits.",
                        signed_atomics(&comparison["wallets"]["aggregate_change_atomic"])?
                    )?;
                }
                if comparison["zec"]["status"] == "recorded" {
                    writeln!(
                        out,
                        "Saved shielded ZEC changes: confirmed **{} zatoshis**, spendable **{} zatoshis**; funding, change confirmation and external transfers may affect these values.",
                        signed_atomics(
                            &comparison["zec"]["balances"]["confirmed_shielded_zatoshis"]["change"]
                        )?,
                        signed_atomics(
                            &comparison["zec"]["balances"]["spendable_shielded_zatoshis"]["change"]
                        )?
                    )?;
                } else {
                    out.push_str(
                        "ZEC baseline change: unobserved (missing saved sync observations).\n",
                    );
                }
            }
            let attributed = &observation["run_source_accounting"];
            if attributed["scope"] == "run_funding_permits" {
                writeln!(
                    out,
                    "Run-attributed source costs: principal **{} zatoshis**, fees **{} zatoshis**, consumed **{} zatoshis**; treasury reservations **{} zatoshis**. Registry unreleased bounds: **{} zatoshis**; released bounds: **{} zatoshis**; attempts without treasury budget evidence: **{}**. Refund outputs: **{} zatoshis**, reported separately. Registry bounds remain conservative after settlement and are not additional spending.",
                    atomics(&attributed["recorded_principal_zatoshis"])?,
                    atomics(&attributed["recorded_fees_zatoshis"])?,
                    atomics(&attributed["recorded_consumed_zatoshis"])?,
                    atomics(&attributed["source_reserved_zatoshis"])?,
                    atomics(&attributed["registry_unreleased_bounds_zatoshis"])?,
                    atomics(&attributed["registry_released_bounds_zatoshis"])?,
                    atomics(&attributed["attempts_without_treasury_budget"])?,
                    atomics(&attributed["recorded_refund_outputs_zatoshis"])?
                )?;
            } else {
                out.push_str("Run source-cost attribution was not recorded for this snapshot.\n");
            }

            writeln!(
                out,
                "\nSnapshot {}: recorded principal **{} zatoshis**, recorded fees **{} zatoshis**, consumed cost **{} zatoshis**, pending source reservations **{} zatoshis**, refund outputs **{} zatoshis**.",
                index + 1,
                atomics(&summary["recorded_principal_zatoshis"])?,
                atomics(&summary["recorded_fees_zatoshis"])?,
                atomics(&summary["recorded_consumed_zatoshis"])?,
                atomics(&summary["source_reserved_zatoshis"])?,
                atomics(&summary["recorded_refund_outputs_zatoshis"])?
            )?;
            let exposure = summary["unresolved_payment_exposure_atomic"]
                .as_str()
                .context("missing saved payment exposure")?;
            ensure!(
                !exposure.is_empty() && exposure.bytes().all(|b| b.is_ascii_digit()),
                "invalid saved payment exposure"
            );
            writeln!(
                out,
                "Saved unresolved payment exposure: **{exposure} USDC atomic**; not a verified debit or available balance."
            )?;
            if !summary["zec_observation"].is_null() {
                writeln!(
                    out,
                    "Saved shielded ZEC: confirmed **{} zatoshis**, spendable **{} zatoshis** (freshness not asserted).",
                    atomics(&summary["zec_observation"]["confirmed_shielded_zatoshis"])?,
                    atomics(&summary["zec_observation"]["spendable_shielded_zatoshis"])?
                )?;
            }
            out.push_str("\n| Wallet role | Wallets | Anchored observations | Unobserved | Saved balance (USDC atomic) |\n| --- | ---: | ---: | ---: | ---: |\n");
            for role in ["ACTIVE", "READY", "ALLOCATED", "RETIRED"] {
                let row = &summary["wallet_roles"][role];
                if row.is_null() {
                    continue;
                }
                let balance = row["saved_balance_atomic"]
                    .as_str()
                    .context("missing saved wallet balance")?;
                ensure!(
                    !balance.is_empty() && balance.bytes().all(|b| b.is_ascii_digit()),
                    "invalid saved wallet balance"
                );
                writeln!(
                    out,
                    "| {role} | {} | {} | {} | {balance} |",
                    atomics(&row["wallets"])?,
                    atomics(&row["anchored_observations"])?,
                    atomics(&row["unobserved_wallets"])?
                )?;
            }
        }
    }
    if let Some(rows) = report["fee_observations"]["cases"].as_array() {
        ensure!(
            rows.len() == cases.len(),
            "fee rows differ from report cases"
        );
        out.push_str("\n## Observed fee samples\n\nAdmission is a pre-signing amount, not a debit. These samples do not establish a provider-wide maximum or minimum wallet size. Observed challenge offers may be rejected or unselected; they grant no payment authority.\n\n| Case | Admitted amount | Above $0.01 | Verified debit above $0.01 |\n| --- | ---: | --- | --- |\n");
        for (index, (row, case)) in rows.iter().zip(cases).enumerate() {
            ensure!(
                row["case"] == case["case"],
                "fee row order differs from cases"
            );
            let admitted = if row["admitted_atomic"].is_null() {
                "not established".into()
            } else {
                dollars(atomics(&row["admitted_atomic"])?)
            };
            let flag = |v: &Value| match v.as_bool() {
                Some(true) => "yes",
                Some(false) => "no",
                None => "not established",
            };
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                index + 1,
                admitted,
                flag(&row["admitted_above_one_cent"]),
                flag(&row["verified_debit_above_one_cent"])
            )?;
        }
        out.push_str("\n| Case | Challenge observation | Offers | Base USDC offers above $0.01 | Base USDC amounts unknown |\n| --- | --- | ---: | ---: | ---: |\n");
        for (index, row) in rows.iter().enumerate() {
            let observations = row["challenge_observations"].as_array();
            if observations.is_none_or(|v| v.is_empty()) {
                writeln!(
                    out,
                    "| {} | not observed | unavailable | unavailable | unavailable |",
                    index + 1
                )?;
                continue;
            }
            for observation in observations.expect("checked") {
                let label = match observation["status"].as_str() {
                    Some("observed") => "observed",
                    Some("header_absent") => "header absent; body offers not observed",
                    Some("duplicate_headers") => "duplicate headers",
                    Some("header_limit_exceeded") => "header limit exceeded (65536 bytes)",
                    Some("offer_limit_exceeded") => "offer limit exceeded (128 offers)",
                    Some("malformed_header" | "malformed_offers") => "malformed",
                    _ => "unknown (inspect private JSON)",
                };
                if label == "observed" {
                    let offers = observation["offers"]
                        .as_array()
                        .context("observed offers missing")?;
                    let above = offers
                        .iter()
                        .filter(|o| o["above_one_cent"] == true)
                        .count();
                    let unknown = offers
                        .iter()
                        .filter(|o| o["base_usdc"] == true && o["amount_atomic"].is_null())
                        .count();
                    writeln!(
                        out,
                        "| {} | {label} | {} | {above} | {unknown} |",
                        index + 1,
                        offers.len()
                    )?;
                } else {
                    writeln!(
                        out,
                        "| {} | {label} | unavailable | unavailable | unavailable |",
                        index + 1
                    )?;
                }
            }
        }
        if report["fee_observations"]["challenges_status"] != "validated" {
            out.push_str("\nChallenge correlation is unavailable or invalid; no offer prices are qualified.\n");
        }
    }
    if let Some(phases) = report["catalog_stages"].as_array() {
        writeln!(
            out,
            "\nCatalog inspection (initial inputs may be local or remote; shared loads are not separate network requests):\n"
        )?;
        writeln!(
            out,
            "| Inspection | Completed | Failed | Cancelled | Not started |\n| --- | ---: | ---: | ---: | ---: |"
        )?;
        for phase in phases {
            let label = match phase["phase"].as_str() {
                Some("initial_inspection") => "Initial inputs",
                Some("frozen_reload") => "Frozen reload",
                _ => "Unknown inspection",
            };
            if phase["status"] == "unobserved" {
                writeln!(
                    out,
                    "| {label} | unobserved | unobserved | unobserved | unobserved |"
                )?;
                continue;
            }
            let rows = phase["sources"]
                .as_array()
                .context("missing catalog observations")?;
            let count = |state: &str| rows.iter().filter(|r| r["state"] == state).count();
            writeln!(
                out,
                "| {label} | {} | {} | {} | {} |",
                count("completed"),
                count("failed"),
                count("cancelled"),
                count("not_started")
            )?;
        }
    }
    if let Some(rows) = report["pricing_stages"].as_array() {
        out.push_str("\n| Pricing observation | Stage | Assertion | Observed endpoints | Available prices | Cache initializations | Cache hits | Shared results | Expired | Capped | HTTP codes (count) |\n| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
        for (index, row) in rows.iter().enumerate() {
            let stage = match row["stage"].as_str() {
                Some("disabled") => "disabled",
                Some("completed") => "completed",
                Some("failed") => "failed",
                Some("incomplete") => "incomplete",
                _ => "not observed",
            };
            let assertion = if row["required"] == true {
                status(&row["assessment"])
            } else {
                "not required"
            };
            let e = &row["evidence"];
            let count = |v: &Value| {
                v.as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unavailable".into())
            };
            let cache = |key: &str| {
                if e.is_object() {
                    count(&e["cache"].get(key).cloned().unwrap_or(serde_json::json!(0)))
                } else {
                    "unavailable".into()
                }
            };
            let codes = if let Some(codes) = e["http_statuses"].as_object() {
                let mut rows = Vec::new();
                for (code, n) in codes {
                    if let (Ok(code), Some(n)) = (code.parse::<u16>(), n.as_u64())
                        && (100..=999).contains(&code)
                    {
                        rows.push(format!("{code} ({n})"));
                    }
                }
                if rows.is_empty() {
                    "none observed".into()
                } else {
                    rows.join(", ")
                }
            } else {
                "unavailable".into()
            };
            writeln!(
                out,
                "| {} | {stage} | {assertion} | {} | {} | {} | {} | {} | {} | {} | {codes} |",
                index + 1,
                count(&e["observed"]),
                count(&e["available_prices"]),
                cache("initialized"),
                cache("hit"),
                cache("shared"),
                count(&e["expired"]),
                count(&e["capped"])
            )?;
        }
        out.push_str("\nPricing completion is not provider health or payment qualification. Cached failure/success outcomes are observations, not fresh requests; cache initialization does not prove a request reached the provider. Startup prices never authorize payment.\n");
    }
    if let Some(rows) = report["help_stages"].as_array() {
        out.push_str("\n| Help case | Assessment | Cache path | Retrieval | Required |\n| --- | --- | --- | --- | --- |\n");
        for (index, case) in cases.iter().enumerate() {
            if let Some(row) = rows.iter().find(|r| r["case"] == case["case"]) {
                let cache = match row["cache"].as_str() {
                    Some("fetch") => "fetch",
                    Some("hit") => "hit",
                    Some("shared") => "shared",
                    _ => "not observed",
                };
                let retrieval = match row["result"]["status"].as_str() {
                    Some("succeeded") => "succeeded",
                    Some("failed") => match row["result"]["failure_category"].as_str() {
                        Some("http_connect") => "connection failure",
                        Some("http_timeout") => "timeout",
                        Some("http_status") => "HTTP error",
                        _ => "other failure",
                    },
                    _ => "not observed",
                };
                let retrieval = if retrieval == "HTTP error" {
                    row["result"]["http_status"]
                        .as_u64()
                        .filter(|code| (100..=999).contains(code))
                        .map(|code| format!("HTTP {code}"))
                        .unwrap_or_else(|| retrieval.to_owned())
                } else {
                    retrieval.to_owned()
                };
                writeln!(
                    out,
                    "| {} | {} | {cache} | {retrieval} | {} |",
                    index + 1,
                    status(&row["status"]),
                    if row["required"] == true { "yes" } else { "no" }
                )?;
            }
        }
    }
    out.push_str("\n## Assertions\n\n| Concurrency batch | Result | MCP overlap | Application overlap | Payment-work overlap | Signed-request future overlap |\n| --- | --- | --- | --- | --- | --- |\n");
    for (i, batch) in report["concurrency"]
        .as_array()
        .context("concurrency report missing")?
        .iter()
        .enumerate()
    {
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            i + 1,
            status(&batch["status"]),
            observed(&batch["mcp_overlap"]),
            observed(&batch["application_overlap"]),
            observed(&batch["payment_work_overlap"]),
            observed(&batch["signed_request_future_overlap"])
        )?;
    }
    out.push_str("\n| Rotation/lifecycle phase | Result |\n| --- | --- |\n");
    for (i, phase) in report["rotation"]
        .as_array()
        .context("rotation report missing")?
        .iter()
        .enumerate()
    {
        writeln!(out, "| {} | {} |", i + 1, status(&phase["status"]))?;
    }
    if let Some(groups) = report["reliability"].as_array() {
        out.push_str("\n| Reliability comparison | Result | Planned | Passed | Failed | Incomplete |\n| --- | --- | ---: | ---: | ---: | ---: |\n");
        for (index, group) in groups.iter().enumerate() {
            let count = |field: &str| {
                group[field]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unavailable".into())
            };
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} |",
                index + 1,
                status(&group["status"]),
                count("planned"),
                count("passed"),
                count("failed"),
                count("incomplete")
            )?;
        }
        out.push_str("\nReliability comparisons describe observed end-to-end cases. They do not automatically attribute failures to a provider or Tor or guarantee future reliability.\n");
    }
    out.push_str("\n| Provider case | Content assessment | Category | Required |\n| --- | --- | --- | --- |\n");
    if let Some(rows) = report["provider_semantics"].as_array() {
        for (index, case) in cases.iter().enumerate() {
            let row = rows
                .iter()
                .find(|r| r["case"] == case["case"])
                .context("provider assessment missing")?;
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                index + 1,
                status(&row["status"]),
                semantic_category(&row["category"]),
                if row["required"] == true { "yes" } else { "no" }
            )?;
        }
    } else {
        out.push_str("\nProvider-content assessment is unavailable in this report.\n");
    }
    out.push_str("\nEmpty assertion tables mean no assertions were selected, not a pass.\n\nTor/confinement and cover qualification require their separate retained artifacts;\nthis registry report does not independently verify them. Treasury balances, actual\nsource fees, residual assets and exhaustion warnings are not inferred from API\nreservations or missing debit proofs. No overall live-suite success is asserted.\n");
    if let Some(eligibility) = report.get("eligibility") {
        ensure!(
            eligibility["execution_authorized"] == false,
            "eligibility report cannot grant authority"
        );
        let assessment = match eligibility["status"].as_str() {
            Some("observed") => "observed",
            Some("refused") => "refused; inspect private JSON",
            _ => anyhow::bail!("invalid eligibility assessment"),
        };
        let unstarted = eligibility["run_unstarted"]
            .as_bool()
            .context("missing unstarted-run observation")?;
        writeln!(
            out,
            "\n## Eligibility observation\n\nCase-window assessment: {assessment}. Run unstarted: {}.\nThis grants no execution authority. Started or reserved runs cannot run again;\nwindow eligibility alone does not establish safe execution.\n",
            if unstarted { "yes" } else { "no" }
        )?;
    }
    ensure!(
        out.len() <= crate::files::DOCUMENT_BYTES,
        "Markdown report exceeds evidence byte limit; no partial report emitted"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn report() -> Value {
        json!({"version":1,"api_reserved_atomic":60000,"source_reserved_zatoshis":200000,"funding_jobs_reserved":1,
            "canonical_api_debits":{"status":"validated","cases":{"secret-case":"14000"}},
            "cases":[{"case":"secret-case","execution":"COMPLETED","semantic":"PASSED","settlement":"USED","charged_reservation_atomic":20000,"wallet":"secret-wallet"},
                {"case":"failed-case","execution":"COMPLETED","semantic":"FAILED","settlement":"NOT_SIGNED","charged_reservation_atomic":20000},
                {"case":"skipped-case","execution":"SKIPPED_TARGET_REACHED","semantic":"NOT_EXECUTED","settlement":"NOT_SIGNED","charged_reservation_atomic":0}],
            "concurrency":[{"status":"incomplete","reason":"secret-error","mcp_overlap":true,"signed_request_future_overlap":false}],
            "rotation":[{"status":"incomplete","wallet":"secret-wallet"}],"runtime_events":[{"password":"secret-token","body":"secret-response"}]})
    }
    #[test]
    fn eligibility_projection_is_observational_and_redacts_private_details() {
        let mut r = report();
        for state in ["observed", "refused"] {
            r["eligibility"] = json!({"status":state,"run_unstarted":false,"execution_authorized":false,
                "reason":"secret-reason", "windows":{"eligible":["secret-case"]}});
            let text = markdown(&r).unwrap();
            assert!(text.contains("Run unstarted: no"));
            assert!(text.contains("grants no execution authority"));
            assert!(!text.contains("secret"));
        }
        r["eligibility"]["execution_authorized"] = json!(true);
        assert!(markdown(&r).is_err());
    }
    #[test]
    fn shareable_projection_preserves_accounting_and_omits_raw_evidence() {
        let text = markdown(&report()).unwrap();
        assert!(!text.contains("secret"));
        for expected in [
            "$0.040000",
            "$0.014000",
            "$0.060000",
            "200000 zatoshis",
            "not established",
            "skipped: target reached",
            "not observed",
            "unavailable",
            "incomplete",
        ] {
            assert!(text.contains(expected), "{expected}");
        }
    }
    #[test]
    fn joined_provider_table_redacts_identities_and_marks_partial_inventory() {
        let mut value = report();
        value["summary"] = json!({"version":1,"source_inventory_complete":false,
            "providers":[{"source":"secret-provider","catalog":[{"scope":"secret-phase","state":"failed","http_status":429,"pricing_evidence":null}],
                "help":[],"pricing":[],"cases":[]}],
            "source_funding":{"saved_accounting":"unobserved","observations":0,"reserved_zatoshis":200000,"reserved_jobs":1},
            "lifecycle":[],"network":"owned_control_audit_required"});
        let text = markdown(&value).unwrap();
        assert!(!text.contains("secret"));
        assert!(text.contains("Historical source inventory is incomplete"));
        assert!(text.contains("| 1 | failed | not attempted"));
        assert!(text.find("Provider stages").unwrap() < text.find("API accounting").unwrap());
        value["summary"]["providers"][0]["catalog"][0]["state"] = json!("secret-error");
        assert!(markdown(&value).is_err());
    }
    #[test]
    fn treasury_accounting_distinguishes_unobserved_balances_and_redacts_identities() {
        let mut value = report();
        value["treasury_accounting"] = json!({"observations":[]});
        assert!(
            markdown(&value)
                .unwrap()
                .contains("No clean-shutdown accounting snapshot")
        );
        value["treasury_accounting"]["observations"] = json!([{"session":"private-session","summary":crate::accounting::summarize(&crate::accounting::fixture()).unwrap()}]);
        let text = markdown(&value).unwrap();
        for expected in [
            "100 zatoshis",
            "8 zatoshis",
            "108 zatoshis",
            "5 USDC atomic",
            "| ALLOCATED | 1 | 0 | 1 | 0 |",
            "| RETIRED | 1 | 1 | 0 | 7 |",
            "not fresh chain reads",
        ] {
            assert!(text.contains(expected), "{expected}");
        }
        assert!(!text.contains("private-"));
        assert!(!text.contains("secret"));
    }
    #[test]
    fn attributed_source_projection_keeps_historical_costs_separate() {
        let mut value = report();
        let data = crate::accounting::fixture();
        let attributed = crate::accounting_attribution::source(
            &data,
            &[json!({"operation":"deposit","source_bound":110,"released":false})],
        )
        .unwrap();
        value["treasury_accounting"] = json!({"observations":[{"summary":crate::accounting::summarize(&data).unwrap(),"run_source_accounting":attributed}]});
        let text = markdown(&value).unwrap();
        assert!(text.contains("Run-attributed source costs: principal **100 zatoshis**, fees **5 zatoshis**, consumed **105 zatoshis**"));
        assert!(text.contains("recorded fees **8 zatoshis**"));
        assert!(!text.contains("private-"));
    }
    #[test]
    fn baseline_projection_preserves_signed_changes_without_identifiers() {
        let mut value = report();
        let data = crate::accounting::fixture();
        let mut historical = data.clone();
        historical["source_budget_entries"][2]["reserved"] = json!(100);
        let comparison = crate::accounting_baseline::compare(
            &crate::accounting_baseline::snapshot(&historical),
            &data,
        )
        .unwrap();
        value["treasury_accounting"] = json!({"observations":[{"summary":crate::accounting::summarize(&data).unwrap(),"baseline_comparison":comparison}]});
        let text = markdown(&value).unwrap();
        assert!(text.contains("| Treasury reservations | 100 | 70 | -30 |"));
        assert!(text.contains("may span multiple runs"));
        assert!(text.contains("unobserved (missing wallet anchors)"));
        assert!(!text.contains("private-"));
        value["treasury_accounting"]["observations"][0]["baseline_comparison"]["source"]["recorded_fees_zatoshis"]
            ["change"] = json!("private-content");
        assert!(markdown(&value).is_err());
    }
    #[test]
    fn fee_samples_remain_anonymous_and_missing_debits_are_explicit() {
        let mut value = report();
        value["application_evidence"] = json!({"payment_attempts_correlated":1});
        value["runtime_events"] = json!([{"kind":"application_payment","detail":{"case":"secret-case","amount":"14000"}}]);
        value["fee_observations"] = crate::fees::report(&value).unwrap();
        let text = markdown(&value).unwrap();
        assert!(!text.contains("secret"));
        assert!(text.contains("| 1 | $0.014000 | yes | yes |"));
        assert!(text.contains("| 2 | not established | not established | not established |"));
        assert!(text.contains("may be rejected or unselected"));
        value["fee_observations"]["cases"][0]["case"] = json!("wrong");
        assert!(markdown(&value).is_err());
    }
    #[test]
    fn pricing_projection_keeps_counts_and_http_codes_without_source_identifiers() {
        let mut value = report();
        value["pricing_stages"] = json!([{"source":"secret-source","session":"secret-session","stage":"completed","required":true,"assessment":"passed",
            "evidence":{"observed":2,"available_prices":0,"expired":1,"capped":3,"cache":{"initialized":1,"hit":1},"http_statuses":{"403":1,"429":1}}}]);
        let text = markdown(&value).unwrap();
        assert!(
            text.contains(
                "| 1 | completed | passed | 2 | 0 | 1 | 1 | 0 | 1 | 3 | 403 (1), 429 (1) |"
            )
        );
        assert!(text.contains("Startup prices never authorize payment"));
        assert!(!text.contains("secret"));
    }
    #[test]
    fn catalog_projection_does_not_turn_frozen_success_into_live_success() {
        let mut value = report();
        value["catalog_stages"] = json!([
            {"phase":"initial_inspection","status":"unobserved","sources":[]},
            {"phase":"frozen_reload","status":"completed","sources":[{"source":"secret-source","state":"completed","stage":"generation","http_status":null}]}
        ]);
        let text = markdown(&value).unwrap();
        assert!(
            text.contains("| Initial inputs | unobserved | unobserved | unobserved | unobserved |")
        );
        assert!(text.contains("| Frozen reload | 1 | 0 | 0 | 0 |"));
        assert!(!text.contains("secret"));
    }
    #[test]
    fn help_stage_projection_exposes_failure_stage_without_private_content() {
        let mut value = report();
        value["help_stages"] = json!([{"case":"secret-case","required":true,"status":"failed","cache":"fetch","result":{"status":"failed","failure_category":"http_status","http_status":403,"body":"secret-docs"}}]);
        let text = markdown(&value).unwrap();
        assert!(text.contains("| 1 | failed | fetch | HTTP 403 | yes |"));
        assert!(!text.contains("secret"));
    }
    #[test]
    fn challenge_report_discloses_bounds_and_preserves_redaction() {
        let mut value = report();
        value["fee_observations"] = crate::fees::report(&value).unwrap();
        value["fee_observations"]["cases"][0]["challenge_observations"] = json!([
            {"status":"observed","secret":"private-url","offers":[{"above_one_cent":true}]},
            {"status":"header_limit_exceeded","raw":"secret-content","offers":[]}]);
        let text = markdown(&value).unwrap();
        assert!(text.contains("| 1 | observed | 1 | 1 |"));
        assert!(text.contains("header limit exceeded (65536 bytes)"));
        assert!(text.contains("correlation is unavailable or invalid"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("private-url"));
    }
    #[test]
    fn reliability_counts_are_visible_without_private_identifiers() {
        let mut value = report();
        value["reliability"] = json!([{"status":"incomplete","planned":3,"passed":1,"failed":1,"incomplete":1,
            "samples":[{"case":"secret-case","phase":"secret-phase","reason":"secret-response"}]}]);
        let text = markdown(&value).unwrap();
        assert!(text.contains("| 1 | incomplete | 3 | 1 | 1 | 1 |"));
        assert!(text.contains("do not automatically attribute failures"));
        assert!(!text.contains("secret"));
        value["reliability"][0] = json!({"status":"invalid","reason":"secret-error"});
        let text = markdown(&value).unwrap();
        assert!(text.contains("unavailable"));
        assert!(!text.contains("secret"));
    }
    #[test]
    fn invalid_proofs_unknown_statuses_and_overflow_never_become_success_or_silent_omission() {
        let mut value = report();
        value["canonical_api_debits"] = json!({"status":"invalid_or_incomplete","cases":{}});
        value["cases"][0]["semantic"] = json!("secret-vendor-message");
        let text = markdown(&value).unwrap();
        assert!(text.contains("unknown (inspect private JSON)"));
        assert!(text.contains("no total is asserted"));
        assert!(!text.contains("secret"));
        value["cases"][0]["charged_reservation_atomic"] = json!(u64::MAX);
        assert!(markdown(&value).is_err());
        let mut value = report();
        value["canonical_api_debits"]["cases"]["missing-case"] = json!("1");
        assert!(markdown(&value).is_err());
        let mut value = report();
        value["cases"] = json!(vec![value["cases"][0].clone(); 10001]);
        assert!(
            markdown(&value)
                .unwrap_err()
                .to_string()
                .contains("no partial report")
        );
    }
}
