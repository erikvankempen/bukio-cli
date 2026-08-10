-- 018_actor_signing.sql — Tier 0 signed actor commands.
-- Per-company actor key registry (Ed25519 public keys) plus the signing
-- enforcement flag, and the audit-log signature columns that make the trail
-- cryptographically verifiable. Additive only: existing audit rows read back
-- with sig_status = 'unsigned' (claimed, not yet provable).

CREATE TABLE IF NOT EXISTS actor_keys (
  actor          TEXT PRIMARY KEY,   -- 'agent:bartholomeus' | 'human:erik' | 'system:<name>'
  keyid          TEXT NOT NULL,      -- sha256(SPKI DER), first 16 bytes as hex
  public_key     TEXT NOT NULL,      -- SPKI PEM
  enrolled_at    TEXT NOT NULL,      -- ISO 8601 UTC
  revoked_at     TEXT,               -- set on revoke; the row is retained for history
  revoked_reason TEXT
);

CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT
);                                   -- 'signing_enforce' = 'on' | 'off'

ALTER TABLE audit_log ADD COLUMN digest_hash TEXT;
ALTER TABLE audit_log ADD COLUMN sig_keyid TEXT;
ALTER TABLE audit_log ADD COLUMN sig_nonce TEXT;
ALTER TABLE audit_log ADD COLUMN sig_ts TEXT;
ALTER TABLE audit_log ADD COLUMN sig TEXT;
ALTER TABLE audit_log ADD COLUMN sig_status TEXT NOT NULL DEFAULT 'unsigned';
