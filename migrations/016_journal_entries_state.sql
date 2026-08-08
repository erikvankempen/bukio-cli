-- 016_journal_entries_state.sql — drop the dead 'reversed' state + reversed_at.
-- Reversal works via a linked contra-entry (reversed_from_id); the engine has
-- never written state='reversed' or reversed_at. The schema lied — entries
-- are either 'draft' or 'posted', nothing else.
-- SQLite cannot ALTER a CHECK constraint, so the table is rebuilt (the
-- standard procedure). The migration runner detects `PRAGMA foreign_keys`
-- and applies this file OUTSIDE its usual transaction wrapper (the pragma is
-- a no-op inside a transaction).
-- All six triggers that reference journal_entries (3 on postings + 3 on the
-- table itself) must be dropped before the DROP TABLE — SQLite refuses to
-- drop a table still referenced by a trigger — and recreated afterwards.

PRAGMA foreign_keys = OFF;

DROP TRIGGER IF EXISTS trg_postings_entry_draft_insert;
DROP TRIGGER IF EXISTS trg_postings_entry_draft_update;
DROP TRIGGER IF EXISTS trg_postings_entry_draft_delete;
DROP TRIGGER IF EXISTS trg_entries_post_requires_balance;
DROP TRIGGER IF EXISTS trg_entries_post_not_reversed;
DROP TRIGGER IF EXISTS trg_entries_posted_immutable;

-- stale `journal_entries_new` from a previously failed attempt (the rebuild
-- runs OUTSIDE a transaction — PRAGMA foreign_keys is a no-op inside one — so
-- a crash mid-migration could leave the table behind; drop it so a retry
-- starts clean instead of failing on CREATE TABLE)
DROP TABLE IF EXISTS journal_entries_new;

CREATE TABLE journal_entries_new (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  date            TEXT NOT NULL,  -- ISO yyyy-mm-dd
  description     TEXT NOT NULL,
  source          TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','bank','invoice','agent','reversal','recurring','closing','import','xaf','assets')),
  source_ref      TEXT,
  state           TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft','posted')),
  reversed_from_id INTEGER REFERENCES journal_entries(id),
  created_by      TEXT NOT NULL,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  posted_at       TEXT
);

INSERT INTO journal_entries_new (id, date, description, source, source_ref, state, reversed_from_id, created_by, created_at, posted_at)
  SELECT id, date, description, source, source_ref, state, reversed_from_id, created_by, created_at, posted_at
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

CREATE TRIGGER trg_entries_posted_immutable
BEFORE UPDATE ON journal_entries
WHEN OLD.state = 'posted'
BEGIN
  SELECT RAISE(ABORT, 'cannot modify a posted entry');
END;
