"""Wallet / payment configuration loaded from environment and CLI overrides."""

from __future__ import annotations

import os
from dataclasses import dataclass, field

from dotenv import load_dotenv

USDC_DECIMALS = 6


class ConfigError(RuntimeError):
    pass


@dataclass(frozen=True)
class PaymentConfig:
    evm_private_key: str | None
    svm_private_key: str | None
    max_price_usd: float | None
    wallet_address: str | None = None

    @property
    def max_price_atomic(self) -> int | None:
        if self.max_price_usd is None:
            return None
        return int(round(self.max_price_usd * 10**USDC_DECIMALS))


def load_payment_config(
    env_file: str | None = None,
    max_price_usd_override: float | None = None,
) -> PaymentConfig:
    """Load wallet settings. Requires at least one of EVM_PRIVATE_KEY / SVM_PRIVATE_KEY.

    X402_MAX_PRICE_USD caps what the server will pay per request (6-decimal
    atomic amount, i.e. treats the asset as USD-quoted). Empty string or
    "none" disables the cap. Default cap: $1.00.
    """
    if env_file:
        load_dotenv(env_file, override=True)
    else:
        load_dotenv()

    evm = os.getenv("EVM_PRIVATE_KEY") or None
    svm = os.getenv("SVM_PRIVATE_KEY") or None
    if not evm and not svm:
        raise ConfigError(
            "No wallet configured. Set EVM_PRIVATE_KEY (0x-prefixed hex) and/or "
            "SVM_PRIVATE_KEY (base58) in the environment or an .env file."
        )

    raw_cap = os.getenv("X402_MAX_PRICE_USD", "1.00").strip().lower()
    if max_price_usd_override is not None:
        cap = max_price_usd_override
    elif raw_cap in ("", "none", "off", "disable", "disabled"):
        cap = None
    else:
        try:
            cap = float(raw_cap)
        except ValueError as e:
            raise ConfigError(f"Invalid X402_MAX_PRICE_USD value: {raw_cap!r}") from e

    return PaymentConfig(evm_private_key=evm, svm_private_key=svm, max_price_usd=cap)
