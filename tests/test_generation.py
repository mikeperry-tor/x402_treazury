from x402_mcp.config import PaymentConfig
from x402_mcp.socialfetch import PLATFORMS, build_tools


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


def test_socialfetch_schemas_openai_safe():
    """OpenAI tool-schema validation rejects draft-04 boolean
    exclusiveMinimum/Maximum; no served schema may carry one (litellm
    'True is not of type number')."""

    def walk(node):
        if isinstance(node, dict):
            for k, v in node.items():
                if k in ("exclusiveMinimum", "exclusiveMaximum"):
                    assert not isinstance(v, bool), f"boolean {k} at {node}"
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    for tool in build_tools(list(PLATFORMS)):
        walk(tool.input_schema)


def test_spend_cap_atomic():
    assert PaymentConfig(None, None, 0.5).max_price_atomic == 500_000
    assert PaymentConfig(None, None, None).max_price_atomic is None
