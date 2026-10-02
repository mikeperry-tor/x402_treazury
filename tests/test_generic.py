"""Offline tests for the generic launcher and the shared digest module.

No external network, no wallet: probing runs against a localhost
http.server, config fixtures use tmp_path files.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from x402_mcp.config import ConfigError
from x402_mcp.digest import build_operations, sanitize_schema
from x402_mcp.generic import (
    GenericConfig,
    PricingCache,
    _generic_parser,
    _pricing_line,
    _tool_from_op,
    apply_overrides,
    assign_names,
    build_tools,
    default_prefix,
    load_source,
    match_prefix,
    resolve_config,
    select_ops,
)

REPO_ROOT = Path(__file__).resolve().parents[1]

LONG_DESC = "word " * 100  # 500 chars -> trimmed to 400 by trim_desc

USDC_BASE = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"


def _op(path: str, method: str = "get", **kw) -> dict:
    op = {
        "path": path,
        "method": method,
        "summary": None,
        "description": None,
        "pricing": None,
        "params": [],
        "body": None,
    }
    op.update(kw)
    return op


@pytest.fixture(autouse=True)
def _clean_generic_env(monkeypatch):
    for name in list(os.environ):
        if name.startswith("X402_MCP_GENERIC_"):
            monkeypatch.delenv(name)


def _cli(**kw) -> argparse.Namespace:
    defaults = {
        "config": None,
        "spec": None,
        "name": None,
        "prefix": None,
        "include": None,
        "exclude": None,
        "tags": None,
        "exclude_tags": None,
        "pricing_key": None,
        "probe": None,
        "probe_ttl": None,
        "probe_max_endpoints": None,
    }
    defaults.update(kw)
    return argparse.Namespace(**defaults)


def _write_spec(tmp_path, spec) -> Path:
    p = tmp_path / "spec.json"
    p.write_text(json.dumps(spec))
    return p


MINI_SPEC = {
    "openapi": "3.1.0",
    "info": {"title": "Example API", "version": "2.0"},
    "servers": [{"url": "https://api.example.dev"}],
    "paths": {
        "/search": {
            "get": {
                "summary": "Search things",
                "description": LONG_DESC,
                "x-example-credits": {"per_call": 1},
                "parameters": [
                    {
                        "name": "q",
                        "in": "query",
                        "required": True,
                        "description": "Query text",
                        "schema": {"type": "string", "maxLength": 100},
                    }
                ],
            }
        },
        "/things": {
            "post": {
                "summary": "Create a thing",
                "requestBody": {
                    "required": True,
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "size": {
                                        "type": "integer",
                                        "maximum": 100,
                                        "description": "Results per page",
                                    }
                                },
                                "required": ["size"],
                            }
                        }
                    },
                },
            }
        },
        "/things/{id}": {
            "get": {
                "summary": "Get a thing",
                "parameters": [
                    {
                        "name": "id",
                        "in": "path",
                        "required": True,
                        "schema": {"type": "string"},
                    },
                    {
                        "name": "X-Key",
                        "in": "header",
                        "schema": {"type": "string"},
                    },
                ],
            }
        },
    },
}


# --------------------------------------------------------------------------
# digest.build_operations golden (regression guard for the script extraction)
# --------------------------------------------------------------------------


def test_build_operations_golden():
    # trim=True pins the committed-digest artifact behavior (the script builds
    # with trimming for compactness); tool generation builds with trim=False.
    ops = build_operations(MINI_SPEC, pricing_key="x-example-credits", trim=True)
    assert [(o["path"], o["method"]) for o in ops] == [
        ("/search", "get"),
        ("/things", "post"),
        ("/things/{id}", "get"),
    ]
    by_path = {o["path"]: o for o in ops}

    search = by_path["/search"]
    assert len(search["description"]) == 400 and search["description"].endswith("…")
    assert search["pricing"] == {"per_call": 1}
    assert search["params"] == [
        {
            "name": "q",
            "in": "query",
            "required": True,
            "description": "Query text",
            "schema": {"type": "string", "maxLength": 100},
        }
    ]
    assert search["body"] is None

    things = by_path["/things"]
    assert things["pricing"] is None  # key configured but op lacks the extension
    assert things["body"] == {
        "required": True,
        "schema": {
            "type": "object",
            "properties": {
                "size": {
                    "type": "integer",
                    "maximum": 100,
                    "description": "Results per page",
                }
            },
            "required": ["size"],
        },
    }

    detail = by_path["/things/{id}"]
    # digest keeps header params; tool generation (not the digest) drops them
    assert [p["in"] for p in detail["params"]] == ["path", "header"]


def test_build_operations_default_pricing_key():
    ops = build_operations(MINI_SPEC)
    assert all(o["pricing"] is None for o in ops)


def test_build_operations_default_keeps_vendor_text_whole():
    """Tool-generation paths build with trim=False (the default): cutting
    vendor descriptions/arg text at 400 chars loses model instructions."""
    (search,) = [o for o in build_operations(MINI_SPEC) if o["path"] == "/search"]
    assert search["description"] == LONG_DESC  # 500 chars, no ellipsis

    spec = {
        "paths": {
            "/a": {
                "get": {
                    "parameters": [
                        {
                            "name": "q",
                            "in": "query",
                            "description": LONG_DESC,
                            "schema": {"type": "string"},
                        }
                    ]
                }
            },
            "/b": {
                "get": {
                    "parameters": [
                        {
                            "name": "q",
                            "in": "query",
                            "schema": {"type": "string", "description": LONG_DESC},
                        }
                    ]
                }
            },
        }
    }
    ops = build_operations(spec)
    by_path = {o["path"]: o for o in ops}
    # identical param and schema descriptions dedupe down to the param field
    assert by_path["/a"]["params"][0]["description"] == LONG_DESC
    assert "description" not in by_path["/a"]["params"][0]["schema"]
    # schema-only descriptions surface on the param entry (the dedup moves,
    # never drops, the text) — whole, not trimmed
    assert by_path["/b"]["params"][0]["description"] == LONG_DESC


def test_draft04_boolean_exclusive_bounds_become_numeric():
    """OpenAI tool-schema validation rejects draft-04 booleans; the digest
    builder and every tool-schema path must emit numeric bounds."""
    spec = {
        "paths": {
            "/a": {
                "get": {
                    "parameters": [
                        # true + numeric sibling -> numeric exclusiveMinimum
                        {"name": "w", "in": "query",
                         "schema": {"type": "integer", "minimum": 0, "exclusiveMinimum": True}},
                        # false -> redundant, dropped; plain minimum stays
                        {"name": "x", "in": "query",
                         "schema": {"type": "integer", "minimum": 5, "exclusiveMinimum": False}},
                        # numeric (already 2020-12) -> passes through
                        {"name": "y", "in": "query",
                         "schema": {"type": "integer", "exclusiveMaximum": 10}},
                        # bare true without a numeric sibling -> dropped
                        {"name": "z", "in": "query",
                         "schema": {"type": "integer", "exclusiveMinimum": True}},
                    ]
                }
            }
        }
    }
    (op,) = build_operations(spec)
    schemas = {p["name"]: p["schema"] for p in op["params"]}
    assert schemas["w"] == {"type": "integer", "minimum": 0, "exclusiveMinimum": 0}
    assert schemas["x"] == {"type": "integer", "minimum": 5}
    assert schemas["y"] == {"type": "integer", "exclusiveMaximum": 10}
    assert schemas["z"] == {"type": "integer"}
    # nested positions are converted too
    nested = {"properties": {"id": {"minimum": 1, "exclusiveMinimum": True}},
              "items": {"exclusiveMaximum": False},
              "oneOf": [{"exclusiveMinimum": True, "minimum": 2}]}
    clean = sanitize_schema(nested)
    assert clean["properties"]["id"] == {"minimum": 1, "exclusiveMinimum": 1}
    assert clean["items"] == {}
    assert clean["oneOf"] == [{"exclusiveMinimum": 2, "minimum": 2}]


def test_flatten_sanitizes_digest_passthrough_schemas():
    """Ops from a pre-built digest bypass minify_schema; the flatten step must
    still strip draft-04 boolean exclusive bounds (old committed digests)."""
    op = _op(
        "/things/{id}",
        params=[
            {"name": "id", "in": "path", "required": True,
             "schema": {"type": "integer", "minimum": 0, "exclusiveMinimum": True}},
        ],
    )
    tool = _tool_from_op("ex_thing", op, "Paid per call via x402.")
    assert tool.input_schema["properties"]["id"] == {
        "type": "integer",
        "minimum": 0,
        "exclusiveMinimum": 0,
    }


def test_build_operations_resolves_local_refs():
    spec = {
        "openapi": "3.1.0",
        "paths": {
            "/a": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/Body"},
            }}}}},
            "/b": {"get": {"parameters": [{
                "name": "q", "in": "query", "required": True,
                "schema": {"$ref": "#/components/schemas/Query"},
            }]}},
            "/c": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/Cyclic"},
            }}}}},
            "/d": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "https://elsewhere.example/x.json"},
            }}}}},
            "/e": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "#/components/missing/Body"},
            }}}}},
        },
        "components": {"schemas": {
            "Body": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "opt": {"$ref": "#/components/schemas/Query"}},
                "required": ["name"],
            },
            "Query": {"type": "string", "enum": ["x", "y"]},
            "Cyclic": {"type": "object", "properties": {"self": {"$ref": "#/components/schemas/Cyclic"}}},
        }},
    }
    by = {(o["method"], o["path"]): o for o in build_operations(spec)}
    body = by[("post", "/a")]["body"]["schema"]
    assert body["properties"]["name"] == {"type": "string"}
    assert body["properties"]["opt"] == {"type": "string", "enum": ["x", "y"]}
    assert body["required"] == ["name"]
    assert by[("get", "/b")]["params"][0]["schema"] == {"type": "string", "enum": ["x", "y"]}
    # cyclic refs are cut with a bare object; external/dangling refs are left
    # for minify_schema, which reduces them to {} exactly as it always did
    assert by[("post", "/c")]["body"]["schema"]["properties"]["self"] == {"type": "object"}
    assert by[("post", "/d")]["body"]["schema"] == {}
    assert by[("post", "/e")]["body"]["schema"] == {}


def test_build_operations_resolves_fastapi_style_ref_spec():
    # FastAPI vendors emit $ref bodies exclusively; pre-deref they digested to
    # parameterless tools (the concordance trap). End-to-end through the
    # generic builder: the tool must carry the real flattened schema.
    spec = {
        "openapi": "3.1.0",
        "info": {"title": "RefVendor"},
        "servers": [{"url": "https://refvendor.example"}],
        "paths": {"/tools/search": {"post": {
            "operationId": "search",
            "summary": "Search Endpoint",
            "requestBody": {"required": True, "content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/SearchRequest"},
            }}},
            "responses": {"200": {"description": "ok"}},
        }}},
        "components": {"schemas": {
            "SearchRequest": {
                "properties": {
                    "query": {"type": "string", "title": "Query"},
                    "limit": {"type": "integer", "maximum": 100.0, "minimum": 1.0, "default": 20},
                },
                "type": "object",
                "required": ["query"],
            },
        }},
    }
    ops = build_operations(spec)
    cfg = GenericConfig(spec="x", base_url="https://refvendor.example", prefix="rv")
    tools = build_tools(cfg, ops, base_url="https://refvendor.example", prefix="rv", probe=False)
    assert len(tools) == 1
    schema = tools[0].input_schema
    assert schema["required"] == ["query"]
    assert schema["properties"]["limit"]["maximum"] == 100


def test_build_operations_resolves_ref_parameters():
    # A vendor may share one path parameter across routes via a pure $ref to
    # components/parameters (brazilayer's {cnpj}); pre-deref such routes
    # digested to argument-less tools calling unsubstituted {cnpj} URLs.
    spec = {
        "openapi": "3.1.0",
        "paths": {
            "/company/{cnpj}": {"get": {
                "summary": "Company data",
                "parameters": [{"$ref": "#/components/parameters/cnpj"}],
                "responses": {"200": {"description": "ok"}},
            }},
            "/lookup": {"get": {
                "summary": "External-ref param is skipped, not a crash",
                "parameters": [{"$ref": "https://elsewhere.example/p.json"}],
                "responses": {"200": {"description": "ok"}},
            }},
        },
        "components": {"parameters": {
            "cnpj": {
                "name": "cnpj", "in": "path", "required": True,
                "description": "Brazilian company tax ID",
                "schema": {"type": "string", "pattern": "^\\d{14}$"},
            },
        }},
    }
    (company, lookup) = build_operations(spec)
    assert company["params"] == [{
        "name": "cnpj",
        "in": "path",
        "required": True,
        "description": "Brazilian company tax ID",
        "schema": {"type": "string", "pattern": "^\\d{14}$"},
    }]
    assert lookup["params"] == []
    cfg = GenericConfig(spec="x", base_url="https://refvendor.example", prefix="rv")
    tools = build_tools(cfg, build_operations(spec), base_url="x", prefix="rv", probe=False)
    by = {t.name: t for t in tools}
    assert by["rv_company_cnpj"].param_routes == {"cnpj": "path"}
    assert by["rv_company_cnpj"].input_schema["required"] == ["cnpj"]
    assert by["rv_lookup"].input_schema["properties"] == {}


def test_build_script_output_is_canonical(tmp_path):
    """The script wrapper must emit the committed-digest format byte-for-byte."""
    spec_path = _write_spec(tmp_path, MINI_SPEC)
    out = tmp_path / "digest.json"
    subprocess.run(
        [
            sys.executable,
            str(REPO_ROOT / "scripts" / "build_socialfetch_digest.py"),
            str(spec_path),
            str(out),
        ],
        check=True,
        cwd=REPO_ROOT,
        capture_output=True,
    )
    raw = out.read_bytes()
    data = json.loads(raw)
    assert set(data) == {"meta", "operations"}
    assert data["meta"]["title"] == "Example API"
    assert data["meta"]["source"] == "https://www.socialfetch.dev/openapi.json"
    assert data["meta"]["operation_count"] == 3
    assert json.dumps(data, separators=(",", ":"), sort_keys=True).encode() == raw


# --------------------------------------------------------------------------
# Filtering, naming, prefix derivation
# --------------------------------------------------------------------------


def test_match_prefix_boundary():
    # /v1/web must not swallow /v1/webhook-*
    assert match_prefix("/v1/web/extract", ["/v1/web"]) == "/v1/web"
    assert match_prefix("/v1/webhook-x", ["/v1/web"]) is None
    assert match_prefix("/v1/web", ["/v1/web"]) == "/v1/web"
    assert match_prefix("/v1/web/extract", ["/v1", "/v1/web"]) == "/v1/web"


def test_include_exclude_filtering():
    ops = [_op("/v1/web/extract"), _op("/v1/webhook-x"), _op("/v1/twitter/profiles")]

    cfg = GenericConfig(spec="x", include=("/v1/web",), exclude=("/v1/web/extract",))
    assert select_ops(cfg, ops) == []

    cfg = GenericConfig(spec="x", include=("/v1/web",))
    assert [o["path"] for o, _ in select_ops(cfg, ops)] == ["/v1/web/extract"]

    cfg = GenericConfig(spec="x", exclude=("/v1/web",))
    assert [o["path"] for o, _ in select_ops(cfg, ops)] == [
        "/v1/twitter/profiles",
        "/v1/webhook-x",
    ]

    cfg = GenericConfig(spec="x")
    assert len(select_ops(cfg, ops)) == 3


def test_build_operations_tags_are_opt_in():
    spec = {
        "openapi": "3.1.0",
        "paths": {
            "/a": {"get": {"tags": ["Alpha", 3], "summary": "A"}},
            "/b": {"post": {"summary": "B"}},
        },
    }
    # digest-build paths keep their byte-identical artifacts: no tags key
    assert "tags" not in build_operations(spec)[0]
    # opt-in copies string tags only
    ops = build_operations(spec, include_tags=True)
    assert ops[0]["tags"] == ["Alpha"]
    assert ops[1]["tags"] == []


def test_tag_include_exclude_filtering():
    ops = [
        _op("/news", tags=["Market & Token Intelligence"]),
        _op("/weather", tags=["Real-World Data"]),
        _op("/swap", tags=["Execution"]),
        _op("/misc"),
    ]

    cfg = GenericConfig(spec="x", tags=("Real-World Data",))
    assert [o["path"] for o, _ in select_ops(cfg, ops)] == ["/weather"]

    cfg = GenericConfig(spec="x", exclude_tags=("Execution",))
    assert [o["path"] for o, _ in select_ops(cfg, ops)] == [
        "/misc",
        "/news",
        "/weather",
    ]

    # composed with path filters; untagged ops only survive unfiltered
    cfg = GenericConfig(spec="x", tags=("Market & Token Intelligence", "Real-World Data"))
    assert [o["path"] for o, _ in select_ops(cfg, ops)] == ["/news", "/weather"]
    cfg = GenericConfig(spec="x", tags=("Real-World Data",), exclude=("/weather",))
    assert select_ops(cfg, ops) == []
    cfg = GenericConfig(spec="x", tags=("Real-World Data",), include=("/swap",))
    assert select_ops(cfg, ops) == []


def test_resolve_config_tags_from_cli_env_and_conf(tmp_path, monkeypatch):
    conf = tmp_path / "c.json"
    conf.write_text(json.dumps({"spec": "s.json", "tags": ["A"], "exclude_tags": ["B"]}))
    pre, _ = _generic_parser().parse_known_args(["--config", str(conf)])
    assert resolve_config(pre).tags == ("A",)
    assert resolve_config(pre).exclude_tags == ("B",)

    monkeypatch.setenv("X402_MCP_GENERIC_TAGS", "C,D")
    pre, _ = _generic_parser().parse_known_args(["--spec", "s.json"])
    assert resolve_config(pre).tags == ("C", "D")

    pre, _ = _generic_parser().parse_known_args(["--spec", "s.json", "--exclude-tags", "E"])
    assert resolve_config(pre).exclude_tags == ("E",)


def test_default_prefix_derivation():
    assert default_prefix("https://api.socialfetch.dev") == "socialfetch"
    assert default_prefix("https://stable-deepline.dev") == "stable_deepline"
    assert default_prefix("http://localhost:8080") == "localhost"
    assert default_prefix("https://api.example.dev/v1") == "example"


def test_naming_multi_method_path():
    ops = [_op("/things", "get"), _op("/things", "post")]
    named = assign_names("ex", select_ops(GenericConfig(spec="x"), ops))
    assert [n for _, n in named] == ["ex_things_get", "ex_things_post"]


def test_naming_full_path_when_no_include():
    named = assign_names(
        "ex", select_ops(GenericConfig(spec="x"), [_op("/v1/things/{id}")])
    )
    assert [n for _, n in named] == ["ex_v1_things_id"]


def test_naming_path_equal_include_prefix_slugs_root():
    ops = [
        _op("/v1/things", "get"),
        _op("/v1/things", "post"),
        _op("/v1/things/{id}", "delete"),
    ]
    cfg = GenericConfig(spec="x", include=("/v1/things",))
    named = assign_names("ex", select_ops(cfg, ops))
    assert [n for _, n in named] == ["ex_root_get", "ex_root_post", "ex_id"]


def test_naming_backstop_for_slug_collisions():
    # /v1/a-b and /v1/a_b slug identically with the same method:
    # both get the method suffix, then the _{n} backstop disambiguates.
    ops = [_op("/v1/a-b"), _op("/v1/a_b")]
    named = assign_names("ex", select_ops(GenericConfig(spec="x"), ops))
    assert sorted(n for _, n in named) == ["ex_v1_a_b_get", "ex_v1_a_b_get_2"]


def test_naming_anchor_longest_include():
    ops = [_op("/v1/twitter/profiles/{handle}")]
    cfg = GenericConfig(spec="x", include=("/v1/twitter",))
    named = assign_names("socialfetch", select_ops(cfg, ops))
    assert [n for _, n in named] == ["socialfetch_profiles_handle"]

    cfg = GenericConfig(spec="x", include=("/v1",))
    named = assign_names("socialfetch", select_ops(cfg, ops))
    assert [n for _, n in named] == ["socialfetch_twitter_profiles_handle"]


# --------------------------------------------------------------------------
# Tool generation: schema flattening and routing
# --------------------------------------------------------------------------


def test_schema_flattening_and_routing(tmp_path):
    spec_path = _write_spec(tmp_path, MINI_SPEC)
    ops, title, server_url = load_source(str(spec_path), "x-example-credits")
    assert title == "Example API"
    assert server_url == "https://api.example.dev"

    cfg = GenericConfig(spec=str(spec_path))
    tools = build_tools(cfg, ops, base_url=server_url, prefix="ex", probe=False)
    by = {t.name: t for t in tools}
    assert set(by) == {"ex_search", "ex_things", "ex_things_id"}

    t = by["ex_search"]
    assert t.method == "GET" and not t.has_body
    assert t.param_routes == {"q": "query"}
    assert t.input_schema["required"] == ["q"]
    assert t.input_schema["properties"]["q"]["maxLength"] == 100
    assert 'Pricing: {"per_call":1} (vendor spec extension).' in t.description
    assert "$0.014" not in t.description
    # HTTP method/path stay internal: never appended to the model description
    assert "Endpoint:" not in t.description
    assert t.description.endswith('Pricing: {"per_call":1} (vendor spec extension).')
    assert len(t.description) <= 4096

    t = by["ex_things"]
    assert t.has_body and t.method == "POST"
    assert t.param_routes == {"size": "body"}
    assert t.input_schema["properties"]["size"]["maximum"] == 100
    assert t.input_schema["required"] == ["size"]
    assert "Paid per call via x402" in t.description

    t = by["ex_things_id"]
    # header param dropped; path param routed
    assert set(t.input_schema["properties"]) == {"id"}
    assert t.param_routes == {"id": "path"}


def test_body_query_collision_rename(tmp_path):
    spec = {
        "paths": {
            "/mix": {
                "post": {
                    "parameters": [
                        {"name": "dup", "in": "query", "schema": {"type": "string"}}
                    ],
                    "requestBody": {
                        "required": True,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "dup": {"type": "integer"},
                                        "only_body": {"type": "string"},
                                    },
                                    "required": ["dup", "only_body"],
                                }
                            }
                        },
                    },
                }
            }
        }
    }
    ops, _title, _url = load_source(str(_write_spec(tmp_path, spec)), None)
    tools = build_tools(
        GenericConfig(spec="x"), ops, base_url="https://x.dev", prefix="ex", probe=False
    )
    t = tools[0]
    assert set(t.param_routes.values()) == {"query", "body"}
    assert t.input_schema["properties"]["dup_body"]["type"] == "integer"
    # body-required names map back to the surviving property names, as in
    # socialfetch._tool_from_op ("dup" stays required; the body copy moved
    # to dup_body)
    assert t.input_schema["required"] == ["dup", "only_body"]


# --------------------------------------------------------------------------
# Config resolution
# --------------------------------------------------------------------------


def test_config_precedence_cli_beats_env_beats_conf(tmp_path, monkeypatch):
    conf_path = tmp_path / "conf.json"
    conf_path.write_text(
        json.dumps(
            {
                "spec": "conf-spec.json",
                "name": "conf-name",
                "prefix": "conf_prefix",
                "probe_pricing": False,
            }
        )
    )
    monkeypatch.setenv("X402_MCP_GENERIC_NAME", "env-name")
    monkeypatch.setenv("X402_MCP_GENERIC_PREFIX", "env_prefix")
    monkeypatch.setenv("X402_MCP_GENERIC_PROBE", "true")

    cfg = resolve_config(_cli(config=str(conf_path), name="cli-name"))
    assert cfg.spec == "conf-spec.json"  # conf is the only source for spec
    assert cfg.name == "cli-name"  # CLI beats env beats conf
    assert cfg.prefix == "env_prefix"  # env beats conf
    assert cfg.probe_pricing is True  # env(true) beats conf(false)


def test_config_defaults():
    cfg = resolve_config(_cli(spec="s.json"))
    assert cfg.probe_pricing is True
    assert cfg.probe_ttl_seconds == 3600.0
    assert cfg.probe_concurrency == 4
    assert cfg.probe_timeout == 5.0
    assert cfg.probe_max_endpoints == 200
    assert cfg.probe_methods == ("GET",)
    assert cfg.include == () and cfg.exclude == ()
    assert cfg.overrides == {}


def test_config_env_lists_and_bools(monkeypatch):
    monkeypatch.setenv("X402_MCP_GENERIC_INCLUDE", "/a, /b")
    monkeypatch.setenv("X402_MCP_GENERIC_EXCLUDE", "/c")
    monkeypatch.setenv("X402_MCP_GENERIC_PROBE", "off")
    monkeypatch.setenv("X402_MCP_GENERIC_PROBE_TTL", "60")
    cfg = resolve_config(_cli(spec="s.json"))
    assert cfg.include == ("/a", "/b")
    assert cfg.exclude == ("/c",)
    assert cfg.probe_pricing is False
    assert cfg.probe_ttl_seconds == 60.0


def test_unknown_conf_key_fails_loudly(tmp_path):
    conf_path = tmp_path / "conf.json"
    conf_path.write_text(json.dumps({"spec": "s", "spec_typo": 1}))
    with pytest.raises(ConfigError, match="spec_typo"):
        resolve_config(_cli(config=str(conf_path)))


def test_spec_required():
    with pytest.raises(ConfigError, match="no spec"):
        resolve_config(_cli())


def test_bad_env_bool(monkeypatch):
    monkeypatch.setenv("X402_MCP_GENERIC_PROBE", "maybe")
    with pytest.raises(ConfigError, match="PROBE"):
        resolve_config(_cli(spec="s.json"))


# --------------------------------------------------------------------------
# Spec/digest input detection
# --------------------------------------------------------------------------


def test_load_source_digest_detection(tmp_path):
    digest_path = tmp_path / "digest.json"
    digest_path.write_text(
        json.dumps(
            {
                "meta": {"title": "Digested API"},
                "operations": [_op("/paid", pricing={"per_call": 1})],
            }
        )
    )
    ops, title, server_url = load_source(str(digest_path), None)
    assert title == "Digested API"
    assert server_url is None  # digests carry no servers block
    assert ops[0]["pricing"] == {"per_call": 1}  # passthrough, untouched


def test_load_source_rejects_neither_shape(tmp_path):
    p = tmp_path / "junk.json"
    p.write_text(json.dumps({"info": {"title": "x"}}))
    with pytest.raises(ConfigError, match="neither"):
        load_source(str(p), None)


def test_load_source_missing_file():
    with pytest.raises(ConfigError, match="cannot load spec"):
        load_source("/nonexistent/spec.json", None)


# --------------------------------------------------------------------------
# Price discovery probe (localhost http.server)
# --------------------------------------------------------------------------


@pytest.fixture()
def probe_server():
    state = {"hits": 0}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            state["hits"] += 1
            if self.path == "/paid":
                challenge = {
                    "x402Version": 2,
                    "accepts": [
                        {
                            "scheme": "exact",
                            "network": "eip155:8453",
                            "maxAmountRequired": "14000",
                            "asset": USDC_BASE,
                            "payTo": "0x0000000000000000000000000000000000000001",
                        }
                    ],
                }
                payload = base64.b64encode(json.dumps(challenge).encode()).decode()
                self.send_response(402)
                self.send_header("PAYMENT-REQUIRED", payload)
                self.send_header("Content-Length", "0")
                self.end_headers()
            else:
                body = b'{"ok": true}'
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        def log_message(self, *args):  # silence
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}", state
    finally:
        server.shutdown()
        server.server_close()


def _cache(base_url, **kw) -> PricingCache:
    return PricingCache(
        base_url=base_url,
        ttl_seconds=kw.pop("ttl_seconds", 3600),
        timeout=kw.pop("timeout", 5.0),
        concurrency=kw.pop("concurrency", 2),
        methods=kw.pop("methods", ("GET",)),
        max_endpoints=kw.pop("max_endpoints", 200),
    )


def test_probe_cached_two_lookups_one_request(probe_server):
    base, state = probe_server
    cache = _cache(base)
    assert cache.warm([_op("/paid")]) == 1
    assert state["hits"] == 1

    first = cache.get("get", "/paid")
    second = cache.get("GET", "/paid")
    assert first is not None and second == first
    assert state["hits"] == 1  # cache lookups never hit the wire

    assert first["status"] == "challenge"
    assert first["challenge"]["scheme"] == "exact"
    assert first["challenge"]["maxAmountRequired"] == "14000"
    assert first["expires_at"] > 0


def test_challenge_accepts_v2_amount_field():
    """Live x402 v2 challenges carry `amount`, not maxAmountRequired."""
    from x402_mcp.generic import _challenge_line

    line = _challenge_line(
        {
            "scheme": "exact",
            "network": "eip155:8453",
            "amount": "14000",
            "asset": USDC_BASE,
        }
    )
    assert line == "Price: $0.014 per call (x402 exact, USDC on eip155:8453)."


def test_probe_rendering_into_description(probe_server):
    base, state = probe_server
    ops = [_op("/paid"), _op("/free")]
    tools = build_tools(
        GenericConfig(spec="x"), ops, base_url=base, prefix="ex", probe=True
    )
    by = {t.name: t for t in tools}

    assert state["hits"] == 2  # one unpaid GET per probed endpoint, ever
    assert (
        "$0.014 per call (x402 exact, USDC on eip155:8453)."
        in by["ex_paid"].description
    )
    assert "Paid per call via x402" in by["ex_free"].description  # 200: not gated


def test_probe_non_usdc_asset_shown_atomic(probe_server):
    base, _state = probe_server
    cache = _cache(base)
    cache.warm([_op("/paid")])
    from x402_mcp.generic import _challenge_line

    line = _challenge_line(
        {
            "scheme": "exact",
            "network": "eip155:8453",
            "maxAmountRequired": "2500000",
            "asset": "0x00000000000000000000000000000000deadbeef",
        }
    )
    assert line == "Price: 2500000 atomic per call (x402 exact, asset 0x00000000000000000000000000000000deadbeef on eip155:8453)."

    upto = _challenge_line(
        {
            "scheme": "upto",
            "network": "eip155:8453",
            "maxAmountRequired": 14000,
            "asset": USDC_BASE,
        }
    )
    assert upto == "Metered: up to $0.014 per call (x402 upto, USDC on eip155:8453)."


def test_probe_unreachable_yields_fallback_line():
    tools = build_tools(
        GenericConfig(spec="x"),
        [_op("/paid")],
        base_url="http://127.0.0.1:9",  # closed port
        prefix="ex",
        probe=True,
    )
    assert tools[0].description.startswith("GET /paid Paid per call via x402")


def test_probe_disabled_via_config_issues_zero_requests(probe_server):
    base, state = probe_server
    cfg = GenericConfig(spec="x", probe_pricing=False)
    build_tools(cfg, [_op("/paid")], base_url=base, prefix="ex", probe=True)
    assert state["hits"] == 0


def test_probe_skips_templated_paths_and_pricing_ops(probe_server):
    base, state = probe_server
    cache = _cache(base)
    ops = [
        _op("/paid"),
        _op("/things/{id}"),  # templated: never probed
        _op("/documented", pricing={"per_call": 2}),  # vendor pricing: skip
        _op("/paid", "post"),  # not in probe_methods (GET only)
    ]
    assert cache.warm(ops) == 1
    assert state["hits"] == 1
    assert cache.get("post", "/paid") is None
    assert cache.get("get", "/things/{id}") is None


def test_probe_max_endpoints_cap(probe_server):
    base, state = probe_server
    cache = _cache(base, concurrency=4, max_endpoints=2)
    ops = [_op("/z"), _op("/a"), _op("/m")]
    assert cache.warm(ops) == 2
    assert state["hits"] == 2
    assert cache.get("get", "/a") is not None
    assert cache.get("get", "/m") is not None
    assert cache.get("get", "/z") is None  # sorted order: /z beyond the cap


def test_probe_expiry_reads_absent_not_reprobes(probe_server):
    base, state = probe_server
    cache = _cache(base, ttl_seconds=0.0)  # immediately expired
    cache.warm([_op("/paid")])
    assert cache.get("get", "/paid") is None
    assert state["hits"] == 1  # expired entries never trigger a new request


# --------------------------------------------------------------------------
# Description assembly: one vendor text + vendor pricing prose
# --------------------------------------------------------------------------


def test_description_uses_long_vendor_text_not_the_summary_stub():
    """One vendor text, never both: the long `description` supersedes the
    summary (socialfetch-style paraphrase stubs and arkham-style echoes both
    duplicated text); summary only when no description exists."""
    op = _op(
        "/shop",
        summary="Get Widget page",
        description="Get a creator Widget storefront by URL.",
    )
    tool = _tool_from_op("ex_shop", op, "Paid per call.")
    assert tool.description == "Get a creator Widget storefront by URL. Paid per call."

    stub = _op("/page", summary="Get Widget page")
    assert _tool_from_op("ex_page", stub, "Paid.").description == (
        "Get Widget page Paid."
    )

    bare = _op("/raw")
    assert _tool_from_op("ex_raw", bare, "Paid.").description == "GET /raw Paid."


def test_vendor_pricing_line_shapes():
    from x402_mcp.generic import _vendor_pricing_line

    arkham_fixed = {
        "price": {"amount": "0.20", "currency": "USD", "mode": "fixed"},
        "protocols": [{"x402": {}}],
    }
    assert _vendor_pricing_line(arkham_fixed) == "Price: $0.20 per call (vendor spec)."

    arkham_dynamic = {
        "price": {"currency": "USD", "max": "659.60", "min": "0.40", "mode": "dynamic"},
        "protocols": [{"x402": {}}],
    }
    assert _vendor_pricing_line(arkham_dynamic) == (
        "Price: $0.40\u2013$659.60 per call, scaling with usage (vendor spec)."
    )

    # pdl writes six-decimal amounts; display trims to two decimals
    pdl_zeros = {"price": {"amount": "0.280000", "currency": "USD", "mode": "fixed"}}
    assert _vendor_pricing_line(pdl_zeros) == "Price: $0.28 per call (vendor spec)."

    credits = {"price": {"amount": "5", "currency": "credits", "mode": "fixed"}}
    assert _vendor_pricing_line(credits) == "Price: 5 credits per call (vendor spec)."

    # integer amounts and flat (non-nested) price blocks render too
    assert _vendor_pricing_line({"amount": 3, "currency": "USD"}) == (
        "Price: $3 per call (vendor spec)."
    )

    assert _vendor_pricing_line({"per_call": 1}) is None  # unknown shape
    assert _vendor_pricing_line("0.20") is None  # not a dict
    assert _vendor_pricing_line({"price": {"amount": "abc"}}) is None
    assert _vendor_pricing_line({"price": {"amount": 0.5}}) is None  # float: ambiguous
    assert _vendor_pricing_line({"price": {"min": "0.40"}}) is None  # range needs both


def test_description_prefers_long_text_and_skips_price_echo():
    """Arkham-style spec: the description already contains the summary echo
    and states the price the vendor extension also carries — use the one
    long text and render each fact once."""
    op = _op(
        "/balances/address",
        "post",
        summary="Get token balances",
        description="Get token balances. $0.20 per call.",
        pricing={
            "price": {"amount": "0.20", "currency": "USD", "mode": "fixed"},
            "protocols": [{"x402": {}}],
        },
    )
    line, vendor_priced = _pricing_line(op, None)
    assert (line, vendor_priced) == ("Price: $0.20 per call (vendor spec).", True)
    tool = _tool_from_op("ex_balances", op, line, vendor_priced=vendor_priced)
    assert tool.description == "Get token balances. $0.20 per call."


def test_dynamic_vendor_price_kept_when_prose_lacks_max():
    """Vendor prose covers the min but not the max — the rendered range
    (and its spend-cap-relevant max) must survive."""
    op = _op(
        "/swaps",
        "post",
        summary="Get swaps",
        description="Get swaps. Priced per row: $0.40 × limit.",
        pricing={
            "price": {
                "currency": "USD",
                "max": "659.60",
                "min": "0.40",
                "mode": "dynamic",
            }
        },
    )
    line, vendor_priced = _pricing_line(op, None)
    tool = _tool_from_op("ex_swaps", op, line, vendor_priced=vendor_priced)
    assert tool.description == (
        "Get swaps. Priced per row: $0.40 × limit."
        " Price: $0.40\u2013$659.60 per call, scaling with usage (vendor spec)."
    )


def test_probed_price_line_not_dropped_for_vendor_prose(probe_server):
    """A live challenge price outranks vendor prose: the probe line stays
    even when the description already mentions the same amount."""
    base, _state = probe_server
    ops = [_op("/paid", description="Costs $0.014.")]
    tools = build_tools(
        GenericConfig(spec="x"), ops, base_url=base, prefix="ex", probe=True
    )
    desc = tools[0].description
    assert "Costs $0.014." in desc
    assert "$0.014 per call (x402 exact, USDC on eip155:8453)." in desc


# --------------------------------------------------------------------------
# Overrides
# --------------------------------------------------------------------------


def test_overrides_replace_descriptions(probe_server):
    base, _state = probe_server
    ops = [
        _op(
            "/paid",
            params=[{"name": "id", "in": "query", "schema": {"type": "string"}}],
        )
    ]
    cfg = GenericConfig(
        spec="x",
        overrides={
            "ex_paid": {
                "description": "Search posts; cost scales with `size` (~$0.014/result).",
                "params": {"id": "Results per page, 1-100. Each result is billed."},
            },
            "ghost_tool": {"description": "stale override"},  # warned, ignored
        },
    )
    tools = build_tools(cfg, ops, base_url=base, prefix="ex", probe=False)
    t = tools[0]
    assert t.description == "Search posts; cost scales with `size` (~$0.014/result)."
    assert t.input_schema["properties"]["id"]["description"] == (
        "Results per page, 1-100. Each result is billed."
    )


def test_description_cap_is_opt_in_not_silent():
    """Vendor text is never trimmed and never silently capped: the assembled
    description stays whole unless the operator sets max_description_chars."""
    op = _op("/big", description="x" * 6000)
    whole = _tool_from_op("ex_big", op, "Paid per call.")
    assert len(whole.description) > 6000  # uncapped by default

    capped = _tool_from_op("ex_big", op, "Paid per call.", max_desc_chars=100)
    assert len(capped.description) == 100

    cfg = GenericConfig(spec="x", max_description_chars=100)
    tools = build_tools(
        cfg, [_op("/big", description="x" * 6000)], base_url="https://x.dev", prefix="ex", probe=False
    )
    assert len(tools[0].description) == 100


def test_config_max_description_chars_validation(tmp_path):
    conf_path = tmp_path / "conf.json"
    conf_path.write_text(json.dumps({"spec": "s", "max_description_chars": 0}))
    with pytest.raises(ConfigError, match="max_description_chars"):
        resolve_config(_cli(config=str(conf_path)))


def test_config_guidance_keys_from_conf_and_env(tmp_path, monkeypatch):
    conf_path = tmp_path / "conf.json"
    conf_path.write_text(json.dumps({"spec": "s", "instructions_text": "conf text"}))
    monkeypatch.setenv("X402_MCP_GENERIC_INSTRUCTIONS_TEXT", "env text")
    monkeypatch.setenv("X402_MCP_GENERIC_MAX_DESCRIPTION_CHARS", "250")
    cfg = resolve_config(_cli(config=str(conf_path)))
    assert cfg.instructions_text == "env text"  # env beats conf
    assert cfg.max_description_chars == 250
    assert cfg.help_url is None  # unset everywhere

    conf_path.write_text(
        json.dumps(
            {
                "spec": "s",
                "instructions_text": "Short guidance.",
                "help_url": "https://x.dev/llms.txt",
                "max_description_chars": 500,
            }
        )
    )
    monkeypatch.delenv("X402_MCP_GENERIC_INSTRUCTIONS_TEXT")
    monkeypatch.delenv("X402_MCP_GENERIC_MAX_DESCRIPTION_CHARS")
    cfg = resolve_config(_cli(config=str(conf_path)))
    assert cfg.instructions_text == "Short guidance."
    assert cfg.help_url == "https://x.dev/llms.txt"
    assert cfg.max_description_chars == 500


def test_help_tool_lazy_fetch_and_cache(probe_server):
    base, state = probe_server
    cfg = GenericConfig(spec="x", help_url=f"{base}/docs")
    tools = build_tools(cfg, [_op("/things")], base_url=base, prefix="ex", probe=False)
    help_tool = {t.name: t for t in tools}["ex_help"]
    assert not help_tool.input_schema.get("properties")
    assert "ex_*" in help_tool.description

    text1 = help_tool.local_content()
    text2 = help_tool.local_content()
    assert text1 == text2  # served from the process cache
    assert state["hits"] == 1  # fetched from the wire exactly once


def test_help_tool_fetch_failure_raises_and_can_retry():
    cfg = GenericConfig(spec="x", help_url="http://127.0.0.1:9/docs")  # closed port
    tools = build_tools(
        cfg, [_op("/things")], base_url="http://127.0.0.1:9", prefix="ex", probe=False
    )
    with pytest.raises(Exception):  # URLError; the runner turns it into isError
        {t.name: t for t in tools}["ex_help"].local_content()


def test_override_unknown_tool_warns_not_raises(caplog):
    tools = [_tool_from_op("ex_paid", _op("/paid"), "Paid per call via x402.")]
    with caplog.at_level("WARNING", logger="x402_mcp.generic"):
        apply_overrides(tools, {"ghost": {"description": "x"}})
    assert any("ghost" in r.getMessage() for r in caplog.records)
    assert "Paid per call via x402." in tools[0].description


# --------------------------------------------------------------------------
# Launcher wiring (main / --list-tools)
# --------------------------------------------------------------------------


def test_list_tools_probes_nothing(tmp_path, monkeypatch, capsys):
    from x402_mcp.generic import main as generic_main

    spec = {
        "info": {"title": "Probe API"},
        "servers": [{"url": "http://unused.example"}],
        "paths": {"/paid": {"get": {"summary": "Paid thing"}}},
    }
    spec_path = _write_spec(tmp_path, spec)

    state = {"hits": 0}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            state["hits"] += 1
            self.send_response(402)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = f"http://127.0.0.1:{server.server_address[1]}"
    try:
        generic_main(
            ["--spec", str(spec_path), "--base-url", base, "--prefix", "ex", "--list-tools"]
        )
    finally:
        server.shutdown()
        server.server_close()

    assert state["hits"] == 0  # --list-tools stays wallet- and network-free
    out = capsys.readouterr().out
    assert "ex_paid" in out
    assert "Description:" in out  # new listing format: legend + per-tool blocks
