-- Return inventory shares the source pass; provider poll timestamps and opaque hints do not invalidate it.
CREATE INDEX return_confirmed_inventory ON treasury_operations(id) WHERE submission='CONFIRMED';
CREATE TRIGGER return_inventory_refund_outputs_insert AFTER INSERT ON refund_outputs BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_refund_outputs_delete AFTER DELETE ON refund_outputs BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
END;
CREATE TRIGGER return_inventory_refund_outputs_update AFTER UPDATE ON refund_outputs WHEN OLD.txid IS NOT NEW.txid OR OLD.output_index IS NOT NEW.output_index OR OLD.operation_id IS NOT NEW.operation_id OR OLD.amount IS NOT NEW.amount OR OLD.height IS NOT NEW.height OR OLD.credited IS NOT NEW.credited BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_delivery_hints_insert AFTER INSERT ON funding_delivery_hints BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_delivery_hints_delete AFTER DELETE ON funding_delivery_hints BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
END;
CREATE TRIGGER return_inventory_funding_delivery_hints_update AFTER UPDATE ON funding_delivery_hints WHEN OLD.operation_id IS NOT NEW.operation_id OR OLD.job_id IS NOT NEW.job_id OR OLD.quote_digest IS NOT NEW.quote_digest OR OLD.declared_return IS NOT NEW.declared_return OR OLD.return_binding IS NOT NEW.return_binding OR OLD.return_conflict IS NOT NEW.return_conflict BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_recovery_insert AFTER INSERT ON funding_recovery BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_recovery_delete AFTER DELETE ON funding_recovery BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
END;
CREATE TRIGGER return_inventory_funding_recovery_update AFTER UPDATE ON funding_recovery WHEN OLD.operation_id IS NOT NEW.operation_id OR OLD.job_id IS NOT NEW.job_id OR OLD.refund IS NOT NEW.refund BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_refunds_insert AFTER INSERT ON funding_refunds BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT operation_id FROM funding_progress WHERE job_id=NEW.job_id);
END;
CREATE TRIGGER return_inventory_funding_refunds_delete AFTER DELETE ON funding_refunds BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT operation_id FROM funding_progress WHERE job_id=OLD.job_id);
END;
CREATE TRIGGER return_inventory_funding_refunds_update AFTER UPDATE ON funding_refunds WHEN OLD.job_id IS NOT NEW.job_id OR OLD.address IS NOT NEW.address BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT operation_id FROM funding_progress WHERE job_id=OLD.job_id);
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT operation_id FROM funding_progress WHERE job_id=NEW.job_id);
END;
CREATE TRIGGER return_inventory_funding_progress_insert AFTER INSERT ON funding_progress BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
CREATE TRIGGER return_inventory_funding_progress_delete AFTER DELETE ON funding_progress BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
END;
CREATE TRIGGER return_inventory_funding_progress_update AFTER UPDATE ON funding_progress WHEN OLD.operation_id IS NOT NEW.operation_id OR OLD.quote IS NOT NEW.quote OR OLD.accepted_output IS NOT NEW.accepted_output OR OLD.accepted_recipient IS NOT NEW.accepted_recipient BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT OLD.operation_id);
 UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id IN (SELECT NEW.operation_id);
END;
