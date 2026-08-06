-- 009_assets.sql — fixed assets module (Phase 7): depreciation schemes,
-- asset register with mid-life adoption (recognition date + cumulative
-- depreciation at recognition), monthly depreciation runs.
-- journal_entries.source gains 'assets' — CHECK widening via table rebuild,
-- the standard procedure (same as 008; the runner detects `PRAGMA
-- foreign_keys` and applies this file OUTSIDE its transaction wrapper).

PRAGMA foreign_keys = OFF;

DROP TRIGGER IF EXISTS trg_postings_entry_draft_insert;
DROP TRIGGER IF EXISTS trg_postings_entry_draft_update;
DROP TRIGGER IF EXISTS trg_postings_entry_draft_delete;
DROP TRIGGER IF EXISTS trg_entries_post_requires_balance;
DROP TRIGGER IF EXISTS trg_entries_post_not_reversed;
DROP TRIGGER IF EXISTS trg_entries_posted_immutable;

CREATE TABLE journal_entries_new (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  date            TEXT NOT NULL,  -- ISO yyyy-mm-dd
  description     TEXT NOT NULL,
  source          TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','bank','invoice','agent','reversal','recurring','closing','import','xaf','assets')),
  source_ref      TEXT,
  state           TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft','posted','reversed')),
  reversed_from_id INTEGER REFERENCES journal_entries(id),
  created_by      TEXT NOT NULL,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  posted_at       TEXT,
  reversed_at     TEXT
);

INSERT INTO journal_entries_new (id, date, description, source, source_ref, state, reversed_from_id, created_by, created_at, posted_at, reversed_at)
  SELECT id, date, description, source, source_ref, state, reversed_from_id, created_by, created_at, posted_at, reversed_at
  FROM journal_entries;

DROP TABLE journal_entries;
ALTER TABLE journal_entries_new RENAME TO journal_entries;

CREATE INDEX idx_entries_date  ON journal_entries(date);
CREATE INDEX idx_entries_state ON journal_entries(state);

-- Recreate all triggers (dropped with / before the old table).
CREATE TRIGGER trg_postings_entry_draft_insert
BEFORE INSERT ON postings
BEGIN
  SELECT CASE WHEN (SELECT state FROM journal_entries WHERE id = NEW.entry_id) != 'draft'
    THEN RAISE(ABORT, 'cannot modify postings of a non-draft entry') END;
END;

CREATE TRIGGER trg_postings_entry_draft_update
BEFORE UPDATE ON postings
BEGIN
  SELECT CASE WHEN (SELECT state FROM journal_entries WHERE id = NEW.entry_id) != 'draft'
    THEN RAISE(ABORT, 'cannot modify postings of a non-draft entry') END;
END;

CREATE TRIGGER trg_postings_entry_draft_delete
BEFORE DELETE ON postings
BEGIN
  SELECT CASE WHEN (SELECT state FROM journal_entries WHERE id = OLD.entry_id) != 'draft'
    THEN RAISE(ABORT, 'cannot modify postings of a non-draft entry') END;
END;

CREATE TRIGGER trg_entries_post_requires_balance
BEFORE UPDATE OF state ON journal_entries
WHEN NEW.state = 'posted'
BEGIN
  SELECT CASE WHEN (SELECT COUNT(*) FROM postings WHERE entry_id = NEW.id) < 2
    THEN RAISE(ABORT, 'an entry needs at least 2 postings to be posted') END;
  SELECT CASE WHEN (SELECT COALESCE(SUM(amount_cents),0) FROM postings WHERE entry_id = NEW.id) != 0
    THEN RAISE(ABORT, 'cannot post an unbalanced entry') END;
END;

CREATE TRIGGER trg_entries_post_not_reversed
BEFORE UPDATE OF state ON journal_entries
WHEN NEW.state = 'posted'
BEGIN
  SELECT CASE WHEN OLD.state = 'reversed'
    THEN RAISE(ABORT, 'cannot post a reversed entry') END;
END;

CREATE TRIGGER trg_entries_posted_immutable
BEFORE UPDATE ON journal_entries
WHEN OLD.state = 'posted'
BEGIN
  SELECT RAISE(ABORT, 'cannot modify a posted entry');
END;

-- --- fixed assets -----------------------------------------------------------

-- Depreciation schemes: the booking policy for a class of assets.
-- residual_bp = residual value as basis points of the purchase price (0-10000,
-- so 2.5% = 250). The standard scheme is 5 years linear, monthly, 0% residual.
CREATE TABLE depreciation_schemes (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  name          TEXT NOT NULL UNIQUE,
  method        TEXT NOT NULL DEFAULT 'lineair' CHECK (method IN ('lineair','degressief')),
  life_months   INTEGER NOT NULL CHECK (life_months BETWEEN 1 AND 600),
  residual_bp   INTEGER NOT NULL DEFAULT 0 CHECK (residual_bp BETWEEN 0 AND 10000),
  created_by    TEXT NOT NULL,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- The asset register. Assets enter the module at `recognition_date` with
-- `cum_dep_at_recognition_cents` already booked in the ledger (the purchase
-- and past depreciation are NOT re-booked by this module — only the REMAINING
-- depreciation runs forward, monthly, on the 1st of each month).
CREATE TABLE assets (
  id                            INTEGER PRIMARY KEY AUTOINCREMENT,
  name                          TEXT NOT NULL,
  category                      TEXT,
  serial                        TEXT,
  status                        TEXT NOT NULL DEFAULT 'active'
    CHECK (status IN ('active','paused','fully_depreciated','disposed')),
  scheme_id                     INTEGER NOT NULL REFERENCES depreciation_schemes(id),
  purchase_date                 TEXT NOT NULL,
  purchase_price_cents          INTEGER NOT NULL CHECK (purchase_price_cents > 0),
  residual_cents                INTEGER NOT NULL DEFAULT 0 CHECK (residual_cents >= 0),
  depreciation_start_date       TEXT NOT NULL,
  recognition_date              TEXT NOT NULL,
  cum_dep_at_recognition_cents  INTEGER NOT NULL DEFAULT 0 CHECK (cum_dep_at_recognition_cents >= 0),
  asset_account_id              INTEGER NOT NULL REFERENCES accounts(id),
  cum_dep_account_id            INTEGER REFERENCES accounts(id),   -- NULL: book on the asset account
  expense_account_id            INTEGER NOT NULL REFERENCES accounts(id),
  entry_id                      INTEGER REFERENCES journal_entries(id),  -- purchase booking (invoice)
  note                          TEXT,
  disposed_date                 TEXT,
  disposed_proceeds_cents       INTEGER,
  disposal_entry_id             INTEGER REFERENCES journal_entries(id),
  created_by                    TEXT NOT NULL,
  created_at                    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_assets_status ON assets(status);

-- Every monthly run the module books (idempotency key: asset_id + period).
CREATE TABLE asset_depreciation_runs (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  asset_id      INTEGER NOT NULL REFERENCES assets(id),
  period        TEXT NOT NULL,               -- 'YYYY-MM' (the run books on the 1st)
  entry_id      INTEGER NOT NULL REFERENCES journal_entries(id),
  amount_cents  INTEGER NOT NULL CHECK (amount_cents > 0),
  created_by    TEXT NOT NULL,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (asset_id, period)
);
