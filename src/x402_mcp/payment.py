"""Build an x402 payment-capable HTTP client holding the wallet.

The x402 httpx transport intercepts HTTP 402 responses, signs the requested
payment with the configured wallet, and retries the original request with a
PAYMENT-SIGNATURE header. Schemes registered:

- exact (EVM, all eip155 networks, V2 wildcard + V1 legacy networks)
- upto  (EVM, all eip155 networks) — required by metered routes such as
  Social Fetch search endpoints. Metered routes settle via Permit2, so the
  paying wallet must have USDC.approve(Permit2) once on-chain.
- exact (Solana) if SVM_PRIVATE_KEY is set and x402[svm] extras are installed
"""

from __future__ import annotations

import logging

from eth_account import Account
from x402 import x402Client
from x402.http import x402HTTPClient
from x402.http.clients import x402HttpxClient
from x402.mechanisms.evm import EthAccountSigner
from x402.mechanisms.evm.exact.register import register_exact_evm_client
from x402.mechanisms.evm.upto import UptoEvmScheme

from .config import ConfigError, PaymentConfig

logger = logging.getLogger("x402_mcp")


def _log_before_payment(ctx) -> None:
    req = ctx.selected_requirements
    amount = req.get_amount() if hasattr(req, "get_amount") else getattr(req, "amount", "?")
    pay_to = getattr(req, "pay_to", None) or getattr(req, "payTo", "?")
    logger.info(
        "x402 payment signing: scheme=%s network=%s amount=%s asset=%s pay_to=%s",
        req.scheme,
        req.network,
        amount,
        req.asset,
        pay_to,
    )
    return None


def build_x402_client(cfg: PaymentConfig) -> x402Client:
    client = x402Client()

    if cfg.evm_private_key:
        account = Account.from_key(cfg.evm_private_key)
        signer = EthAccountSigner(account)
        register_exact_evm_client(client, signer)
        client.register("eip155:*", UptoEvmScheme(signer))
        logger.info("EVM wallet: %s", account.address)

    if cfg.svm_private_key:
        try:
            from x402.mechanisms.svm import KeypairSigner
            from x402.mechanisms.svm.exact.register import register_exact_svm_client
        except ImportError as e:  # pragma: no cover
            raise ConfigError(
                "SVM_PRIVATE_KEY is set but Solana support is not installed. "
                "Install with: uv sync --extra svm"
            ) from e
        svm_signer = KeypairSigner.from_base58(cfg.svm_private_key)
        register_exact_svm_client(client, svm_signer)
        logger.info("Solana wallet: %s", svm_signer.address)

    # Spend controls enforce an asset allowlist plus a per-payment USD cap.
    # None (X402_MAX_PRICE_USD=none) disables them entirely.
    if cfg.max_price_usd is None:
        client.set_spend_controls(False)
        logger.warning("x402 spend controls disabled (X402_MAX_PRICE_USD=none)")
    else:
        client.set_spend_controls({"max_amount_per_payment": f"${cfg.max_price_usd:g}"})
        logger.info("x402 per-payment spend cap: $%g", cfg.max_price_usd)

    client.on_before_payment_creation(_log_before_payment)
    return client


def build_paid_http_client(
    x402_client: x402Client,
    base_url: str,
    timeout_s: float,
) -> tuple[x402HttpxClient, x402HTTPClient]:
    """Return (payment-enabled httpx client, http helper for receipt decoding)."""
    import httpx

    http = x402HttpxClient(
        x402_client,
        base_url=base_url,
        timeout=httpx.Timeout(timeout_s, connect=15.0),
        follow_redirects=True,
    )
    return http, x402HTTPClient(x402_client)
