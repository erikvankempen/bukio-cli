-- 002_bank_vat.sql — bank module + optional VAT module (Phase 2)
-- Core ledger stays VAT-agnostic: the postings.vat_* columns are nullable
-- and inert when the company's vat_module flag is off. All VAT semantics
-- live in src/vat/ — the core engine only persists what it is given.

CREATE TABLE bank_accounts (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  iban         TEXT NOT NULL UNIQUE,
  name         TEXT,
  account_code TEXT NOT NULL DEFAULT '1100' REFERENCES accounts(code),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE bank_transactions (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  bank_account_id INTEGER NOT NULL REFERENCES bank_accounts(id),
  date            TEXT NOT NULL,          -- ISO yyyy-mm-dd
  amount_cents    INTEGER NOT NULL CHECK (amount_cents != 0),
  counterparty    TEXT,
  description     TEXT,
  iban_counter    TEXT,
  hash            TEXT NOT NULL UNIQUE,   -- sha256(iban|date|amount|counterparty|description)
  state           TEXT NOT NULL DEFAULT 'unmatched' CHECK (state IN ('unmatched','matched','ignored')),
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_bank_tx_account ON bank_transactions(bank_account_id);
CREATE INDEX idx_bank_tx_state   ON bank_transactions(state);

CREATE TABLE reconciliations (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  bank_tx_id  INTEGER NOT NULL REFERENCES bank_transactions(id),
  target_type TEXT NOT NULL CHECK (target_type IN ('entry','invoice')),
  target_id   INTEGER NOT NULL,
  method      TEXT NOT NULL CHECK (method IN ('exact','fuzzy','rule','manual','agent')),
  confidence  REAL,
  created_by  TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(bank_tx_id, target_type, target_id)
);
CREATE INDEX idx_recon_tx    ON reconciliations(bank_tx_id);
CREATE INDEX idx_recon_entry ON reconciliations(target_type, target_id);

-- VAT module (only used when company.vat_module = 1)
CREATE TABLE vat_codes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  code        TEXT NOT NULL UNIQUE,        -- 21 | 9 | 0 | V | R | RE | M | P
  rate_bp     INTEGER NOT NULL,            -- basis points: 2100 = 21%
  type        TEXT NOT NULL CHECK (type IN ('standard','exempt','reverse','margin','private')),
  eu_reverse  INTEGER NOT NULL DEFAULT 0,  -- 1 = reverse charge within the EU (OB field 4b)
  description TEXT
);

CREATE TABLE vat_returns (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  type        TEXT NOT NULL CHECK (type IN ('OB','ICP')),
  period      TEXT NOT NULL,               -- '2026-Q2' or '2026-07'
  status      TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','filed')),
  fields_json TEXT,
  filed_at    TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(type, period)
);

ALTER TABLE postings ADD COLUMN vat_code_id INTEGER REFERENCES vat_codes(id);
ALTER TABLE postings ADD COLUMN vat_amount_cents INTEGER;
