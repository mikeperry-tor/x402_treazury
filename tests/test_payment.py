"""Regression test: over-long challenge descriptions must not poison payments.

Coinbase's CDP facilitator validates the echoed resource.description with a
500-character maximum. Vendors that inline full op prose into their 402
challenge (straits.live's hormuz/risk-screen does, at 1159 chars) otherwise
fail every payment for those routes with "'paymentPayload' is invalid". The
description is not signature-covered, so payment.py trims it at challenge
parse time.
"""

from __future__ import annotations

import base64
import json

from x402_mcp.payment import (
    MAX_CHALLENGE_DESCRIPTION,
    _ChallengeSanitizingHTTPClient,
)


def _header(description: str) -> str:
    challenge = {
        "x402Version": 2,
        "error": "Payment Required",
        "resource": {
            "url": "https://vendor.example/api/premium/thing",
            "description": description,
            "mimeType": "application/json",
        },
        "accepts": [
            {
                "scheme": "exact",
                "network": "eip155:8453",
                "amount": "10000",
                "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                "payTo": "0x1B24dEAc0951EFCD6923f684b574Fe9a4202Cf1f",
                "maxTimeoutSeconds": 300,
                "extra": {"name": "USD Coin", "version": "2"},
            }
        ],
        "extensions": {},
    }
    return base64.b64encode(json.dumps(challenge).encode()).decode()


def test_long_challenge_description_is_trimmed_to_facilitator_cap():
    client = _ChallengeSanitizingHTTPClient.__new__(_ChallengeSanitizingHTTPClient)
    parsed = client.get_payment_required_response(
        lambda name: _header("x" * 1159) if name.upper() == "PAYMENT-REQUIRED" else None
    )
    assert len(parsed.resource.description) == MAX_CHALLENGE_DESCRIPTION == 500
    assert parsed.resource.url == "https://vendor.example/api/premium/thing"
    assert parsed.accepts[0].amount == "10000"


def test_short_challenge_description_passes_through_untouched():
    client = _ChallengeSanitizingHTTPClient.__new__(_ChallengeSanitizingHTTPClient)
    parsed = client.get_payment_required_response(
        lambda name: _header("x" * 429) if name.upper() == "PAYMENT-REQUIRED" else None
    )
    assert len(parsed.resource.description) == 429
