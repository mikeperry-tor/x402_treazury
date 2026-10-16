-- An operator declaration, never inferred from chain scans or a backup's age.
CREATE TABLE restored_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1));
CREATE TRIGGER restored_state_retained BEFORE DELETE ON restored_state BEGIN SELECT RAISE(ABORT,'retained restoration declaration'); END;
CREATE TRIGGER restored_state_immutable BEFORE UPDATE ON restored_state BEGIN SELECT RAISE(ABORT,'immutable restoration declaration'); END;
