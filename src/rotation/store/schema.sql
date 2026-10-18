-- Fresh schema16 only. Legacy conversion has a separate strict preflight.
-- Older schema16 stores may retain unused payment_anchors and funding_progress.turn.
-- They are never read/written by runtime code; no destructive cleanup is needed.
CREATE TABLE instance(id TEXT PRIMARY KEY,version INTEGER NOT NULL CHECK(version=1),birthday INTEGER NOT NULL,network TEXT NOT NULL CHECK(network='mainnet'),account INTEGER NOT NULL CHECK(account=0));
CREATE TABLE snapshots(revision INTEGER PRIMARY KEY,bytes BLOB NOT NULL);
CREATE TABLE pools(id TEXT PRIMARY KEY,name TEXT UNIQUE NOT NULL,generation INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)), payment_limit TEXT, retirement_tolerance TEXT);
CREATE TABLE wallets(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),sequence INTEGER NOT NULL,address TEXT UNIQUE NOT NULL,key BLOB NOT NULL,target TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('ALLOCATED','READY','ACTIVE','RETAINED','RETIRED')),balance TEXT NOT NULL DEFAULT '0',block_hash TEXT,block_height INTEGER, financial_height INTEGER, financial_hash TEXT, debit_height INTEGER, debit_hash TEXT, evidence_repair TEXT, unresolved_exposure TEXT NOT NULL DEFAULT '0',UNIQUE(pool_id,sequence));
CREATE TABLE funding_jobs(id TEXT PRIMARY KEY,wallet_id TEXT UNIQUE NOT NULL REFERENCES wallets(id),state TEXT NOT NULL CHECK(state IN ('QUEUED','COMPLETE')),target TEXT NOT NULL, readiness_operation TEXT, readiness_requirement TEXT, readiness_capacity TEXT, readiness_hash TEXT, readiness_height INTEGER, readiness_support TEXT, readiness_latest_hash TEXT, readiness_endpoint TEXT, readiness_checkpoint_hash TEXT, readiness_disputed TEXT, completion_outcome TEXT, pool_id TEXT NOT NULL REFERENCES pools(id), allocation_kind TEXT NOT NULL CHECK(allocation_kind IN ('small','large')), claim_generation INTEGER NOT NULL CHECK(claim_generation>=0));
CREATE TABLE budget_entries(id TEXT PRIMARY KEY,pool_id TEXT REFERENCES pools(id),day INTEGER NOT NULL,original_day INTEGER NOT NULL,requested INTEGER NOT NULL CHECK(requested>0),reserved INTEGER NOT NULL CHECK(reserved>=0),consumed INTEGER NOT NULL CHECK(consumed>=0));
CREATE TABLE outgoing(id TEXT PRIMARY KEY REFERENCES budget_entries(id),state TEXT NOT NULL CHECK(state IN ('PREPARED','RESOLVED')),raw BLOB NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
CREATE TABLE payment_attempts(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),wallet_id TEXT NOT NULL REFERENCES wallets(id),generation INTEGER NOT NULL,amount TEXT NOT NULL,requirements_hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('POSSIBLY_SUBMITTED','RESOLVED')),payer TEXT,payee TEXT,nonce TEXT,valid_after INTEGER,valid_before INTEGER, reconciliation_sequence INTEGER,UNIQUE(wallet_id,nonce));
CREATE TABLE treasury_sync(singleton INTEGER PRIMARY KEY CHECK(singleton=1),observation TEXT NOT NULL);
CREATE TABLE treasury_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),facts TEXT NOT NULL,submission TEXT NOT NULL CHECK(submission IN ('PREPARED','BROADCAST_REQUESTED','BROADCAST','UNKNOWN','CONFIRMED')),attempts INTEGER NOT NULL DEFAULT 0);
CREATE TABLE funding_progress(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),operation_id TEXT NOT NULL UNIQUE,phase TEXT NOT NULL,quote BLOB,attempts INTEGER NOT NULL DEFAULT 0,next_poll INTEGER NOT NULL DEFAULT 0,last_error TEXT, provider_success INTEGER NOT NULL DEFAULT 0, provider_failed INTEGER NOT NULL DEFAULT 0, provider_refunded INTEGER NOT NULL DEFAULT 0, provider_incomplete INTEGER NOT NULL DEFAULT 0, provider_observed_at INTEGER, quote_target TEXT, quote_ceiling TEXT, accepted_output TEXT, accepted_recipient TEXT);
CREATE TABLE funding_refunds(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),address BLOB NOT NULL);
CREATE TABLE funding_recovery(operation_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES funding_jobs(id),phase TEXT NOT NULL,quote BLOB,refund BLOB, provider_success INTEGER NOT NULL DEFAULT 0, provider_failed INTEGER NOT NULL DEFAULT 0, provider_refunded INTEGER NOT NULL DEFAULT 0, provider_incomplete INTEGER NOT NULL DEFAULT 0, provider_observed_at INTEGER, quote_target TEXT, quote_ceiling TEXT, accepted_output TEXT, accepted_recipient TEXT);
CREATE TABLE refund_outputs(txid TEXT NOT NULL,output_index INTEGER NOT NULL,operation_id TEXT NOT NULL REFERENCES treasury_operations(id),amount INTEGER NOT NULL CHECK(amount>0),height INTEGER NOT NULL,credited INTEGER NOT NULL CHECK(credited>=0),PRIMARY KEY(txid,output_index));
CREATE TABLE expired_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),height INTEGER NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
CREATE TABLE funding_health(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),started_at INTEGER,error_streak INTEGER NOT NULL DEFAULT 0,timed_out INTEGER NOT NULL DEFAULT 0);
CREATE TABLE operation_network(operation_id TEXT PRIMARY KEY,recipient TEXT NOT NULL);
CREATE TABLE payment_resolutions(attempt_id TEXT PRIMARY KEY REFERENCES payment_attempts(id),outcome TEXT NOT NULL CHECK(outcome IN ('USED','EXPIRED_UNUSED')),height INTEGER NOT NULL,hash TEXT NOT NULL,block_time INTEGER NOT NULL, support TEXT);
CREATE TABLE funding_quote_refreshes(operation_id TEXT PRIMARY KEY REFERENCES funding_recovery(operation_id));
CREATE TABLE payment_admission_evidence(attempt_id TEXT PRIMARY KEY REFERENCES payment_attempts(id),block_height INTEGER,block_hash TEXT,balance TEXT,reserved_before TEXT,valid_until TEXT, support TEXT, latest_height INTEGER, latest_hash TEXT, endpoint TEXT, payer_role TEXT NOT NULL CHECK(payer_role IN ('ACTIVE','RETAINED','READY')), reserve_slot TEXT CHECK(reserve_slot IN ('small','large')), claim_generation INTEGER, CHECK((payer_role='READY' AND reserve_slot IS NOT NULL AND claim_generation IS NOT NULL AND claim_generation>=0) OR (payer_role!='READY' AND reserve_slot IS NULL AND claim_generation IS NULL)));
CREATE TABLE reserve_claims(
 pool_id TEXT NOT NULL REFERENCES pools(id),
 slot TEXT NOT NULL CHECK(slot IN ('small','large')),
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
 generation INTEGER NOT NULL DEFAULT 0 CHECK(generation>=0),
 wallet_id TEXT UNIQUE REFERENCES wallets(id),
 target TEXT NOT NULL,
 maximum_output TEXT NOT NULL,
 PRIMARY KEY(pool_id,slot));
