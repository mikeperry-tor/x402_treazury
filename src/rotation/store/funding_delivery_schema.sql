-- Provider hints are a bounded cache, never financial evidence. One owned
-- candidate and one replaceable queued candidate per original operation.
CREATE TABLE funding_delivery_hints(
 operation_id TEXT PRIMARY KEY,
 job_id TEXT NOT NULL REFERENCES funding_jobs(id),
 quote_digest BLOB NOT NULL CHECK(length(quote_digest)=32),
 active BLOB,
 queued BLOB,
 ticket TEXT,
 issue_bits INTEGER NOT NULL DEFAULT 0,
 prefer_event INTEGER NOT NULL DEFAULT 1 CHECK(prefer_event IN (0,1)),
 declared_return TEXT CHECK(declared_return IS NULL OR (length(declared_return) BETWEEN 1 AND 78 AND declared_return NOT GLOB '*[^0-9]*')),
 return_conflict INTEGER NOT NULL DEFAULT 0 CHECK(return_conflict IN (0,1)),
 CHECK((active IS NULL)=(ticket IS NULL))
);
-- Event ownership survives disputes and replacement canonical support.
CREATE TABLE funding_delivery_events(
 id INTEGER PRIMARY KEY,
 operation_id TEXT NOT NULL,
 job_id TEXT NOT NULL REFERENCES funding_jobs(id),
 wallet_id TEXT NOT NULL REFERENCES wallets(id),
 quote_digest BLOB NOT NULL CHECK(length(quote_digest)=32),
 source_transaction TEXT NOT NULL,
 transaction_hash TEXT NOT NULL,
 log_index INTEGER NOT NULL,
 recipient TEXT NOT NULL,
 amount TEXT NOT NULL,
 UNIQUE(transaction_hash,log_index)
);
CREATE INDEX funding_delivery_operation ON funding_delivery_events(operation_id,id);
-- Chain and token are pinned by this table's sole writer to Base native USDC.
CREATE TABLE funding_delivery_proofs(
 id INTEGER PRIMARY KEY,
 event_id INTEGER NOT NULL REFERENCES funding_delivery_events(id),
 block_height INTEGER NOT NULL,
 block_hash TEXT NOT NULL,
 endpoint TEXT NOT NULL,
 endpoint_generation INTEGER NOT NULL,
 UNIQUE(event_id,block_height,block_hash,endpoint,endpoint_generation)
);
CREATE INDEX funding_delivery_endpoint ON funding_delivery_proofs(endpoint,event_id);
CREATE INDEX funding_delivery_anchor ON funding_delivery_proofs(block_hash,event_id);
CREATE TABLE funding_delivery_disputes(
 proof_id INTEGER PRIMARY KEY REFERENCES funding_delivery_proofs(id),
 reason TEXT NOT NULL
);
CREATE TRIGGER funding_delivery_event_immutable BEFORE UPDATE ON funding_delivery_events BEGIN SELECT RAISE(ABORT,'immutable delivery event'); END;
CREATE TRIGGER funding_delivery_event_retained BEFORE DELETE ON funding_delivery_events BEGIN SELECT RAISE(ABORT,'retained delivery event'); END;
CREATE TRIGGER funding_delivery_proof_immutable BEFORE UPDATE ON funding_delivery_proofs BEGIN SELECT RAISE(ABORT,'immutable delivery proof'); END;
CREATE TRIGGER funding_delivery_proof_retained BEFORE DELETE ON funding_delivery_proofs BEGIN SELECT RAISE(ABORT,'retained delivery proof'); END;
CREATE TABLE funding_delivery_current(
 operation_id TEXT PRIMARY KEY,
 event_id INTEGER NOT NULL UNIQUE REFERENCES funding_delivery_events(id),
 proof_id INTEGER NOT NULL REFERENCES funding_delivery_proofs(id)
);
