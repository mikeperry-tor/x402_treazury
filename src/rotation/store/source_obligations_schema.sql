-- Ownership disputes reference the original signed journal; they carry no amounts.
-- Burial and historical observation eviction cannot release this ownership.
CREATE TABLE source_obligations(
 operation_id TEXT PRIMARY KEY REFERENCES treasury_operations(id),
 clear_pass_id TEXT REFERENCES source_support_passes(id)
);
CREATE VIEW active_source_obligations AS
 SELECT o.operation_id FROM source_obligations o
 LEFT JOIN source_support_passes p ON p.id=o.clear_pass_id
 WHERE COALESCE(p.activated,0)=0;
CREATE INDEX source_obligation_clear_pass ON source_obligations(clear_pass_id);
