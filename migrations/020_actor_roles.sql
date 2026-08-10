-- 020_actor_roles.sql — Tier 0.5 per-actor authorizations (segregation of duties).
-- actor_roles: which roles each actor holds in THIS company. Plain mutable
-- state (revoke = DELETE) — the append-only audit log carries the
-- grant/revoke evidence, like every other action. The role→capability map
-- lives in CODE (src/core/authz.js), not in the DB — versioned with the
-- binary, never a schema/versioning concern.
CREATE TABLE IF NOT EXISTS actor_roles (
  actor      TEXT NOT NULL,
  role       TEXT NOT NULL CHECK (role IN ('owner','bookkeeper','payments','tax','assets','readonly')),
  granted_by TEXT NOT NULL,
  granted_at TEXT NOT NULL,
  PRIMARY KEY (actor, role)
);

-- authz_mode: 'on' | 'off' per company, like signing_enforce (default off).
INSERT INTO settings (key, value) SELECT 'authz_mode', 'off' WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'authz_mode');
