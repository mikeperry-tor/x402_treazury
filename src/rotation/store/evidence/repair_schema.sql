-- Additive incident-repair audit; original financial support is never rewritten.
CREATE TABLE IF NOT EXISTS wallet_evidence_repairs(
    id INTEGER PRIMARY KEY,
    wallet_id TEXT NOT NULL REFERENCES wallets(id),
    verified_at INTEGER NOT NULL,
    evidence_digest TEXT NOT NULL,
    resolution_count INTEGER NOT NULL,
    balance_count INTEGER NOT NULL,
    confirmed_height INTEGER NOT NULL,
    confirmed_hash TEXT NOT NULL,
    latest_height INTEGER NOT NULL,
    latest_hash TEXT NOT NULL
);
