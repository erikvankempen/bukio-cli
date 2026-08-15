-- 023: allow contra-asset accounts (GB profile, Phase B)
--
-- The GB default chart follows the UK convention of accumulated-depreciation
-- contra-asset accounts (type 'asset' with a CREDIT normal balance — e.g.
-- 1600/1800). The CHECK in 001 only allowed asset+debit. This migration
-- rebuilds the accounts table with the CHECK extended (contra-asset credit
-- added); all rows are copied verbatim, including the taxonomy discriminator
-- added in 021/022. Runs with PRAGMA foreign_keys = OFF because postings
-- reference accounts — the runner executes PRAGMA migrations outside a
-- transaction and restores the flag afterwards.
PRAGMA foreign_keys = OFF;
BEGIN;
CREATE TABLE accounts_new (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  code           TEXT NOT NULL UNIQUE,
  name           TEXT NOT NULL,
  type           TEXT NOT NULL CHECK (type IN ('asset','liability','equity','income','expense')),
  taxonomy_code       TEXT,
  normal_balance TEXT NOT NULL CHECK (normal_balance IN ('debit','credit')),
  active         INTEGER NOT NULL DEFAULT 1,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  taxonomy TEXT,
  CHECK (
    (type IN ('asset','expense') AND normal_balance = 'debit') OR
    (type IN ('liability','equity','income') AND normal_balance = 'credit') OR
    (type = 'asset' AND normal_balance = 'credit')
  )
);
INSERT INTO accounts_new (id, code, name, type, taxonomy_code, normal_balance, active, created_at, taxonomy)
  SELECT id, code, name, type, taxonomy_code, normal_balance, active, created_at, taxonomy FROM accounts;
DROP TABLE accounts;
ALTER TABLE accounts_new RENAME TO accounts;
COMMIT;
PRAGMA foreign_keys = ON;
