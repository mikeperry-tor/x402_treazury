-- Canonical source observations are projections of the original outgoing journal.
-- Inactive passes have no accounting authority; only final sync publication selects one.
CREATE TABLE source_support_passes(
 id TEXT PRIMARY KEY,
 base_revision INTEGER NOT NULL,
 anchor_height INTEGER NOT NULL CHECK(anchor_height>0),
 anchor_hash BLOB NOT NULL CHECK(length(anchor_hash)=32),
 inventory_revision INTEGER NOT NULL,
 boundary INTEGER NOT NULL,
 after_id TEXT NOT NULL DEFAULT '',
 complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)),
 activated INTEGER NOT NULL DEFAULT 0 CHECK(activated IN (0,1))
);
CREATE TABLE source_support_current(
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 inventory_revision INTEGER NOT NULL DEFAULT 0 CHECK(inventory_revision>=0 AND inventory_revision<9223372036854775807),
 pass_id TEXT REFERENCES source_support_passes(id),
 snapshot_revision INTEGER
);
INSERT INTO source_support_current(singleton) VALUES(1);
CREATE TABLE source_support_rows(
 pass_id TEXT NOT NULL REFERENCES source_support_passes(id),
 operation_id TEXT NOT NULL,
 facts TEXT NOT NULL,
 outgoing_revision INTEGER NOT NULL,
 disposition TEXT NOT NULL CHECK(disposition IN ('CONFIRMED','UNAVAILABLE','CONFLICT','EXPIRED')),
 confirmed_height INTEGER,
 returned_amount TEXT CHECK(returned_amount IS NULL OR (length(returned_amount) BETWEEN 1 AND 78 AND returned_amount NOT GLOB '*[^0-9]*')),
 return_issue INTEGER NOT NULL DEFAULT 1 CHECK(return_issue IN (0,1)),
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 PRIMARY KEY(pass_id,operation_id),
 CHECK((disposition='CONFIRMED')=(confirmed_height IS NOT NULL))
);
CREATE TRIGGER source_support_treasury_operations_insert AFTER INSERT ON treasury_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_treasury_operations_update AFTER UPDATE ON treasury_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_treasury_operations_delete AFTER DELETE ON treasury_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
END;
CREATE TRIGGER source_support_outgoing_insert AFTER INSERT ON outgoing BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_outgoing_update AFTER UPDATE ON outgoing BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_outgoing_delete AFTER DELETE ON outgoing BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
END;
CREATE TRIGGER source_support_expired_operations_insert AFTER INSERT ON expired_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_expired_operations_update AFTER UPDATE ON expired_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=NEW.id;
END;
CREATE TRIGGER source_support_expired_operations_delete AFTER DELETE ON expired_operations BEGIN
 UPDATE source_support_current SET inventory_revision=inventory_revision+1 WHERE singleton=1;
 UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=OLD.id;
END;
