-- 003_recurring.sql — recurring entries & period automation (Phase 3, FR3A)
-- Templates are validated at creation (accounts exist, postings balanced,
-- VAT legs expanded when VAT-aware). Generation just replays them, so runs
-- are deterministic and the engine stays simple.

CREATE TABLE recurring_templates (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  name               TEXT NOT NULL,
  description        TEXT,
  frequency          TEXT NOT NULL CHECK (frequency IN ('monthly','quarterly','yearly')),
  -- day of the period the entry is booked on (1-28; 29-31 avoided to skip
  -- month-end clamping edge cases)
  day_of_period      INTEGER NOT NULL DEFAULT 1 CHECK (day_of_period BETWEEN 1 AND 28),
  start_date         TEXT NOT NULL,
  end_date           TEXT,                      -- NULL = no end date
  runs               INTEGER CHECK (runs IS NULL OR runs > 0),  -- NULL = unlimited
  -- final resolved posting set (VAT legs included where applicable)
  postings_json      TEXT NOT NULL,
  -- optional posting set for the LAST run (e.g. depreciation remainder);
  -- NULL = normal postings every run
  final_postings_json TEXT,
  -- accrual pattern: each run first reverses the previous generated entry,
  -- then books the new one (standard "nog te betalen kosten" close)
  reverse_previous   INTEGER NOT NULL DEFAULT 0,
  status             TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','paused','completed')),
  next_run_date      TEXT NOT NULL,
  last_run_date      TEXT,
  last_entry_id      INTEGER,
  runs_done          INTEGER NOT NULL DEFAULT 0,
  vat_aware          INTEGER NOT NULL DEFAULT 0,
  created_by         TEXT NOT NULL,
  created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_recurring_due ON recurring_templates(status, next_run_date);
