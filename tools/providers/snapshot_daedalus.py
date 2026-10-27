#!/usr/bin/env python3
"""Development-only request snapshot repair; reads local public OpenAPI JSON."""

import argparse
import copy
import json
from pathlib import Path


def snapshot(document):
    document = copy.deepcopy(document)
    header = copy.deepcopy(
        document["paths"]["/v1/site-audit"]["post"]["parameters"][0]
    )
    assert header["name"] == "X-Agent-Id" and header["in"] == "header"
    header.pop("example", None)
    repaired = []
    for path, item in document["paths"].items():
        for method, operation in item.items():
            if method not in {"get", "post", "put", "patch", "delete", "head", "options", "trace"}:
                continue
            security = operation.get("security", document.get("security", []))
            parameters = operation.get("parameters", [])
            if any("AgentIdentity" in entry for entry in security) and not any(
                parameter.get("in") == "header"
                and parameter.get("name", "").lower() == "x-agent-id"
                for parameter in parameters
            ):
                operation.setdefault("parameters", []).append(copy.deepcopy(header))
                repaired.append(f"{method.upper()} {path}")
            operation.pop("responses", None)
    document["x-treazury-provenance"] = {
        "source": "https://agents.daedalusdevelopmentgroup.com/openapi.json",
        "transformation": "Request snapshot; materialize AgentIdentity security as the vendor site-audit X-Agent-Id parameter only when absent; omit response documentation.",
        "identity_header_operations": repaired,
    }
    return json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    result = snapshot(json.loads(args.source.read_text()))
    args.output.write_text(result)
