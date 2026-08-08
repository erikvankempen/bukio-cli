-- v0.14.0: SEPA direct debit (incasso) — B2B/CORE mandate register +
-- pain.008 export. Batches gain a kind; lines carry the mandate reference.
CREATE TABLE sepa_mandates (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  contact_id   INTEGER NOT NULL REFERENCES contacts(id),
  mandate_ref  TEXT NOT NULL,                 -- SEPA mandate ID, max 35 chars
  mandate_date TEXT NOT NULL,                 -- signature date YYYY-MM-DD
  scheme       TEXT NOT NULL DEFAULT 'core' CHECK (scheme IN ('core','b2b')),
  created_by   TEXT NOT NULL,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(contact_id, mandate_ref)
);

ALTER TABLE payment_batches ADD COLUMN batch_kind TEXT NOT NULL DEFAULT 'transfer'
  CHECK (batch_kind IN ('transfer','direct_debit'));

ALTER TABLE payment_batch_lines ADD COLUMN mandate_ref TEXT;   -- direct-debit lines only
ALTER TABLE payment_batch_lines ADD COLUMN mandate_seq TEXT;   -- 'FRST' | 'RCUR'
ALTER TABLE payment_batch_lines ADD COLUMN mandate_date TEXT;  -- DtOfSgntr snapshot
ALTER TABLE payment_batch_lines ADD COLUMN scheme TEXT;        -- 'core' | 'b2b' snapshot

CREATE INDEX idx_mandates_contact ON sepa_mandates(contact_id);
