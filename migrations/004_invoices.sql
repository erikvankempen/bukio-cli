-- 004_invoices.sql — invoicing module (Phase 3, FR3)
-- Outgoing invoices compliant with the 12 verplichte factuurvereisten.
-- The VAT rate is snapshotted per line at creation (vat_rate_bp), so UBL and
-- the OB readout stay correct even if the vat_codes table later changes.

ALTER TABLE company ADD COLUMN address TEXT;
ALTER TABLE company ADD COLUMN postal_code TEXT;
ALTER TABLE company ADD COLUMN city TEXT;

CREATE TABLE contacts (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL,
  address    TEXT,
  postal_code TEXT,
  city       TEXT,
  country    TEXT NOT NULL DEFAULT 'NL',
  email      TEXT,
  vat_id     TEXT,   -- btw-id of the customer (required when btw verlegd)
  kvk        TEXT,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE invoices (
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
  created_by           TEXT NOT NULL,
  created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE invoice_lines (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  invoice_id       INTEGER NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
  line_no          INTEGER NOT NULL,
  description      TEXT NOT NULL,
  quantity         INTEGER NOT NULL DEFAULT 1 CHECK (quantity > 0),
  unit_price_cents INTEGER NOT NULL CHECK (unit_price_cents > 0),
  vat_code         TEXT,                     -- '21', '9', '0', 'V', 'R', 'RE', 'M', 'P'
  vat_rate_bp      INTEGER NOT NULL DEFAULT 0,   -- snapshot at creation
  amount_cents     INTEGER NOT NULL,         -- quantity * unit_price
  vat_amount_cents INTEGER NOT NULL          -- round(amount * rate_bp / 10000)
);

CREATE TABLE invoice_payments (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  invoice_id   INTEGER NOT NULL REFERENCES invoices(id),
  date         TEXT NOT NULL,
  amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
  method       TEXT NOT NULL DEFAULT 'bank',
  bank_tx_id   INTEGER REFERENCES bank_transactions(id),
  created_by   TEXT NOT NULL,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_invoices_contact ON invoices(contact_id);
CREATE INDEX idx_invoices_status ON invoices(status);
CREATE INDEX idx_invoice_lines_inv ON invoice_lines(invoice_id);
