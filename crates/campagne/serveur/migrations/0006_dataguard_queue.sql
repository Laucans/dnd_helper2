-- The DataQueue: one row per command, from submission to its terminal state,
-- kept for good (replay and result lookup read it). Commands queue FIFO per
-- partition (`<Aggregate>/<id>`, `position`); across partitions the applier
-- follows `enqueue_seq`. `operation`, `payload` and `based_on` are JSON text,
-- not jsonb: jsonb would turn a level of `1e1` into the integer 10.
CREATE SEQUENCE dataguard_enqueue_seq;

CREATE TABLE dataguard_queue (
  command uuid PRIMARY KEY,
  enqueue_seq bigint NOT NULL CONSTRAINT dataguard_queue_enqueue_seq_key UNIQUE,
  data_capability text NOT NULL,
  author text NOT NULL,
  partition text NOT NULL,
  position bigint NOT NULL CONSTRAINT dataguard_queue_position_nonnegative CHECK (position >= 0),
  -- The campaign partition whose rows the invariants of this command read.
  scope text NOT NULL,
  mode text NOT NULL
    CONSTRAINT dataguard_queue_mode CHECK (mode IN ('confirm_on_stale', 'overwrite', 'relative')),
  touches text[] NOT NULL,
  -- `<Aggregate>/<id>.<field>`: the declared touches on the row, plus the
  -- cascade once applied. What staleness compares.
  effects text[] NOT NULL DEFAULT '{}',
  operation text NOT NULL,
  payload text NOT NULL,
  payload_digest text NOT NULL,
  idempotency_key text,
  based_on text NOT NULL,
  projection jsonb,
  your_value jsonb,
  state text NOT NULL
    CONSTRAINT dataguard_queue_state CHECK (state IN (
      'queued', 'awaiting_confirmation', 'confirmed', 'awaiting_review', 'parked',
      'applied', 'rejected', 'cancelled', 'expired')),
  confirmation jsonb,
  parked jsonb,
  parked_at timestamptz,
  requeued_from uuid,
  messages jsonb NOT NULL DEFAULT '[]',
  warnings text[] NOT NULL DEFAULT '{}',
  violations text[] NOT NULL DEFAULT '{}',
  data_version bigint,
  impact_plan jsonb,
  enqueued_at timestamptz NOT NULL,
  settled_at timestamptz,
  CONSTRAINT dataguard_queue_partition_position UNIQUE (partition, position),
  CONSTRAINT dataguard_queue_settled CHECK (
    (state IN ('applied', 'rejected', 'cancelled', 'expired')) = (settled_at IS NOT NULL)),
  CONSTRAINT dataguard_queue_version CHECK ((state = 'applied') = (data_version IS NOT NULL))
);

-- One command per key, per author and DataCapability.
CREATE UNIQUE INDEX dataguard_queue_idempotency
  ON dataguard_queue (author, data_capability, idempotency_key)
  WHERE idempotency_key IS NOT NULL;

CREATE INDEX dataguard_queue_pending ON dataguard_queue (state, enqueue_seq);

-- Staleness asks which commands touched a field of a row: `effects && $1`.
CREATE INDEX dataguard_queue_effects ON dataguard_queue USING gin (effects);

-- A hold on a partition: recorded and reported, never in a command's way.
CREATE TABLE dataguard_hold (
  id uuid PRIMARY KEY,
  partition text NOT NULL,
  author text NOT NULL,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL
);

REVOKE ALL ON dataguard_queue, dataguard_hold FROM PUBLIC;
REVOKE ALL ON SEQUENCE dataguard_enqueue_seq FROM PUBLIC;
