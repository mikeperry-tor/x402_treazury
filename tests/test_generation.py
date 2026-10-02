from x402_mcp.config import PaymentConfig
from x402_mcp.deepline import TOOLS as DL
from x402_mcp.pdl import TOOLS as PDL
from x402_mcp.socialfetch import PLATFORMS, build_tools


def test_pdl_tools():
    names = {t.name for t in PDL}
    assert names == {
        "pdl_person_enrich",
        "pdl_company_enrich",
        "pdl_person_search",
        "pdl_company_search",
    }
    assert all(t.method == "POST" and t.has_body for t in PDL)


def test_deepline_tools_and_required():
    assert len(DL) == 11
    by = {t.name: t for t in DL}
    assert by["deepline_email_work"].input_schema["required"] == [
        "first_name",
        "last_name",
        "domain",
    ]
    assert by["deepline_ads_search"].input_schema["properties"]["platform"]["enum"] == [
        "facebook",
        "google",
        "linkedin",
        "tiktok",
    ]


def test_socialfetch_generation_and_routing():
    tools = build_tools(list(PLATFORMS))
    assert len(tools) > 200
    by = {t.name: t for t in tools}

    t = by["tiktok_profiles_handle"]
    assert t.path_params == ["handle"]
    assert t.param_routes["handle"] == "path"
    assert not t.has_body

    t = by["web_extract"]
    assert t.has_body
    assert set(t.param_routes.values()) == {"body"}

    names = set(by)
    assert not any(n.startswith("whoami") or n.startswith("balance") or n == "ask" for n in names)
    assert not any("monitors" in n or "webhook" in n for n in names)
    assert "linkedin_v1_profiles" in names and "linkedin_v2_profiles" in names


def test_spend_cap_atomic():
    assert PaymentConfig(None, None, 0.5).max_price_atomic == 500_000
    assert PaymentConfig(None, None, None).max_price_atomic is None
