-- Canonical replacements append new authority; original resolutions stay immutable.
CREATE TABLE payment_resolution_repairs(
 sequence INTEGER PRIMARY KEY,
 id TEXT NOT NULL UNIQUE,
 wallet_id TEXT NOT NULL REFERENCES wallets(id),
 activated INTEGER NOT NULL DEFAULT 1 CHECK(activated IN (0,1)),
 verified_at INTEGER,
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
 balance_support TEXT,
 CHECK(activated=0 OR (verified_at IS NOT NULL AND balance_support IS NOT NULL))
);
CREATE INDEX payment_repair_wallet ON payment_resolution_repairs(wallet_id,activated,sequence DESC);
CREATE INDEX payment_repair_confirmed ON payment_resolution_repairs(confirmed_hash,wallet_id,sequence);
CREATE INDEX payment_repair_latest ON payment_resolution_repairs(latest_hash,wallet_id,sequence);
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
 SELECT r.attempt_id,COALESCE(n.outcome,r.outcome) AS outcome,COALESCE(n.height,r.height) AS height,
 COALESCE(n.hash,r.hash) AS hash,COALESCE(n.block_time,r.block_time) AS block_time,
 CASE WHEN n.id IS NOT NULL THEN n.support ELSE r.support END AS support,
 CASE WHEN n.id IS NOT NULL THEN n.endpoint ELSE
 CASE WHEN json_valid(r.support) THEN CASE WHEN json_type(r.support,'$.endpoint')='text'
 THEN json_extract(r.support,'$.endpoint') END END END AS endpoint
 FROM payment_resolutions r JOIN payment_attempts a ON a.id=r.attempt_id
 LEFT JOIN payment_resolution_replacements n ON n.attempt_id=r.attempt_id AND n.repair_id=(
 SELECT p.id FROM payment_resolution_repairs p WHERE p.wallet_id=a.wallet_id AND p.activated=1 ORDER BY p.sequence DESC LIMIT 1);
CREATE TRIGGER payment_repair_revision_insert AFTER INSERT ON payment_resolution_repairs BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=NEW.wallet_id AND NEW.activated=1; END;
CREATE TRIGGER payment_replacement_revision_insert AFTER INSERT ON payment_resolution_replacements BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=(SELECT wallet_id FROM payment_resolution_repairs WHERE id=NEW.repair_id AND activated=1); END;
CREATE TRIGGER payment_repair_revision_update AFTER UPDATE ON payment_resolution_repairs BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=OLD.wallet_id AND OLD.activated=1; UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=NEW.wallet_id AND NEW.activated=1; END;
CREATE TRIGGER payment_replacement_revision_update AFTER UPDATE ON payment_resolution_replacements BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=(SELECT wallet_id FROM payment_resolution_repairs WHERE id=OLD.repair_id AND activated=1); UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=(SELECT wallet_id FROM payment_resolution_repairs WHERE id=NEW.repair_id AND activated=1); END;
CREATE TRIGGER payment_repair_revision_delete AFTER DELETE ON payment_resolution_repairs BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=OLD.wallet_id AND OLD.activated=1; END;
CREATE TRIGGER payment_replacement_revision_delete AFTER DELETE ON payment_resolution_replacements BEGIN UPDATE wallets SET resolved_revision=resolved_revision+1,liability_revision=liability_revision+1 WHERE id=(SELECT wallet_id FROM payment_resolution_repairs WHERE id=OLD.repair_id AND activated=1); END;
