"""Verify remaining published sources plus reviewed transport and MCP diagnostic patches."""
from verify_transport import verify

verify()
verify("rmcp-sources.json")
