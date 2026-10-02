"""Offline catalog differential against the existing Python implementation.
Run from repo root: .venv/bin/python rust-prototype/tests/compatibility.py
"""
import dataclasses
import json
import os
from pathlib import Path
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from x402_mcp.generic import GenericConfig, build_tools, default_prefix, load_source

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / 'rust-prototype/target/debug/x402-mcp-prototype'
CASES = [(f'confs/{name}.json', f'tests/fixtures/{fixture}_openapi.json') for name, fixture in [
    ('pdl', 'pdl'), ('deepline', 'deepline'), ('kronos', 'kronos'),
    ('regimeshift', 'regimeshift'), ('agentfund', 'agentfund'), ('otto', 'otto'),
    ('lonestar', 'lonestar_unified'), ('brazilayer', 'brazilayer'), ('genuinegood', 'genuinegood'),
]] + [(f'confs/{name}.json', f'confs/{name}/{name}_digest.json') for name in ['straits', 'concordance', 'locus']] + [('confs/glassnode/glassnode.json', 'confs/glassnode/glassnode_digest.json')]

def main():
    total = 0
    for conf, fixture in CASES:
        if not (ROOT / conf).exists():
            raise AssertionError(f'missing fixture conf: {conf}')
        cfg = GenericConfig(**json.loads((ROOT / conf).read_text()))
        ops, base, _ = load_source(str(ROOT / fixture), cfg.pricing_key)
        base = cfg.base_url or base
        tools = build_tools(cfg, ops, base_url=base, prefix=cfg.prefix or default_prefix(base), probe=False)
        expected = []
        for tool in tools:
            item = dataclasses.asdict(tool)
            local = item.pop('local_content')
            if local:
                item['help_url'] = cfg.help_url
            expected.append(item)
        actual = json.loads(subprocess.check_output([str(BIN), '--config', conf, '--spec', fixture, '--list-tools'], cwd=ROOT,
            env={k: v for k, v in os.environ.items() if not k.startswith(('X402_', 'EVM_', 'SVM_'))}))
        # Catalog order is not part of the MCP tool contract.
        expected.sort(key=lambda t: t['name'])
        actual.sort(key=lambda t: t['name'])
        assert len(actual) == len(expected), (conf, len(actual), len(expected))
        for got, want in zip(actual, expected):
            for key in want:
                assert got.get(key) == want[key], (conf, want['name'], key, got.get(key), want[key])
        total += len(tools)
        print(f'{conf}: {len(tools)} tools match')
    print(f'{total} tool definitions match Python (names, schemas, descriptions, routes).')

if __name__ == '__main__':
    main()
