-- 024: allow contra-equity accounts (US profile, Phase B)
--
-- The US default chart follows the QuickBooks convention with an
-- Owner's draws account (type 'equity' with a DEBIT normal balance — a
-- contra-equity). The CHECK (extended by 023 for contra-assets) still
-- required equity+credit. Rebuild the accounts table with the CHECK
-- extended once more; all rows are copied verbatim. Same PRAGMA
-- foreign_keys = OFF pattern as 023 (postings reference accounts).
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
    (type = 'asset' AND normal_balance = 'credit') OR
    (type = 'equity' AND normal_balance = 'debit')
  )
);
INSERT INTO accounts_new (id, code, name, type, taxonomy_code, normal_balance, active, created_at, taxonomy)
  SELECT id, code, name, type, taxonomy_code, normal_balance, active, created_at, taxonomy FROM accounts;
DROP TABLE accounts;
ALTER TABLE accounts_new RENAME TO accounts;
COMMIT;
PRAGMA foreign_keys = ON;
