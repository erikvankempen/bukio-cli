-- 025_invoice_language_full_localization.sql
--
-- Phase D + full-localization request: the rendered invoice PDF is fully
-- localised in every market language (en/nl/de/fr/da/fi/nb/sv/it/es/pt plus
-- the nl-be/fr-lu overrides). Migration 011 pinned the invoices.language
-- column to CHECK (language IN ('nl','en')) — that CHECK is now stale and
-- rejects valid document languages ('de', 'it', ...) at INSERT time.
--
-- SQLite cannot alter a column CHECK, so this rebuild drops it: the
-- invoices table is recreated WITHOUT the language constraint (all other
-- columns/CHECKs preserved). Invoice language validation now lives in the
-- engine (src/invoice/index.js, against the i18n TABLES registry).
--
-- Rebuild pattern per 021: declare `PRAGMA foreign_keys = OFF` and run
-- OUTSIDE the runner's transaction (the runner detects the pragma and
-- wraps the script in its own foreign_keys toggle); a crash mid-rebuild
-- rolls back atomically (DDL here is transactional in the runner's
-- BEGIN/COMMIT once FKs are off).

PRAGMA foreign_keys = OFF;

DROP TABLE IF EXISTS invoices_new;
CREATE TABLE invoices_new (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  invoice_number       TEXT UNIQUE,          -- assigned at finalize: YYYY-NNNN
  invoice_type         TEXT NOT NULL DEFAULT 'sales' CHECK (invoice_type IN ('sales','credit')),
  contact_id           INTEGER NOT NULL REFERENCES contacts(id),
  date                 TEXT NOT NULL,        -- factuurdatum
  due_date             TEXT,                 -- vervaldatum
  delivery_date        TEXT,                 -- datum levering/dienst (if different)
  description          TEXT,
  reference            TEXT,                 -- klantkenmerk / inkooporder
  status               TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','sent','paid','overdue','void')),
  credit_for_invoice_id INTEGER REFERENCES invoices(id),
  entry_id             INTEGER REFERENCES journal_entries(id),  -- booking entry (posted at finalize)
  currency             TEXT NOT NULL DEFAULT 'EUR',
  notes                TEXT,
  discount_type        TEXT CHECK (discount_type IN ('pct','amount') OR discount_type IS NULL),
  discount_value       INTEGER,
  language             TEXT NOT NULL DEFAULT 'nl',  -- any i18n table code (no CHECK; validated in the engine)
  created_by           TEXT NOT NULL,
  created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO invoices_new (
  id, invoice_number, invoice_type, contact_id, date, due_date, delivery_date,
  description, reference, status, credit_for_invoice_id, entry_id, currency,
  notes, discount_type, discount_value, language, created_by, created_at
)
SELECT
  id, invoice_number, invoice_type, contact_id, date, due_date, delivery_date,
  description, reference, status, credit_for_invoice_id, entry_id, currency,
  notes, discount_type, discount_value, language, created_by, created_at
FROM invoices;

DROP TABLE invoices;

ALTER TABLE invoices_new RENAME TO invoices;

CREATE INDEX idx_invoices_contact ON invoices(contact_id);
CREATE INDEX idx_invoices_status ON invoices(status);
