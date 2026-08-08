-- v0.14.0: inbound e-invoice intake (bukio import invoice) — payables gain
-- source tracking for idempotency (source 'ubl' + source_ref '<supplier>:<nr>').
ALTER TABLE payables ADD COLUMN source TEXT;      -- 'manual' (default) | 'ubl'
ALTER TABLE payables ADD COLUMN source_ref TEXT;  -- natural key for re-import dedupe

CREATE INDEX idx_payables_source ON payables(source, source_ref);
