-- v0.14.0: in-database document attachments (source documents travel with
-- backups). mode 'db' = BLOB in the DB (default); mode 'file' = content-addressed
-- copy in <db>-attachments/ with the path stored (for very large archives).
CREATE TABLE attachments (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  kind       TEXT NOT NULL CHECK (kind IN ('invoice','entry')),
  ref_id     INTEGER NOT NULL,
  file_name  TEXT NOT NULL,
  mime       TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size > 0),
  sha256     TEXT NOT NULL,
  mode       TEXT NOT NULL DEFAULT 'db' CHECK (mode IN ('db','file')),
  data       BLOB,
  path       TEXT,
  note       TEXT,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  CHECK ((mode = 'db' AND data IS NOT NULL AND path IS NULL)
      OR (mode = 'file' AND path IS NOT NULL AND data IS NULL))
);

CREATE INDEX idx_attachments_ref ON attachments(kind, ref_id);
CREATE INDEX idx_attachments_sha ON attachments(kind, ref_id, sha256);
