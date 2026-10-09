-- The global dataVersion: one row holding one counter, bumped by the applier
-- in the very transaction that applies a command, so a version and its data
-- commit together. Not a SEQUENCE: a rolled-back apply must leave no gap.
-- The single row is the counter's structure, not data; it starts at 0.
CREATE TABLE dataguard_version (
  singleton boolean PRIMARY KEY DEFAULT true
    CONSTRAINT dataguard_version_singleton CHECK (singleton),
  version bigint NOT NULL DEFAULT 0
    CONSTRAINT dataguard_version_nonnegative CHECK (version >= 0)
);

INSERT INTO dataguard_version DEFAULT VALUES;

REVOKE ALL ON dataguard_version FROM PUBLIC;

-- All the read role may know of the engine: the current version. Not the
-- queue, not the idempotency keys, not the payloads.
CREATE VIEW data_version AS
  SELECT version AS "dataVersion" FROM dataguard_version;

REVOKE ALL ON data_version FROM PUBLIC;

GRANT SELECT ON data_version TO app_lecture;
