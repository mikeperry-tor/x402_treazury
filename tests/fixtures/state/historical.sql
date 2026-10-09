-- Frozen additive schema definitions, reviewed against admission_schema.
-- Applied independently; the fixture constructor does not invoke migrations.
-- version 0
CREATE TABLE instance(id TEXT PRIMARY KEY,version INTEGER NOT NULL CHECK(version=1),birthday INTEGER NOT NULL,network TEXT NOT NULL CHECK(network='mainnet'),account INTEGER NOT NULL CHECK(account=0));
CREATE TABLE snapshots(revision INTEGER PRIMARY KEY,bytes BLOB NOT NULL);
CREATE TABLE pools(id TEXT PRIMARY KEY,name TEXT UNIQUE NOT NULL,target TEXT NOT NULL,generation INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),bootstrapped INTEGER NOT NULL DEFAULT 0);
CREATE TABLE wallets(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),sequence INTEGER NOT NULL,address TEXT UNIQUE NOT NULL,key BLOB NOT NULL,target TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('ALLOCATED','READY','ACTIVE','RETIRED')),balance TEXT NOT NULL DEFAULT '0',block_hash TEXT,block_height INTEGER,UNIQUE(pool_id,sequence));
CREATE UNIQUE INDEX one_active ON wallets(pool_id) WHERE role='ACTIVE';
CREATE UNIQUE INDEX one_ready ON wallets(pool_id) WHERE role='READY';
CREATE TABLE funding_jobs(id TEXT PRIMARY KEY,wallet_id TEXT UNIQUE NOT NULL REFERENCES wallets(id),state TEXT NOT NULL CHECK(state IN ('QUEUED','COMPLETE')),target TEXT NOT NULL);
CREATE TABLE budget_entries(id TEXT PRIMARY KEY,pool_id TEXT REFERENCES pools(id),day INTEGER NOT NULL,original_day INTEGER NOT NULL,requested INTEGER NOT NULL CHECK(requested>0),reserved INTEGER NOT NULL CHECK(reserved>=0),consumed INTEGER NOT NULL CHECK(consumed>=0));
CREATE TABLE outgoing(id TEXT PRIMARY KEY REFERENCES budget_entries(id),state TEXT NOT NULL CHECK(state IN ('PREPARED','RESOLVED')),raw BLOB NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
CREATE UNIQUE INDEX one_outgoing ON outgoing((1)) WHERE state='PREPARED';
-- version 1
CREATE TABLE payment_attempts(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),wallet_id TEXT NOT NULL REFERENCES wallets(id),generation INTEGER NOT NULL,amount TEXT NOT NULL,requirements_hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('ADMITTED','POSSIBLY_SUBMITTED','RESOLVED')),payer TEXT,payee TEXT,nonce TEXT,valid_after INTEGER,valid_before INTEGER,UNIQUE(wallet_id,nonce));
-- version 1
CREATE TABLE payment_anchors(pool_id TEXT PRIMARY KEY REFERENCES pools(id),height INTEGER NOT NULL,hash TEXT NOT NULL);
-- version 2
CREATE TABLE treasury_sync(singleton INTEGER PRIMARY KEY CHECK(singleton=1),observation TEXT NOT NULL);
-- version 3
CREATE TABLE treasury_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),facts TEXT NOT NULL,submission TEXT NOT NULL CHECK(submission IN ('PREPARED','BROADCAST_REQUESTED','BROADCAST','UNKNOWN','CONFIRMED')),attempts INTEGER NOT NULL DEFAULT 0);
-- version 4
CREATE TABLE IF NOT EXISTS funding_progress(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),operation_id TEXT NOT NULL UNIQUE,phase TEXT NOT NULL,quote BLOB,attempts INTEGER NOT NULL DEFAULT 0,next_poll INTEGER NOT NULL DEFAULT 0,last_error TEXT,turn INTEGER NOT NULL DEFAULT 0);
-- version 5
CREATE TABLE IF NOT EXISTS funding_refunds(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),address BLOB NOT NULL);
-- version 6
CREATE TABLE IF NOT EXISTS funding_recovery(operation_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES funding_jobs(id),phase TEXT NOT NULL,quote BLOB,refund BLOB);
-- version 7
CREATE TABLE IF NOT EXISTS refund_outputs(txid TEXT NOT NULL,output_index INTEGER NOT NULL,operation_id TEXT NOT NULL REFERENCES treasury_operations(id),amount INTEGER NOT NULL CHECK(amount>0),height INTEGER NOT NULL,credited INTEGER NOT NULL CHECK(credited>=0),PRIMARY KEY(txid,output_index));
-- version 8
CREATE TABLE IF NOT EXISTS expired_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),height INTEGER NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
-- version 9
CREATE TABLE IF NOT EXISTS funding_health(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),started_at INTEGER,error_streak INTEGER NOT NULL DEFAULT 0,timed_out INTEGER NOT NULL DEFAULT 0);
-- version 10
CREATE TABLE IF NOT EXISTS operation_network(operation_id TEXT PRIMARY KEY,recipient TEXT NOT NULL);
-- version 11
CREATE TABLE payment_resolutions(attempt_id TEXT PRIMARY KEY REFERENCES payment_attempts(id),outcome TEXT NOT NULL CHECK(outcome IN ('USED','EXPIRED_UNUSED')),height INTEGER NOT NULL,hash TEXT NOT NULL,block_time INTEGER NOT NULL);

-- version 12
CREATE TABLE funding_quote_refreshes(operation_id TEXT PRIMARY KEY REFERENCES funding_recovery(operation_id));
