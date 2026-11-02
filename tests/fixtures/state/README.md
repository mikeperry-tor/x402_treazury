# State fixture boundaries

Application database fixtures use the current schema 23 and public unfunded keys.
Unsupported old/future versions are compact boundary cases: refusal must preserve
all source bytes and accepted obligations. Historical schema/projection matrices
and database converters are not a supported fixture contract. Current corruption,
read-only inspection, interrupted empty-history initialization, backup and reopen
coverage belongs to the owning Store tests.

Database, SDK, public output and encryption versions are independent; see the
[format contracts](../../../docs/wallet-cli.md#versioned-capacity-output). Fixture
cleanup neither converts private state nor erases financial evidence or grants
replay authority.

## SDK wallet upgrade

`sdk_v6_wallet.bin` is a 462-byte format-42 wallet generated offline from Zingolib
revision `c6381534f802b1022041beda4b01c106ad132329` before the v7 import, using the
application graph at `f52c451` and Rust 1.99.1. It contains only the public BIP39
`abandon` test mnemonic ending in `about`, one account, birthday 2,000,000, default
wallet settings, two additional `all_shielded` unified addresses, and a scan target
at height 2,000,010 with txid bytes `[7; 32]` and `narrow_scan_area=true`. It has no
funded notes or network observations. SHA-256:
`1f6e4b9df21037893a10c0ab8e7ccca983ecb8f57c2d9ff2243527e25f2b7efc`.

`sdk_v6_addresses.json` records account/index/address values before save and after
one further address derivation under that v6 graph. The upgrade regression reads
these original bytes through v7, preserves the nested scan target, saves format
45, reopens it and checks next-address continuity. A freshly created v7 wallet
cannot replace this fixture: that would stop testing the old serialization reader.
