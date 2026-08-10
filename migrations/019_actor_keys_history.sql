-- 019_actor_keys_history.sql — multi-key actor registry.
-- One row per (actor, keyid): a revoked key REMAINS in the table so
-- historical audit rows signed with it stay verifiable ("valid at the
-- time, key since revoked"). Each actor has at most one ACTIVE (non-revoked)
-- row; the sign gate and the registry API use only the active key.
CREATE TABLE actor_keys_new (
  actor          TEXT NOT NULL,   -- 'agent:bartholomeus' | 'human:erik' | 'system:<name>'
  keyid          TEXT NOT NULL,   -- sha256(SPKI DER), first 16 bytes as hex
  public_key     TEXT NOT NULL,   -- SPKI PEM
  enrolled_at    TEXT NOT NULL,   -- ISO 8601 UTC
  revoked_at     TEXT,            -- set on revoke; the row is retained
  revoked_reason TEXT,
  PRIMARY KEY (actor, keyid)
);

INSERT INTO actor_keys_new (actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason)
  SELECT actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason FROM actor_keys;

DROP TABLE actor_keys;
ALTER TABLE actor_keys_new RENAME TO actor_keys;
