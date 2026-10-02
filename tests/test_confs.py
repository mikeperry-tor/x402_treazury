"""Regression tests for the generic-launcher conf files in confs/.

Runs the exact conf files against committed spec snapshots, so tool names,
requireds, enums, bounds, prices, and conf overrides stay pinned offline. If
a vendor spec drifts, refresh tests/fixtures/*.json and re-check the conf
still matches. Conf path "name" (e.g. "glassnode/glassnode") maps to fixture
"{name with / -> _}_openapi.json"; pass spec= to build from another
committed artifact (the hand-authored glassnode digest lives in confs/).
"""

from __future__ import annotations

import dataclasses
import json
from pathlib import Path

from x402_mcp.digest import build_operations
from x402_mcp.generic import (
    _generic_parser,
    build_tools,
    load_source,
    resolve_config,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
CONF_DIR = REPO_ROOT / "confs"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def _tools_from_conf(name: str, spec: str | None = None) -> list:
    """Load confs/<name>.json but build from the committed spec snapshot."""
    conf_path = CONF_DIR / f"{name}.json"
    pre, _rest = _generic_parser().parse_known_args(["--config", str(conf_path)])
    cfg = resolve_config(pre)
    snapshot = dataclasses.replace(
        cfg,
        spec=spec or str(FIXTURES / f"{name.replace('/', '_')}_openapi.json"),
        probe_pricing=False,  # never probe in tests
    )
    ops, _title, server = load_source(snapshot.spec, snapshot.pricing_key)
    return build_tools(
        snapshot,
        ops,
        base_url=snapshot.base_url or server,
        prefix=snapshot.prefix,
        probe=False,
    )


def test_deepline_conf_replaces_handcoded_launcher():
    by = {t.name: t for t in _tools_from_conf("deepline")}
    # same 11 tools, same names and requireds as the removed deepline.py
    assert sorted(by) == [
        "deepline_ads_search",
        "deepline_company_enrich",
        "deepline_contacts_by_role",
        "deepline_email_from_linkedin",
        "deepline_email_personal",
        "deepline_email_validate",
        "deepline_email_work",
        "deepline_linkedin_find",
        "deepline_person_from_email",
        "deepline_person_job_change",
        "deepline_phone_find",
    ]
    assert all(t.method == "POST" and t.has_body for t in by.values())
    assert by["deepline_email_work"].input_schema["required"] == [
        "domain",
        "first_name",
        "last_name",
    ]
    assert by["deepline_ads_search"].input_schema["required"] == ["platform"]
    assert by["deepline_ads_search"].input_schema["properties"]["platform"]["enum"] == [
        "facebook",
        "google",
        "linkedin",
        "tiktok",
    ]
    assert by["deepline_ads_search"].input_schema["properties"]["media_type"]["enum"] == [
        "text",
        "image",
        "video",
    ]
    roles = by["deepline_contacts_by_role"].input_schema["properties"]["roles"]
    assert roles["minItems"] == 1 and roles["items"] == {"type": "string", "minLength": 1}
    limit = by["deepline_contacts_by_role"].input_schema["properties"]["limit"]
    assert limit["minimum"] == 1 and limit["maximum"] == 100
    # vendor prices surface via x-payment-info, rendered as prose
    assert "Price: $0.04 per call (vendor spec)." in by["deepline_email_work"].description
    assert "Price: $0.50 per call (vendor spec)." in by["deepline_phone_find"].description
    # conf pins additionalProperties:false like the old hand-written schemas
    assert all(
        t.input_schema.get("additionalProperties") is False for t in by.values()
    )


def test_pdl_conf_replaces_handcoded_launcher():
    by = {t.name: t for t in _tools_from_conf("pdl")}
    assert sorted(by) == [
        "pdl_company_enrich",
        "pdl_company_search",
        "pdl_person_enrich",
        "pdl_person_search",
    ]
    assert all(t.method == "POST" and t.has_body for t in by.values())
    # overrides restore the semantics the spec lacks
    assert "free on no match" in by["pdl_person_enrich"].description
    assert "$0.28 per match" in by["pdl_person_enrich"].description
    assert "free when the result set is empty" in by["pdl_person_search"].description
    assert "Recommended size 1-5" in by["pdl_person_search"].description
    size = by["pdl_person_search"].input_schema["properties"]["size"]
    assert size["minimum"] == 1 and size["maximum"] == 100
    assert size["description"] == "Batch size 1-100 (default 1). Price scales with size."
    ml = by["pdl_person_enrich"].input_schema["properties"]["min_likelihood"]
    assert ml["minimum"] == 1 and ml["maximum"] == 10
    assert "Higher = fewer, better matches" in ml["description"]
    dataset = by["pdl_person_search"].input_schema["properties"]["dataset"]
    assert "resume" in dataset["description"] and "all" in dataset["description"]
    assert all(
        t.input_schema.get("additionalProperties") is False for t in by.values()
    )


def test_kronos_conf_pins_paid_and_free_routes():
    by = {t.name: t for t in _tools_from_conf("kronos")}
    # the spec's CORS OPTIONS operations must not become tools
    assert all(t.method == "GET" for t in by.values())
    # include ["/api/v1"] anchors strip the v1 path artifact from names
    assert sorted(by) == [
        "kronos_alerts_asset",
        "kronos_alerts_btc",
        "kronos_briefing",
        "kronos_cex_premium_asset",
        "kronos_digest_asset",
        "kronos_fear_greed",
        "kronos_forecast_ada",
        "kronos_forecast_bnb",
        "kronos_forecast_btc",
        "kronos_forecast_doge",
        "kronos_forecast_eth",
        "kronos_forecast_ledger",
        "kronos_forecast_near",
        "kronos_forecast_path_asset",
        "kronos_forecast_sol",
        "kronos_forecast_xrp",
        "kronos_forward_vol_asset",
        "kronos_funding_arb_asset",
        "kronos_funding_extremes",
        "kronos_gex_asset",
        "kronos_implied_prob_asset",
        "kronos_liquidations_asset",
        "kronos_macro",
        "kronos_market_pulse",
        "kronos_ohlc_asset",
        "kronos_options_iv_asset",
        "kronos_overview",
        "kronos_price_asset",
        "kronos_scan",
        "kronos_signals_asset",
        "kronos_snapshot_asset",
        "kronos_stablecoins",
        "kronos_target_prob_asset",
        "kronos_track_record",
        "kronos_trade_preflight",
        "kronos_volatility_asset",
    ]
    # vendor prose carries the price (rendered verbatim); the spec's x-x402
    # extension has the wrong shape for pricing_key, so the boot probe prices
    # these routes live instead — tests run probe-free and pin the prose
    assert "$0.05" in by["kronos_forecast_btc"].description
    assert "$0.10" in by["kronos_briefing"].description
    assert "$0.08" in by["kronos_snapshot_asset"].description
    # the one v1-scoped free route is documented as free via an override
    assert by["kronos_track_record"].description.startswith("Free — no payment required.")
    # asset path params stay routed into the path
    assert by["kronos_signals_asset"].param_routes["asset"] == "path"
    assert "BTC" in by["kronos_signals_asset"].description


def test_regimeshift_conf_pins_paid_data_and_free_clearinghouse():
    by = {t.name: t for t in _tools_from_conf("regimeshift")}
    assert sorted(by) == [
        "regimeshift_active_loans",
        "regimeshift_asset_asset_vrp",
        "regimeshift_intent_borrow",
        "regimeshift_intent_intent_id_match",
        "regimeshift_intent_intent_id_matches",
        "regimeshift_intent_lend",
        "regimeshift_intents_open",
        "regimeshift_liquidatable_loans",
        "regimeshift_loans_registry",
        "regimeshift_matches_recent",
        "regimeshift_rate_sofr_usd",
        "regimeshift_risk_max_ltv",
    ]
    # the three paid signals render the spec's x-payment-info price
    for name in (
        "regimeshift_rate_sofr_usd",
        "regimeshift_risk_max_ltv",
        "regimeshift_asset_asset_vrp",
    ):
        assert "Price: $0.001 per call (vendor spec)." in by[name].description
    # free clearinghouse routes say so via overrides
    for name in (
        "regimeshift_intent_lend",
        "regimeshift_intent_borrow",
        "regimeshift_intents_open",
        "regimeshift_matches_recent",
        "regimeshift_active_loans",
        "regimeshift_liquidatable_loans",
        "regimeshift_loans_registry",
        "regimeshift_intent_intent_id_match",
        "regimeshift_intent_intent_id_matches",
    ):
        assert "Free — no payment required" in by[name].description, name
    # intents POST inline bodies with real constraints
    lend = by["regimeshift_intent_lend"]
    assert lend.method == "POST" and lend.has_body
    amount = lend.input_schema["properties"]["amount"]
    assert amount["exclusiveMinimum"] == 0.0
    assert lend.input_schema["properties"]["asset"]["enum"] == ["USDC"]
    wallet = lend.input_schema["properties"]["wallet"]
    assert wallet["pattern"] == "^0x[a-fA-F0-9]{40}$"
    # the long-poll tool documents its wait/stream semantics
    match = by["regimeshift_intent_intent_id_match"]
    assert match.param_routes["wait"] == "query"
    assert "--timeout 320" in match.description
    assert "not usable through MCP" in match.description


def _dereffed_snapshot_ops():
    """Rebuild the concordance digest operations from the vendor spec snapshot.

    Mirrors the one-off derivation step (inline the FastAPI $ref request
    bodies, attach llms.txt prices as pricing blocks) so the committed digest
    is pinned against a refreshed snapshot.
    """
    spec = json.loads((FIXTURES / "concordance_openapi.json").read_text())
    comps = spec["components"]["schemas"]

    def deref(schema):
        if isinstance(schema, dict) and "$ref" in schema:
            return comps[schema["$ref"].split("/")[-1]]
        return schema

    prices = {
        "/tools/search": "0.01",
        "/tools/series": "0.01",
        "/tools/digest": "0.03",
        "/tools/company-brief": "0.05",
        "/tools/corroboration": "0.04",
        "/tools/timeline": "0.03",
    }
    paths = {}
    for path, amount in sorted(prices.items()):
        op = dict(spec["paths"][path]["post"])
        body = op["requestBody"]["content"]["application/json"]["schema"]
        op["requestBody"]["content"]["application/json"]["schema"] = deref(body)
        op["x-concordance-price"] = {"price": {"amount": amount, "currency": "USD"}}
        paths[path] = {"post": op}
    return build_operations({**spec, "paths": paths}, pricing_key="x-concordance-price")


def test_concordance_digest_conf_pins_paid_tools():
    conf = json.loads((CONF_DIR / "concordance.json").read_text())
    digest = json.loads((REPO_ROOT / conf["spec"]).read_text())
    # the digest stays faithful to the vendor snapshot (refs inlined, prices)
    assert digest["operations"] == _dereffed_snapshot_ops()
    by = {
        t.name: t
        for t in _tools_from_conf("concordance", spec=str(REPO_ROOT / conf["spec"]))
    }
    assert sorted(by) == [
        "concordance_company_brief",
        "concordance_corroboration",
        "concordance_digest",
        "concordance_help",
        "concordance_search",
        "concordance_series",
        "concordance_timeline",
    ]
    paid = [t for name, t in by.items() if name != "concordance_help"]
    assert all(t.method == "POST" and t.has_body for t in paid)
    assert all(t.input_schema.get("additionalProperties") is False for t in paid)
    # prices render from the digest's embedded pricing blocks (POSTs are
    # never probed, so without these blocks no price would surface); the
    # search/series overrides restate the price in prose instead
    for name, price in (
        ("concordance_digest", "$0.03"),
        ("concordance_timeline", "$0.03"),
        ("concordance_corroboration", "$0.04"),
        ("concordance_company_brief", "$0.05"),
    ):
        assert f"Price: {price} per call (vendor spec)." in by[name].description
    for name in ("concordance_search", "concordance_series"):
        assert "$0.01 per call." in by[name].description
    # the spec's bare summary stubs are overridden where the vendor spec has
    # no long description; the other four keep their vendor prose
    assert "Search Endpoint" not in by["concordance_search"].description
    assert "not a single-source wrapper" in by["concordance_search"].description
    assert "Series Endpoint" not in by["concordance_series"].description
    assert "Partial results" in by["concordance_company_brief"].description
    # inlined bodies keep real schemas: requireds and bounds survive
    assert by["concordance_search"].input_schema["required"] == ["query"]
    assert by["concordance_company_brief"].input_schema["required"] == ["company"]
    assert by["concordance_corroboration"].input_schema["required"] == ["claim"]
    assert sorted(by["concordance_series"].input_schema["required"]) == [
        "series_id",
        "source",
    ]
    limit = by["concordance_search"].input_schema["properties"]["limit"]
    assert limit["minimum"] == 1 and limit["maximum"] == 100 and limit["default"] == 20
    src = by["concordance_series"].input_schema["properties"]["source"]["description"]
    assert "fred" in src and "sec_edgar" in src


def test_agentfund_conf_pins_paid_tools():
    by = {t.name: t for t in _tools_from_conf("agentfund")}
    assert sorted(by) == [
        "agentfund_bls_cpi",
        "agentfund_edgar_13f_holdings",
        "agentfund_edgar_filings_feed",
        "agentfund_edgar_financials",
        "agentfund_edgar_full_text_search",
        "agentfund_edgar_insider_transactions",
        "agentfund_macro_energy",
        "agentfund_macro_gdp",
        "agentfund_macro_housing",
        "agentfund_macro_jobs",
        "agentfund_macro_pce",
        "agentfund_macro_release_calendar",
        "agentfund_macro_retail_sales",
        "agentfund_onchain_cross_chain_balances",
        "agentfund_onchain_gas",
        "agentfund_onchain_oracle_price",
        "agentfund_onchain_portfolio",
        "agentfund_onchain_token_balances",
        "agentfund_structured_json_repair",
        "agentfund_tabular_to_json",
        "agentfund_treasury_yield_curve",
    ]
    assert all(t.method == "POST" and t.has_body for t in by.values())
    assert all(t.input_schema.get("additionalProperties") is False for t in by.values())
    # every op publishes a correct-shape x-payment-info price, so prices
    # render from the vendor spec and boots make zero probe requests
    assert (
        "Price: $0.005 per call (vendor spec)."
        in by["agentfund_treasury_yield_curve"].description
    )
    assert "Price: $0.001 per call (vendor spec)." in by["agentfund_onchain_gas"].description
    assert "Price: $0.02 per call (vendor spec)." in by["agentfund_edgar_13f_holdings"].description
    assert "Price: $0.03 per call (vendor spec)." in by["agentfund_tabular_to_json"].description
    # vendor Args prose reaches the model whole; the conf carries no overrides
    assert "When to use" in by["agentfund_treasury_yield_curve"].description
    assert (
        by["agentfund_onchain_token_balances"].input_schema["required"] == ["addresses"]
    )
    assert by["agentfund_edgar_full_text_search"].input_schema["required"] == ["query"]
    assert by["agentfund_structured_json_repair"].input_schema["required"] == ["input"]
    fmt = by["agentfund_tabular_to_json"].input_schema["properties"]["format"]
    assert fmt["enum"] == ["auto", "csv", "tsv", "markdown"]


def test_otto_conf_pins_swarm_and_excludes_execution():
    by = {t.name: t for t in _tools_from_conf("otto")}
    assert sorted(by) == [
        "otto_13d_activist_watch",
        "otto_8k_material_events",
        "otto_analyst_ratings",
        "otto_base_ecosystem_news",
        "otto_base_season",
        "otto_chain_status",
        "otto_crypto_news",
        "otto_defi_analytics",
        "otto_dividend_intel",
        "otto_dns_lookup",
        "otto_domain_age",
        "otto_domain_report",
        "otto_earnings_calendar",
        "otto_equity_intel",
        "otto_equity_news",
        "otto_equity_smart_money",
        "otto_equity_smart_money_brief",
        "otto_feedback",
        "otto_filtered_news",
        "otto_financial_statements",
        "otto_form_144_filings",
        "otto_form_4_feed",
        "otto_funding_rates",
        "otto_fx_rates",
        "otto_gas_oracle",
        "otto_generate_meme_get",
        "otto_generate_meme_post",
        "otto_hl_transaction_history",
        "otto_holder_analytics",
        "otto_hyperliquid_account",
        "otto_hyperliquid_market",
        "otto_insider_trades",
        "otto_institutional_holdings",
        "otto_kol_sentiment",
        "otto_llm_research_get",
        "otto_llm_research_post",
        "otto_lp_intelligence",
        "otto_lp_pool_apr",
        "otto_macro_regime",
        "otto_market_risk",
        "otto_mega_report",
        "otto_meta_intelligence_get",
        "otto_meta_intelligence_post",
        "otto_news_recaps",
        "otto_pm_crypto",
        "otto_pm_markets",
        "otto_pm_movers",
        "otto_pm_search",
        "otto_pools_search",
        "otto_pools_trending",
        "otto_portfolio",
        "otto_protocol_revenue_leaders",
        "otto_rh_season",
        "otto_stablecoin_watch",
        "otto_stock_pools",
        "otto_supported_tokens",
        "otto_technical_signals",
        "otto_token_alpha",
        "otto_token_details",
        "otto_token_fundamentals",
        "otto_token_price",
        "otto_token_score",
        "otto_token_security",
        "otto_token_top_holders",
        "otto_tokenized_equities",
        "otto_tokenized_stock_movers",
        "otto_tradfi_data",
        "otto_transaction_history",
        "otto_trending_altcoins",
        "otto_tweet_search",
        "otto_twitter_summary",
        "otto_tx_explainer_get",
        "otto_tx_explainer_post",
        "otto_video_gen",
        "otto_wallet_holdings",
        "otto_weather",
        "otto_web_answer",
        "otto_web_extract",
        "otto_web_json_extract",
        "otto_web_links",
        "otto_web_search",
        "otto_whois_lookup",
        "otto_yield_alpha",
        "otto_yield_farming_active",
        "otto_yield_farming_historical",
        "otto_yield_markets",
        "otto_yield_recommendations",
    ]
    # fund-moving routes stay excluded: this server holds the wallet
    assert not {"otto_swap", "otto_bridge", "otto_deposit", "otto_withdraw",
                "otto_full_auto"} & set(by)
    # GETs carry query params, POSTs carry inline JSON bodies; multi-method
    # paths get _get/_post suffixes
    assert all((t.method == "POST") == t.has_body for t in by.values())
    assert by["otto_token_price"].method == "GET"
    assert by["otto_meta_intelligence_get"].method == "GET"
    assert by["otto_meta_intelligence_post"].method == "POST"
    assert by["otto_meta_intelligence_post"].has_body
    assert by["otto_tx_explainer_post"].has_body
    assert all(t.input_schema.get("additionalProperties") is False for t in by.values())
    # fixed prices render from the spec's x-payment-info
    assert (
        "Price: $0.001 per call (vendor spec)." in by["otto_crypto_news"].description
    )
    assert "Price: $0.05 per call (vendor spec)." in by["otto_mega_report"].description
    assert "Price: $0.009 per call (vendor spec)." in by["otto_web_search"].description
    # dynamic min/max pricing renders as a range
    assert (
        "Price: $0.46\u2013$4.60 per call, scaling with usage (vendor spec)."
        in by["otto_video_gen"].description
    )
    # where vendor prose already states the price, the rendered line dedupes
    assert "$0.001" in by["otto_token_price"].description
    assert "Price: $0.001 per call (vendor spec)." not in by["otto_token_price"].description
    # vendor usage prose reaches the model whole
    assert "Regenerated hourly" in by["otto_crypto_news"].description
    assert by["otto_token_price"].input_schema["required"] == ["token"]
    assert by["otto_meta_intelligence_post"].input_schema["required"] == ["ask"]
    assert by["otto_feedback"].input_schema["required"] == ["original_tx_hash"]


def test_straits_digest_conf_pins_tiers():
    conf = json.loads((CONF_DIR / "straits.json").read_text())
    digest = json.loads((REPO_ROOT / conf["spec"]).read_text())
    by = {
        t.name: t
        for t in _tools_from_conf("straits", spec=str(REPO_ROOT / conf["spec"]))
    }
    assert sorted(by) == [
        "straits_ais_gaps",
        "straits_brief",
        "straits_briefs",
        "straits_bunker",
        "straits_carriers",
        "straits_chokepoints",
        "straits_cushing",
        "straits_events",
        "straits_export",
        "straits_gas",
        "straits_help",
        "straits_history_feed",
        "straits_hormuz_flags",
        "straits_hormuz_risk_screen",
        "straits_index",
        "straits_insurance",
        "straits_iran_rate",
        "straits_jwc",
        "straits_markets",
        "straits_oil",
        "straits_pipelines",
        "straits_ports",
        "straits_risk_premium",
        "straits_sanctioned_vessels",
        "straits_since",
        "straits_snapshot",
        "straits_spr",
        "straits_stranded",
        "straits_tick",
        "straits_trade_impact",
        "straits_transits",
        "straits_transits_by_type",
        "straits_vessel_imo",
        "straits_vessels",
    ]
    # premium ops stay faithful to the vendor snapshot (the spec's only tag is
    # "Premium"; stream/webhook ops are dropped - session-token/secret auth,
    # not x402)
    spec = json.loads((FIXTURES / "straits_openapi.json").read_text())
    rebuilt = build_operations(spec, pricing_key="x-payment-info", include_tags=True)
    kept = {
        (op["method"], op["path"])
        for op in digest["operations"]
        if op["tags"] == ["Premium"]
    }
    assert kept == {
        (op["method"], op["path"])
        for op in rebuilt
        if op["tags"] == ["Premium"]
        and op["path"] != "/api/premium/stream"
        and op["path"] != "/api/premium/stream/session"
        and op["path"] != "/api/premium/webhooks"
        and op["path"] != "/api/premium/webhooks/{id}"
    }
    # premium prices render from the embedded x-payment-info blocks
    for name, price in (
        ("straits_brief", "$0.01"),
        ("straits_export", "$0.25"),
        ("straits_history_feed", "$0.01"),
        ("straits_risk_premium", "$0.05"),
        ("straits_snapshot", "$0.02"),
        ("straits_vessels", "$0.02"),
    ):
        assert f"Price: {price} per call (vendor spec)." in by[name].description
    # free ops answer 200 unpaid, so the probe would mislabel them; the 23
    # overrides restate semantics under a free marker
    free_names = [n for n, t in by.items() if n != "straits_help" and "Free —" in t.description]
    assert len(free_names) == 23
    assert not any("Paid per call via x402" in t.description for t in by.values())
    # tier selection works through the tags the digest carries
    pre, _rest = _generic_parser().parse_known_args(["--config", str(CONF_DIR / "straits.json")])
    cfg = resolve_config(pre)
    ops, _title, _server = load_source(str(REPO_ROOT / conf["spec"]), cfg.pricing_key)
    assert len(build_tools(dataclasses.replace(cfg, tags=("Free",)), ops,
                           base_url="https://straits.live", prefix="straits",
                           probe=False)) == 24  # 23 free + help
    assert len(build_tools(dataclasses.replace(cfg, tags=("Premium",)), ops,
                           base_url="https://straits.live", prefix="straits",
                           probe=False)) == 11  # 10 premium + help
    # templated paths keep path routing and enums
    hist = by["straits_history_feed"]
    assert hist.param_routes["feed"] == "path" and hist.param_routes["days"] == "query"
    assert hist.input_schema["properties"]["feed"]["enum"] == [
        "index-5min", "transits", "stranded", "ais-gaps",
    ]
    vessel = by["straits_vessel_imo"]
    assert vessel.param_routes["imo"] == "path"
    assert vessel.input_schema["properties"]["imo"]["pattern"] == "^\\d{7}$"
    assert by["straits_index"].input_schema["properties"]["history"]["enum"] == [
        "24h", "7d", "30d",
    ]


def test_lonestar_conf_mirrors_launcher_catalog_from_unified_spec():
    # the conf reads the vendor's ONE unified spec (the launcher digest is
    # built from the same file); tools must come out gateway-routed with the
    # same names, and the vendor's own free markers must render as free
    conf = json.loads((CONF_DIR / "lonestar.json").read_text())
    by = {
        t.name: t
        for t in _tools_from_conf("lonestar", spec=str(FIXTURES / "lonestar_unified_openapi.json"))
    }
    # live vendor spec: pin structurally, not by full name list (it churns)
    assert 90 <= len(by) <= 115  # 96 tools + help at time of writing
    for excluded in (
        "lonestar_lease_portfolio",
        "lonestar_rattler_demo",
        "lonestar_agri_preview",
        "lonestar_token_report_text",
        "lonestar_crownblock_feed",
    ):
        assert excluded not in by
    # gateway routing: relative paths joined onto the /api base (verified
    # payment-equivalent to the per-service subdomains)
    assert by["lonestar_aero_pool"].path == "/aero/pool"
    assert "Price: $0.15 per call (vendor spec)." in by["lonestar_token_report"].description
    assert by["lonestar_token_report"].input_schema["required"] == ["address"]
    # the vendor's authMode:free markers render as free, not as a JSON dump
    free = [n for n, t in by.items() if "Free — no payment required (vendor spec)." in t.description]
    assert {"lonestar_rattler_fix", "lonestar_crownblock_history", "lonestar_doc_report"} <= set(free)
    assert not any("Pricing: {" in t.description for t in by.values())
    # $ref audit bodies deref through the shared flatteners
    audit = by["lonestar_rattler_audit"]
    assert audit.method == "POST" and audit.has_body
    assert "github_url" in audit.input_schema["properties"]
    # floyd's free pollers are gone from the vendor catalog; it now only
    # sells the agent itself
    assert "Price: $0.50 per call (vendor spec)." in by["lonestar_floyd_hire"].description
    assert "lonestar_floyd_status_task_id" not in by


def test_glassnode_digest_conf_pins_gateway_tools():
    conf = json.loads((CONF_DIR / "glassnode" / "glassnode.json").read_text())
    by = {
        t.name: t
        for t in _tools_from_conf(
            "glassnode/glassnode", spec=str(REPO_ROOT / conf["spec"])
        )
    }
    assert sorted(by) == [
        "glassnode_help",
        "glassnode_metadata_assets",
        "glassnode_metadata_metric",
        "glassnode_metadata_metrics",
        "glassnode_metrics_category_metric",
    ]
    metric = by["glassnode_metrics_category_metric"]
    assert metric.method == "GET"
    assert metric.path == "/v1/metrics/{category}/{metric}"
    assert sorted(metric.input_schema["required"]) == ["a", "category", "metric"]
    assert metric.param_routes["category"] == "path"
    assert metric.param_routes["metric"] == "path"
    assert metric.param_routes["a"] == "query"
    assert metric.param_routes["i"] == "query"
    # two-tier pricing from the hand-authored digest; the digest prose states
    # each price itself, so the renderer's "(vendor spec)" line dedupes away
    assert "$0.05 USDC on Base per call" in metric.description
    assert "$0.01 USDC on Base per call" in by["glassnode_metadata_metrics"].description
    assert "$0.01 USDC on Base per call" in by["glassnode_metadata_metric"].description
    assert "$0.01 USDC on Base per call" in by["glassnode_metadata_assets"].description
    # discovery flow is described on the tool itself
    assert "glassnode_metadata_metrics" in metric.description


def test_brazilayer_conf_pins_registry_tools_and_free_overrides():
    by = {t.name: t for t in _tools_from_conf("brazilayer")}
    assert len(by) == 53
    assert all(t.method == "GET" and not t.has_body for t in by.values())
    # every paid route states a dollar price in its final description (37 of
    # 53: vendor summary prose, plus one override where the vendor's longer
    # description dropped the price its summary had); the 16 free routes are
    # marked via overrides and never mislabeled paid
    paid = [t for t in by.values() if "$" in t.description]
    free = [t for t in by.values() if t.description.startswith("Free — no payment required.")]
    assert len(paid) == 37
    assert len(free) == 16
    assert len(paid) + len(free) == len(by)
    # shared $ref path parameters (components/parameters/cnpj) resolve into
    # required path args — the digest flattener derefs op-level $ref params
    cnpj_routes = [
        "brazilayer_bcb_instituicao_cnpj",
        "brazilayer_cnpj_empresa_cnpj",
        "brazilayer_cnpj_empresa_cnpj_completo",
        "brazilayer_cnpj_empresa_cnpj_socios",
        "brazilayer_cnpj_situacao_cnpj",
        "brazilayer_desmatamento_fornecedor_cnpj",
        "brazilayer_empresa_enriquecer_cnpj",
        "brazilayer_fundos_carteira_cnpj",
        "brazilayer_fundos_consulta_cnpj",
        "brazilayer_integridade_consulta_cnpj",
    ]
    for name in cnpj_routes:
        assert by[name].param_routes["cnpj"] == "path"
        assert by[name].input_schema["required"] == ["cnpj"]
    # the other templated routes keep their inline path params
    assert by["brazilayer_ferramentas_cnpj_valor"].param_routes["valor"] == "path"
    assert by["brazilayer_tributos_ncm_codigo"].param_routes["codigo"] == "path"
    assert by["brazilayer_desmatamento_municipio_ibge"].param_routes["ibge"] == "path"
    free_names = {t.name for t in free}
    assert "brazilayer_health" in free_names
    assert "brazilayer_ferramentas_cnpj_valor" in free_names
    # search filters and the vendor's truncated nome description (spec drift
    # junk keys) are patched via overrides; busca's "at least one filter" is
    # semantic (schema-level nome is optional), socio/busca requires nome
    busca = by["brazilayer_cnpj_busca"]
    assert busca.input_schema.get("required") is None
    assert "accent-insensitive" in busca.input_schema["properties"]["nome"]["description"]
    socio = by["brazilayer_cnpj_socio_busca"]
    assert socio.input_schema["required"] == ["nome"]
    assert "accent-insensitive" in socio.input_schema["properties"]["nome"]["description"]
    # bundle tool keeps its $ prose price
    assert "$0.09" in by["brazilayer_pacotes_brasil"].description


def test_genuinegood_conf_pins_grant_tools_and_excludes_pass():
    by = {t.name: t for t in _tools_from_conf("genuinegood")}
    assert sorted(by) == [
        "genuinegood_brief_get",
        "genuinegood_brief_post",
        "genuinegood_detail_get",
        "genuinegood_detail_post",
        "genuinegood_fit_get",
        "genuinegood_fit_post",
        "genuinegood_help",
        "genuinegood_preflight_get",
        "genuinegood_preflight_post",
        "genuinegood_search_get",
        "genuinegood_search_post",
    ]
    # the five data routes ship GET+POST twins (shared paths -> method
    # suffixes); the $15 30-day bearer pass is excluded on purpose - the pass
    # buys Authorization-header access this server cannot attach to vendor
    # calls, so activating it would be $15 for a token no tool can use
    assert "genuinegood_pass" not in by
    assert by["genuinegood_search_post"].method == "POST"
    assert by["genuinegood_search_post"].has_body
    assert by["genuinegood_search_get"].method == "GET"
    assert not by["genuinegood_search_get"].has_body
    # prices render from the spec's x-payment-info; vendor prose states no
    # dollar amounts, so the rendered line survives the dedupe and appears
    # exactly once per tool
    assert "Price: $0.05 per call (vendor spec)." in by["genuinegood_search_post"].description
    assert "Price: $0.08 per call (vendor spec)." in by["genuinegood_detail_get"].description
    assert "Price: $0.20 per call (vendor spec)." in by["genuinegood_fit_post"].description
    assert "Price: $0.50 per call (vendor spec)." in by["genuinegood_brief_post"].description
    assert "Price: $5.00 per call (vendor spec)." in by["genuinegood_preflight_get"].description
    assert "$15" not in "".join(t.description for t in by.values())
    # POST bodies keep the vendor's real constraints
    assert by["genuinegood_detail_post"].input_schema["required"] == ["opportunityId"]
    assert by["genuinegood_fit_post"].input_schema["required"] == ["keywords", "mission"]
    ids = by["genuinegood_preflight_post"].input_schema["properties"]["opportunityIds"]
    assert ids["maxItems"] == 5
    statuses = by["genuinegood_search_post"].input_schema["properties"]["statuses"]
    assert statuses["items"]["enum"] == ["posted", "forecasted", "closed", "archived"]
    # GET twins route the same inputs as query parameters
    assert by["genuinegood_detail_get"].param_routes["opportunityId"] == "query"
    assert by["genuinegood_detail_get"].input_schema["required"] == ["opportunityId"]
    assert all(t.input_schema.get("additionalProperties") is False for t in by.values())


def _locus_builder():
    import importlib.util

    path = REPO_ROOT / "scripts" / "build_locus_digest.py"
    module_spec = importlib.util.spec_from_file_location("build_locus_digest", path)
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    return module


def test_locus_digest_rebuild_is_byte_identical():
    module = _locus_builder()
    snapshot = json.loads((FIXTURES / "locus_openapi.json").read_text())
    rebuilt = module.build_digest(snapshot)
    committed = (REPO_ROOT / "confs" / "locus" / "locus_digest.json").read_text()
    assert json.dumps(rebuilt, separators=(",", ":"), sort_keys=True) == committed


def test_locus_digest_conf_pins_gateway_and_paid_tools():
    conf = json.loads((CONF_DIR / "locus.json").read_text())
    by = {
        t.name: t
        for t in _tools_from_conf("locus", spec=str(REPO_ROOT / conf["spec"]))
    }
    assert len(by) == 111
    # the free tier rides a two-tool gateway (the vendor's own remote-MCP
    # shape): catalog + wrapped executor; both free, never mislabeled paid
    call = by["locus_call"]
    assert call.method == "POST" and call.path == "/tools/call"
    name = call.input_schema["properties"]["name"]
    assert len(name["enum"]) == 114 and "locus_coverage_check" in name["enum"]
    assert call.input_schema["required"] == ["arguments", "name"]
    assert call.param_routes["name"] == "body"
    assert call.param_routes["arguments"] == "body"
    assert "Free — no payment required ($0)" in call.description
    assert "Paid per call" not in call.description
    listing = by["locus_list"]
    assert listing.method == "GET" and listing.path == "/tools/list"
    assert listing.param_routes["compact"] == "query"
    assert listing.param_routes["category"] == "query"
    assert "Free — no payment required ($0)" in listing.description
    # paid hyphen routes: prices embedded from x-payment-info.price, flat
    # POST bodies; no boot probes (probe_pricing is false in the conf)
    paid = [t for n, t in by.items() if n.startswith("locus_locus_")]
    assert len(paid) == 108
    assert all(t.method == "POST" and t.has_body for t in paid)
    assert all("$" in t.description for t in paid)
    brief = by["locus_locus_local_policy_brief"]
    assert brief.path == "/api/locus-local-policy-brief"
    assert "Price: $0.07 per call (vendor spec)." in brief.description
    assert brief.input_schema["required"] == ["intent", "maxSignals", "place"]
    assert (
        by["locus_locus_place_report"].path == "/api/locus-place-report"
        and "$0.05" in by["locus_locus_place_report"].description
    )
    # pollable-job routes are excluded on purpose (pay with no way to poll);
    # the sync listing-claim batch route stays mounted
    assert {
        "locus_locus_place_report_batch",
        "locus_locus_record_batch",
        "locus_locus_property_update",
    }.isdisjoint(by)
    assert "locus_locus_listing_claim_batch" in by
    assert "locus_help" in by  # help_url serves the vendor llms.txt
