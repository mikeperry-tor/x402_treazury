"""Offline regression tests for the LoneStarOracle launcher.

Runs against the committed digest (src/x402_mcp/data/
lonestar_openapi_digest.json) — no network, no wallet. Pins group
resolution, tool naming, absolute per-service URLs, pricing rendering
(including vendor-marked free routes), and $ref-inlined POST bodies.
If the vendor catalog drifts, regenerate the digest with
scripts/build_lonestar_digest.py (single unified-spec fetch) and re-check.
"""

from __future__ import annotations

import pytest

from x402_mcp.lonestar import (
    ALL_SERVICES,
    GROUPS,
    build_tools,
    load_digest,
    resolve_services,
    service_url,
)


def test_digest_covers_whole_catalog_and_prices_everything():
    digest = load_digest()
    ops = digest["operations"]
    services = {o["service"] for o in ops}
    assert services == set(ALL_SERVICES)
    # every op is a root-level route on its own subdomain pattern
    for o in ops:
        assert o["path"].startswith("/") and "{" not in o["service"]
        assert o["method"] in ("get", "post")
    # the unified spec classifies every op (authMode payment|free); nothing
    # ships unpriced anymore — the former rattler /fix hole is closed
    unpriced = [(o["service"], o["path"]) for o in ops if o["pricing"] is None]
    assert unpriced == []


def test_resolve_services_expands_groups_services_and_all():
    assert resolve_services(["crypto"]) == GROUPS["crypto"]
    assert resolve_services(["token", "rates"]) == ["token", "rates"]
    # groups + services mix, order-preserving, deduplicated
    assert resolve_services(["rates", "macro", "rates"]) == ["rates", "macro"]
    assert resolve_services(["all"]) == ALL_SERVICES
    with pytest.raises(SystemExit):
        resolve_services(["nope"])


def test_group_catalog_matches_docs_site_structure():
    # six catalog groups, no service in two groups, no phantoms, and group
    # keys never shadow a service slug (both share the --groups namespace)
    seen = [s for services in GROUPS.values() for s in services]
    assert len(seen) == len(set(seen)) == len(ALL_SERVICES)
    assert sorted(GROUPS) == [
        "audits",
        "crypto",
        "equities",
        "govnews",
        "real-economy",
        "risk",
    ]
    assert not (set(GROUPS) & set(ALL_SERVICES))


def test_build_tools_for_single_service_pins_naming_url_and_price():
    tools = {t.name: t for t in build_tools(["rates"])}
    assert sorted(tools) == ["lonestar_rates_rates"]
    tool = tools["lonestar_rates_rates"]
    assert tool.method == "GET"
    # absolute per-service URL: one server, many subdomains
    assert tool.path == "https://rates.lonestaroracle.xyz/rates"
    assert "Price: $0.05 per call (vendor spec)." in tool.description
    assert not tool.has_body


def test_build_tools_crownblock_pins_paid_reports_and_free_history():
    tools = {t.name: t for t in build_tools(["crownblock"])}
    assert sorted(tools) == [
        "lonestar_crownblock_history",
        "lonestar_crownblock_region_name",
        "lonestar_crownblock_report",
    ]
    assert "Free — no payment required." in tools["lonestar_crownblock_history"].description
    for name in ("lonestar_crownblock_report", "lonestar_crownblock_region_name"):
        assert "Price: $1.00 per call (vendor spec)." in tools[name].description
    assert tools["lonestar_crownblock_region_name"].param_routes["name"] == "path"


def test_build_tools_spec_priced_routes_and_free_fix():
    by_name = {t.name: t for t in build_tools(["ta"])}
    assert sorted(by_name) == ["lonestar_ta_analyze", "lonestar_ta_scan"]
    assert "Price: $0.05 per call (vendor spec)." in by_name["lonestar_ta_analyze"].description
    assert "Price: $0.05 per call (vendor spec)." in by_name["lonestar_ta_scan"].description
    # rattler /fix: the vendor now marks it free in the unified spec (it used
    # to be the one unpriced route rendering the generic paid line)
    rattler = {t.name: t for t in build_tools(["rattler"])}
    assert sorted(rattler) == ["lonestar_rattler_audit", "lonestar_rattler_fix"]
    assert "Free — no payment required." in rattler["lonestar_rattler_fix"].description
    assert "Price: $2.00 per call (vendor spec)." in rattler["lonestar_rattler_audit"].description


def test_audit_post_bodies_are_ref_inlined_with_params():
    # the vendor specs express audit request bodies as $ref; the digest
    # builder inlines them, so the flattened tool has real parameters
    rattler = {t.name: t for t in build_tools(["rattler"])}
    audit = rattler["lonestar_rattler_audit"]
    assert audit.method == "POST" and audit.has_body
    props = audit.input_schema["properties"]
    assert "github_url" in props and "source" in props
    assert audit.param_routes["github_url"] == "body"
    assert audit.path == "https://rattler.lonestaroracle.xyz/audit"


def test_build_tools_across_groups_merges_subdomains_and_names():
    tools = build_tools(["token", "sanctions", "macro"])
    by = {t.name: t for t in tools}
    assert sorted(by) == ["lonestar_macro_macro", "lonestar_sanctions_screen", "lonestar_token_report"]
    assert by["lonestar_token_report"].path == "https://token.lonestaroracle.xyz/report"
    assert by["lonestar_sanctions_screen"].path == service_url("sanctions") + "/screen"
    assert "Price: $0.15 per call (vendor spec)." in by["lonestar_token_report"].description
    assert by["lonestar_token_report"].input_schema["required"] == ["address"]


def test_free_and_oddly_priced_routes_render_honestly():
    by = {t.name: t for t in build_tools(resolve_services(["govnews"]))}
    # free routes: doc report; floyd now only sells its agent (hire, $0.50) —
    # the old free status/bounty pollers are gone from the vendor catalog
    assert "Free — no payment required." in by["lonestar_doc_report"].description
    assert "Price: $0.50 per call (vendor spec)." in by["lonestar_floyd_hire"].description
    assert by["lonestar_floyd_hire"].method == "POST" and by["lonestar_floyd_hire"].has_body
    # read /read is $0.005 — rendered verbatim, never rounded to a cent
    assert "Price: $0.005 per call (vendor spec)." in by["lonestar_read_read"].description


def test_no_duplicate_tool_names_across_everything():
    tools = build_tools(resolve_services(["all"]))
    names = [t.name for t in tools]
    assert len(names) == len(set(names))
    # and the full catalog builds a sane server size
    assert 90 <= len(tools) <= 120
