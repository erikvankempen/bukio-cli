-- 021_jurisdiction_profiles.sql — Phase A: jurisdiction-profile layer schema (additive part).
--
-- * company: add jurisdiction columns (country / base_currency / locale /
--   profile_version) and DROP the NL-only legal_form CHECK — legal-form
--   validation moves to the CLI layer via profile.meta.legalForms. SQLite
--   cannot ALTER a CHECK, so company is rebuilt (the standard procedure).
--   Nothing has an FK to the rebuilt tables, so foreign_keys stays ON and
--   the runner applies this file INSIDE its BEGIN/COMMIT transaction —
--   the whole migration is atomic and crash-retry-safe.
-- * accounts: add the taxonomy discriminator column, backfilled 'rgs' for
--   every existing row (uniform with createAccount's insert). (The
--   rgs_code -> taxonomy_code rename is
--   migration 022 — it is disruptive and must land atomically with the code
--   churn.)
-- * vat_returns / filings: widen `type` (drop the NL-only CHECK) — filing
--   types come from the profile.
-- * The company identifier renames (kvk -> registration_id, btw_id -> tax_id)
--   are migration 022 for the same reason: any column rename breaks every
--   code path that reads the old name until the code lands in the same change.
--
-- No triggers reference company / vat_returns / filings and nothing has an
-- FK to them, so the rebuilds need no trigger/FK surgery beyond the swap.
-- Because nothing references these tables, foreign_keys stays ON and the
-- whole migration runs INSIDE the runner's BEGIN/COMMIT transaction — a
-- crash mid-rebuild rolls back atomically instead of leaving a half-swapped
-- schema behind (the DROP TABLE IF EXISTS *_new guards below only matter
-- for a stale table from a pre-transactional failed attempt).

-- 1. company ---------------------------------------------------------------

ALTER TABLE company ADD COLUMN country TEXT NOT NULL DEFAULT 'NL';
ALTER TABLE company ADD COLUMN base_currency TEXT NOT NULL DEFAULT 'EUR';
ALTER TABLE company ADD COLUMN locale TEXT NOT NULL DEFAULT 'nl';
ALTER TABLE company ADD COLUMN profile_version INTEGER NOT NULL DEFAULT 1;

-- stale table from a previously failed attempt (defensive; the migration
-- now runs inside a transaction, so this should never trigger)
DROP TABLE IF EXISTS company_new;
CREATE TABLE company_new (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  name            TEXT NOT NULL,
  kvk             TEXT,
  legal_form      TEXT,
  btw_id          TEXT,
  iban            TEXT,
  vat_module      INTEGER NOT NULL DEFAULT 0,
  kor_flag        INTEGER NOT NULL DEFAULT 0,
  fiscal_year_end TEXT NOT NULL DEFAULT '12-31',
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  address         TEXT,
  postal_code     TEXT,
  city            TEXT,
  logo            BLOB,
  logo_mime       TEXT,
  country         TEXT NOT NULL DEFAULT 'NL',
  base_currency   TEXT NOT NULL DEFAULT 'EUR',
  locale          TEXT NOT NULL DEFAULT 'nl',
  profile_version INTEGER NOT NULL DEFAULT 1
);
INSERT INTO company_new (
  id, name, kvk, legal_form, btw_id, iban, vat_module, kor_flag,
  fiscal_year_end, created_at, updated_at, address, postal_code, city, logo,
  logo_mime, country, base_currency, locale, profile_version
)
SELECT
  id, name, kvk, legal_form, btw_id, iban, vat_module, kor_flag,
  fiscal_year_end, created_at, updated_at, address, postal_code, city, logo,
  logo_mime, country, base_currency, locale, profile_version
FROM company;
DROP TABLE company;
ALTER TABLE company_new RENAME TO company;

-- 2. accounts --------------------------------------------------------------

ALTER TABLE accounts ADD COLUMN taxonomy TEXT;
UPDATE accounts SET taxonomy = 'rgs' WHERE taxonomy IS NULL;

-- 3. vat_returns (widen type; keep status CHECK + UNIQUE(type, period)) -----

DROP TABLE IF EXISTS vat_returns_new;
CREATE TABLE vat_returns_new (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  type        TEXT NOT NULL,
  period      TEXT NOT NULL,               -- '2026-Q2' or '2026-07'
  status      TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','filed')),
  fields_json TEXT,
  filed_at    TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(type, period)
);
INSERT INTO vat_returns_new (id, type, period, status, fields_json, filed_at, created_at)
  SELECT id, type, period, status, fields_json, filed_at, created_at FROM vat_returns;
DROP TABLE vat_returns;
ALTER TABLE vat_returns_new RENAME TO vat_returns;

-- 4. filings (widen type; keep UNIQUE(type, period)) ------------------------

DROP TABLE IF EXISTS filings_new;
CREATE TABLE filings_new (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  type       TEXT NOT NULL,
  period     TEXT NOT NULL,          -- '2026-Q3' or '2026'
  filed_at   TEXT NOT NULL,          -- date of filing
  created_by TEXT NOT NULL DEFAULT 'human',
  created_ts TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (type, period)
);
INSERT INTO filings_new (id, type, period, filed_at, created_by, created_ts)
  SELECT id, type, period, filed_at, created_by, created_ts FROM filings;
DROP TABLE filings;
ALTER TABLE filings_new RENAME TO filings;
