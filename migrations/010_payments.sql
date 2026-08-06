-- payment batches (SEPA pain.001 export) + contact IBANs
ALTER TABLE contacts ADD COLUMN iban TEXT;

CREATE TABLE payment_batches (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_date  TEXT NOT NULL,                 -- requested execution date (ReqdExctnDt)
  debit_iban  TEXT NOT NULL,                 -- company IBAN at creation
  debit_name  TEXT NOT NULL,                 -- company name at creation
  total_cents INTEGER NOT NULL DEFAULT 0,
  status      TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','exported','paid')),
  msg_id      TEXT UNIQUE,                   -- set at export; guards against double payment
  file_hash   TEXT,                          -- sha256 of the exported file
  schema      TEXT,                          -- 'pain.001.001.03' | 'pain.001.001.09'
  created_by  TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  exported_at TEXT
);

CREATE TABLE payment_batch_lines (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id     INTEGER NOT NULL REFERENCES payment_batches(id) ON DELETE CASCADE,
  contact_id   INTEGER REFERENCES contacts(id),
  name         TEXT NOT NULL,                -- beneficiary name
  iban         TEXT NOT NULL,
  amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
  reference    TEXT,                         -- remittance info, max 140 chars
  status       TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','paid'))
);

CREATE INDEX idx_batch_lines_batch ON payment_batch_lines(batch_id);

-- payables: purchase invoices (crediteuren open items) that can be paid via
-- a payment batch. payment_method 'transfer' = SEPA credit transfer (batch
-- eligible); 'direct_debit' = incasso, collected by the vendor, excluded
-- from batches. Status: unpaid -> in_batch (added to a draft batch) -> paid.
CREATE TABLE payables (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  contact_id     INTEGER NOT NULL REFERENCES contacts(id),
  invoice_ref    TEXT NOT NULL,              -- vendor invoice number
  date           TEXT NOT NULL,              -- invoice date
  due_date       TEXT,
  amount_cents   INTEGER NOT NULL CHECK (amount_cents > 0),
  payment_method TEXT NOT NULL DEFAULT 'transfer' CHECK (payment_method IN ('transfer','direct_debit')),
  status         TEXT NOT NULL DEFAULT 'unpaid' CHECK (status IN ('unpaid','in_batch','paid')),
  entry_id       INTEGER REFERENCES journal_entries(id),  -- optional: linked expense booking
  batch_line_id  INTEGER REFERENCES payment_batch_lines(id),
  created_by     TEXT NOT NULL,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_payables_status ON payables(status);
