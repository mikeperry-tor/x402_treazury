# Historical state schemas

`historical.sql` freezes the base schema and additive table introductions through
user_version 10. The fixture constructor executes only sections at or below the
selected version; it does not run the production migration routine or drop tables
from a current database.

The schemas match the supported additive migration history: v1 admission, v2 sync,
v3 source operations, v4 funding progress, v5 refund addresses, v6 recovery
bindings, v7 refund outputs, v8 expiry, v9 funding health and v10 network identity.
Rows are copied from a disposable populated store because encrypted record format
v1 is independent of these table additions. The resulting fixtures preserve
pending payment/source liability and the quote/refund/recovery records that exist
at each historical version. Foreign-key integrity is checked before migration.

Tests explicitly check v9 health backfill and v10's legacy identity behavior:
funding/recovery bindings supply an immutable recipient where available; an
unbound legacy operation remains unable to construct a network identity. These
fixtures do not claim recovery of information that an older schema never stored.

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
