-- 0001_init.sql — the Postgres control DB (#7), applied idempotently at
-- every boot (CREATE IF NOT EXISTS / CREATE OR REPLACE / ON CONFLICT DO
-- NOTHING throughout — re-running is a no-op).
--
-- Two HashiCorp patterns, literally (docs/hashicorp-steering.md):
--   §5 Boundary: app_valid_state constrains lifecycle transitions AT THE
--      DATABASE (trigger on apps + composite FK from the history table),
--      and app_state_history is an append-only temporal record an auditor
--      can trust even against application bugs.
--   §4 Waypoint: operations are upserted RUNNING before work begins; the
--      row here is the crash-visible record of every agent action.
--
-- Grants (documented intent; dev/staging runs as the owner role): the
-- application role receives INSERT+SELECT only on app_state_history and
-- audit_events — no UPDATE, no DELETE. The append-only triggers below
-- enforce the same invariant even for the owner.

-- The doctor's app records: the full record as jsonb (the in-memory
-- AppRecord is the source of truth for shape), with the lifecycle stage
-- extracted into a real column so the database can enforce transitions.
CREATE TABLE IF NOT EXISTS apps (
  app_id     TEXT PRIMARY KEY,
  stage      TEXT NOT NULL CHECK (stage IN ('sandbox', 'live')),
  record     JSONB NOT NULL,
  updated_at BIGINT NOT NULL
);

-- Boundary's session_valid_state, for the app lifecycle. Seeded below from
-- the SAME transition set as src/state.rs::VALID_STAGE_TRANSITIONS — a test
-- (tests/store_contract.rs) asserts the seed and the Rust const match.
CREATE TABLE IF NOT EXISTS app_valid_state (
  prior_state   TEXT NOT NULL,
  current_state TEXT NOT NULL,
  PRIMARY KEY (prior_state, current_state)
);

INSERT INTO app_valid_state (prior_state, current_state) VALUES
  ('sandbox', 'live'),
  ('live', 'sandbox')
ON CONFLICT DO NOTHING;

-- Append-only temporal record of every stage change. prior_state is NULL
-- exactly once per app (creation); every non-creation row is forced legal
-- by the composite FK into app_valid_state. INSERT only — see the
-- append-only trigger below; no UPDATE/DELETE grants are ever issued.
CREATE TABLE IF NOT EXISTS app_state_history (
  id            BIGSERIAL PRIMARY KEY,
  app_id        TEXT NOT NULL,
  prior_state   TEXT,
  current_state TEXT NOT NULL,
  at            BIGINT NOT NULL,
  operation_id  TEXT,
  FOREIGN KEY (prior_state, current_state)
    REFERENCES app_valid_state (prior_state, current_state)
);

-- Waypoint-style operation rows, upserted by op_id (§4). status spellings
-- mirror src/state.rs::OP_STATUSES (asserted by the same contract test).
CREATE TABLE IF NOT EXISTS operations (
  ord         BIGSERIAL,
  op_id       TEXT PRIMARY KEY,
  app_id      TEXT NOT NULL,
  kind        TEXT NOT NULL,
  status      TEXT NOT NULL
    CHECK (status IN ('running', 'success', 'escalated', 'failed')),
  record      JSONB NOT NULL,
  started_at  BIGINT NOT NULL,
  finished_at BIGINT
);

-- The audit stream, append-only (INSERT+SELECT only; no UPDATE/DELETE
-- grants). seq is written explicitly by the control plane so the durable
-- stream and the in-memory stream are the same numbering.
--
-- #8: `sensitive` holds doctor-authored free text as hmac-sha256:<hex>
-- (Vault salted-HMAC — the platform-wide, non-disclosable form);
-- `sensitive_pt` is the Boundary-style paired plaintext for the owning
-- tenant's own view. The control DB is tenant-scoped storage (apps.record
-- already carries the prompt in plaintext); every cross-tenant surface
-- serializes only the HMAC form (decision 0004).
CREATE TABLE IF NOT EXISTS audit_events (
  seq          BIGSERIAL PRIMARY KEY,
  at           BIGINT NOT NULL,
  actor        TEXT NOT NULL,
  action       TEXT NOT NULL,
  detail       TEXT NOT NULL,
  app_id       TEXT,
  sensitive    JSONB NOT NULL DEFAULT '{}'::jsonb,
  sensitive_pt JSONB NOT NULL DEFAULT '{}'::jsonb
);

-- Idempotent upgrade for control DBs initialized before #8.
ALTER TABLE audit_events
  ADD COLUMN IF NOT EXISTS sensitive JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE audit_events
  ADD COLUMN IF NOT EXISTS sensitive_pt JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Control-plane metadata (id-minting counter survives restarts).
CREATE TABLE IF NOT EXISTS control_meta (
  key   TEXT PRIMARY KEY,
  value BIGINT NOT NULL
);

-- §5, enforced: an UPDATE that changes apps.stage must name a pair present
-- in app_valid_state — application bugs cannot half-promote an app.
CREATE OR REPLACE FUNCTION enforce_app_stage_transition() RETURNS trigger AS $$
BEGIN
  IF OLD.stage IS DISTINCT FROM NEW.stage AND NOT EXISTS (
    SELECT 1 FROM app_valid_state
     WHERE prior_state = OLD.stage AND current_state = NEW.stage
  ) THEN
    RAISE EXCEPTION 'illegal stage transition % -> % for app %',
      OLD.stage, NEW.stage, NEW.app_id;
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS app_stage_transition ON apps;
CREATE TRIGGER app_stage_transition
  BEFORE UPDATE ON apps
  FOR EACH ROW EXECUTE FUNCTION enforce_app_stage_transition();

-- Append-only, enforced in the schema itself (not just by grant hygiene).
CREATE OR REPLACE FUNCTION reject_mutation() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % rejected', TG_TABLE_NAME, TG_OP;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS app_state_history_append_only ON app_state_history;
CREATE TRIGGER app_state_history_append_only
  BEFORE UPDATE OR DELETE ON app_state_history
  FOR EACH ROW EXECUTE FUNCTION reject_mutation();

DROP TRIGGER IF EXISTS audit_events_append_only ON audit_events;
CREATE TRIGGER audit_events_append_only
  BEFORE UPDATE OR DELETE ON audit_events
  FOR EACH ROW EXECUTE FUNCTION reject_mutation();
