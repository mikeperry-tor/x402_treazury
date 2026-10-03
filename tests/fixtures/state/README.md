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
