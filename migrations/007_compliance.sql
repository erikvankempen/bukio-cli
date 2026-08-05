-- 007_compliance.sql — filings registry (Phase 5: compliance calendar)
CREATE TABLE IF NOT EXISTS filings (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  type       TEXT NOT NULL CHECK (type IN ('OB', 'ICP', 'JAARREKENING')),
  period     TEXT NOT NULL,          -- '2026-Q3' or '2026'
  filed_at   TEXT NOT NULL,          -- date of filing
  created_by TEXT NOT NULL DEFAULT 'human',
  created_ts TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (type, period)
);