CREATE TABLE wallet_retirements(
 wallet_id TEXT PRIMARY KEY REFERENCES wallets(id),
 pool_id TEXT NOT NULL REFERENCES pools(id),
 confirmed_amount TEXT NOT NULL,
 latest_amount TEXT NOT NULL,
 confirmed_height INTEGER NOT NULL,
 confirmed_hash TEXT NOT NULL,
 latest_height INTEGER NOT NULL,
 latest_hash TEXT NOT NULL,
 accepted_maximum TEXT NOT NULL,
 observed_bound TEXT NOT NULL,
 support TEXT NOT NULL,
 endpoint TEXT NOT NULL,
 checkpoint_hash TEXT,
 disputed TEXT,
 reason TEXT NOT NULL CHECK(reason='automatic'),
 recorded_at INTEGER NOT NULL);
CREATE INDEX wallet_role_sequence ON wallets(pool_id,role,sequence);
CREATE UNIQUE INDEX one_active ON wallets(pool_id) WHERE role='ACTIVE';
CREATE UNIQUE INDEX one_outgoing ON outgoing((1)) WHERE state='PREPARED';
CREATE UNIQUE INDEX payment_reconciliation_sequence ON payment_attempts(reconciliation_sequence);
CREATE INDEX payment_pending_sequence ON payment_attempts(pool_id,state,reconciliation_sequence);
CREATE INDEX payment_wallet_state ON payment_attempts(wallet_id,state);
CREATE INDEX admission_confirmed_support ON payment_admission_evidence(block_hash,attempt_id);
CREATE INDEX admission_latest_support ON payment_admission_evidence(latest_hash,attempt_id);
CREATE INDEX resolution_anchor_support ON payment_resolutions(hash,attempt_id);
CREATE INDEX refund_operation_outputs ON refund_outputs(operation_id);
CREATE INDEX funding_readiness_endpoint ON funding_jobs(readiness_endpoint,id);
CREATE INDEX funding_readiness_checkpoint ON funding_jobs(readiness_checkpoint_hash,id);
CREATE INDEX funding_readiness_confirmed ON funding_jobs(readiness_hash,id);
CREATE INDEX funding_readiness_latest ON funding_jobs(readiness_latest_hash,id);
CREATE INDEX funding_unfinished_jobs ON funding_jobs(id) WHERE state!='COMPLETE';
CREATE INDEX funding_actionable_inventory ON funding_jobs(id) WHERE state!='COMPLETE' OR readiness_disputed IS NOT NULL;
CREATE INDEX retirement_confirmed_support ON wallet_retirements(confirmed_hash,wallet_id);
CREATE INDEX retirement_latest_support ON wallet_retirements(latest_hash,wallet_id);
CREATE INDEX retirement_endpoint_support ON wallet_retirements(endpoint,wallet_id);
CREATE INDEX retirement_checkpoint_support ON wallet_retirements(checkpoint_hash,wallet_id);
CREATE TRIGGER immutable_allocation BEFORE UPDATE OF pool_id,wallet_id,target,allocation_kind,claim_generation ON funding_jobs
 WHEN OLD.pool_id IS NOT NEW.pool_id OR OLD.wallet_id IS NOT NEW.wallet_id OR OLD.target IS NOT NEW.target OR OLD.allocation_kind IS NOT NEW.allocation_kind OR OLD.claim_generation IS NOT NEW.claim_generation
 BEGIN SELECT RAISE(ABORT,'immutable allocation'); END;
CREATE TRIGGER allocation_binding BEFORE INSERT ON funding_jobs
 WHEN NOT EXISTS(SELECT 1 FROM wallets WHERE id=NEW.wallet_id AND pool_id=NEW.pool_id)
 BEGIN SELECT RAISE(ABORT,'invalid allocation binding'); END;
PRAGMA user_version=17;

-- Scoped operational lookups must not scan retired or unrelated financial history.
CREATE INDEX wallet_live_sequence ON wallets(pool_id,sequence) WHERE role!='RETIRED';
CREATE INDEX funding_claim_lookup ON funding_jobs(pool_id,allocation_kind,claim_generation);
CREATE INDEX funding_recovery_job ON funding_recovery(job_id,operation_id);
