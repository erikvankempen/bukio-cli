-- 026_cost_centers.sql — analytical cost-center dimension on postings.
-- A cost center is an optional tagging axis for management reporting (not a
-- statutory/legal concept). It attaches to a posting exactly like vat_code_id
-- / fx fields already do: nullable, inert when the company never uses it.
CREATE TABLE cost_centers (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  code         TEXT NOT NULL,
  name         TEXT NOT NULL,
  active       INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (code),
  CHECK (active IN (0,1))
);

CREATE INDEX idx_cost_centers_active ON cost_centers(active);

-- Nullable dimension on the atomic money row. No NOT NULL, no backstop
-- trigger: cost centers are purely analytical and must never block a posting
-- (the DB already refuses unbalanced entries — adding a "must have a CC"
-- constraint would be a different product).
ALTER TABLE postings ADD COLUMN cost_center_id INTEGER REFERENCES cost_centers(id);
CREATE INDEX idx_postings_cost_center ON postings(cost_center_id);
