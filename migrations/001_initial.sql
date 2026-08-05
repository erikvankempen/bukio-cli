-- 001_initial.sql — bukio-cli core schema (Phase 0, VAT-agnostic)
-- Balance enforcement strategy:
--   * engine validates inside a transaction on entry create/reverse,
--   * DB trigger validates count + balance when an entry transitions to 'posted'
--     (the moment balance actually matters), so raw-SQL or agent mistakes can't
--     produce an unbalanced posted entry.

CREATE TABLE company (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  name           TEXT NOT NULL,
  kvk            TEXT,
  legal_form     TEXT CHECK (legal_form IN ('eenmanszaak','vof','bv','nv','stichting','vereniging')),
  btw_id         TEXT,
  iban           TEXT,
  vat_module     INTEGER NOT NULL DEFAULT 0,
  kor_flag       INTEGER NOT NULL DEFAULT 0,
  fiscal_year_end TEXT NOT NULL DEFAULT '12-31',
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE accounts (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  code           TEXT NOT NULL UNIQUE,
  name           TEXT NOT NULL,
  type           TEXT NOT NULL CHECK (type IN ('asset','liability','equity','income','expense')),
  rgs_code       TEXT,
  normal_balance TEXT NOT NULL CHECK (normal_balance IN ('debit','credit')),
  active         INTEGER NOT NULL DEFAULT 1,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  CHECK (
    (type IN ('asset','expense') AND normal_balance = 'debit') OR
    (type IN ('liability','equity','income') AND normal_balance = 'credit')
  )
);

CREATE TABLE journal_entries (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  date            TEXT NOT NULL,  -- ISO yyyy-mm-dd
  description     TEXT NOT NULL,
  source          TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','bank','invoice','agent','reversal','recurring','closing')),
  source_ref      TEXT,
  state           TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft','posted','reversed')),
  reversed_from_id INTEGER REFERENCES journal_entries(id),
  created_by      TEXT NOT NULL,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  posted_at       TEXT,
  reversed_at     TEXT
);
CREATE INDEX idx_entries_date  ON journal_entries(date);
CREATE INDEX idx_entries_state ON journal_entries(state);

CREATE TABLE postings (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  entry_id     INTEGER NOT NULL REFERENCES journal_entries(id),
  account_id   INTEGER NOT NULL REFERENCES accounts(id),
  amount_cents INTEGER NOT NULL CHECK (amount_cents != 0),
  document_id  INTEGER
);
CREATE INDEX idx_postings_entry   ON postings(entry_id);
CREATE INDEX idx_postings_account ON postings(account_id);

CREATE TABLE audit_log (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  actor     TEXT NOT NULL,
  action    TEXT NOT NULL,
  command   TEXT,
  args_json TEXT,
  outcome   TEXT,
  entry_ids TEXT
);
CREATE INDEX idx_audit_ts ON audit_log(ts);

-- --- Posting integrity triggers -------------------------------------------

-- Postings of a non-draft entry are immutable.
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

-- Backstop: posting an entry requires >= 2 postings summing to zero.
CREATE TRIGGER trg_entries_post_requires_balance
BEFORE UPDATE OF state ON journal_entries
WHEN NEW.state = 'posted'
BEGIN
  SELECT CASE WHEN (SELECT COUNT(*) FROM postings WHERE entry_id = NEW.id) < 2
    THEN RAISE(ABORT, 'an entry needs at least 2 postings to be posted') END;
  SELECT CASE WHEN (SELECT COALESCE(SUM(amount_cents),0) FROM postings WHERE entry_id = NEW.id) != 0
    THEN RAISE(ABORT, 'cannot post an unbalanced entry') END;
END;

-- A reversed entry cannot be posted again.
-- (Reversal semantics: the ORIGINAL entry stays 'posted'; reversing posts a
--  linked contra-entry (reversed_from_id) whose negated postings cancel the
--  original. State 'reversed' is therefore never set by the engine.)
CREATE TRIGGER trg_entries_post_not_reversed
BEFORE UPDATE OF state ON journal_entries
WHEN NEW.state = 'posted'
BEGIN
  SELECT CASE WHEN OLD.state = 'reversed'
    THEN RAISE(ABORT, 'cannot post a reversed entry') END;
END;

-- Posted entries are immutable, including their metadata: the engine reverses
-- by posting a linked contra-entry, never by mutating the original.
CREATE TRIGGER trg_entries_posted_immutable
BEFORE UPDATE ON journal_entries
WHEN OLD.state = 'posted'
BEGIN
  SELECT RAISE(ABORT, 'cannot modify a posted entry');
END;

-- --- Audit log is append-only ----------------------------------------------

CREATE TRIGGER trg_audit_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;

CREATE TRIGGER trg_audit_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
