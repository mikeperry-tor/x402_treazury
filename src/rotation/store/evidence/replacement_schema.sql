-- Canonical replacements append new authority; original resolutions stay immutable.
CREATE TABLE payment_resolution_repairs(
 sequence INTEGER PRIMARY KEY,
 id TEXT NOT NULL UNIQUE,
 wallet_id TEXT NOT NULL REFERENCES wallets(id),
 verified_at INTEGER NOT NULL,
 original_financial_height INTEGER,
 original_financial_hash TEXT,
 original_debit_height INTEGER,
 original_debit_hash TEXT,
 evidence_digest TEXT NOT NULL,
 resolution_count INTEGER NOT NULL CHECK(resolution_count>=0),
 confirmed_height INTEGER NOT NULL,
 confirmed_hash TEXT NOT NULL,
 latest_height INTEGER NOT NULL,
 latest_hash TEXT NOT NULL,
 endpoint TEXT NOT NULL,
 balance_support TEXT NOT NULL
);
CREATE INDEX payment_repair_wallet ON payment_resolution_repairs(wallet_id,sequence DESC);
CREATE INDEX payment_repair_confirmed ON payment_resolution_repairs(confirmed_hash,wallet_id);
CREATE INDEX payment_repair_latest ON payment_resolution_repairs(latest_hash,wallet_id);
CREATE TABLE payment_resolution_replacements(
 id INTEGER PRIMARY KEY,
 repair_id TEXT NOT NULL REFERENCES payment_resolution_repairs(id),
 attempt_id TEXT NOT NULL REFERENCES payment_attempts(id),
 outcome TEXT NOT NULL CHECK(outcome IN ('USED','EXPIRED_UNUSED')),
 height INTEGER NOT NULL,
 hash TEXT NOT NULL,
 block_time INTEGER NOT NULL,
 support TEXT NOT NULL,
 endpoint TEXT NOT NULL,
 UNIQUE(repair_id,attempt_id)
);
CREATE INDEX payment_replacement_attempt ON payment_resolution_replacements(attempt_id,id DESC);
CREATE INDEX payment_replacement_endpoint ON payment_resolution_replacements(endpoint,attempt_id,id);
CREATE VIEW effective_payment_resolutions AS
 SELECT r.attempt_id,r.outcome,r.height,r.hash,r.block_time,r.support,
 CASE WHEN json_valid(r.support) THEN CASE WHEN json_type(r.support,'$.endpoint')='text'
 THEN json_extract(r.support,'$.endpoint') END END AS endpoint
 FROM payment_resolutions r
 WHERE NOT EXISTS(SELECT 1 FROM payment_resolution_replacements n WHERE n.attempt_id=r.attempt_id)
 UNION ALL
 SELECT r.attempt_id,r.outcome,r.height,r.hash,r.block_time,r.support,r.endpoint
 FROM payment_resolution_replacements r
 WHERE r.id=(SELECT n.id FROM payment_resolution_replacements n WHERE n.attempt_id=r.attempt_id ORDER BY n.id DESC LIMIT 1);
