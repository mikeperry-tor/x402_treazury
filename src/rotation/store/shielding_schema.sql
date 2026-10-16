-- Exact input provenance for internal shielding, not provider refund attribution.
CREATE TABLE shielding_operations(
 operation_id TEXT PRIMARY KEY REFERENCES outgoing(id),
 policy_pool_id TEXT NOT NULL REFERENCES pools(id),
 policy_wallet_id TEXT NOT NULL REFERENCES wallets(id),
 address BLOB NOT NULL,
 max_fee INTEGER NOT NULL CHECK(max_fee>0),
 automatic INTEGER NOT NULL CHECK(automatic IN (0,1))
);
CREATE TABLE shielding_inputs(
 operation_id TEXT NOT NULL REFERENCES shielding_operations(operation_id),
 txid TEXT NOT NULL,
 output_index INTEGER NOT NULL CHECK(output_index>=0 AND output_index<=4294967295),
 amount INTEGER NOT NULL CHECK(amount>0),
 script BLOB NOT NULL,
 PRIMARY KEY(operation_id,txid,output_index)
);
CREATE INDEX shielding_input_claim ON shielding_inputs(txid,output_index,operation_id);
CREATE TRIGGER immutable_shielding_operation BEFORE UPDATE ON shielding_operations
 BEGIN SELECT RAISE(ABORT,'immutable shielding operation'); END;
CREATE TRIGGER immutable_shielding_input BEFORE UPDATE ON shielding_inputs
 BEGIN SELECT RAISE(ABORT,'immutable shielding input'); END;
CREATE TRIGGER retained_shielding_operation BEFORE DELETE ON shielding_operations
 BEGIN SELECT RAISE(ABORT,'retained shielding operation'); END;
CREATE TRIGGER retained_shielding_input BEFORE DELETE ON shielding_inputs
 BEGIN SELECT RAISE(ABORT,'retained shielding input'); END;
