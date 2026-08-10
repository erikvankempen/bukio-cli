/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// DB-backed actor key registry (Tier 0 of the actor authentication design):
// which public key is enrolled for which actor in THIS company database, and
// whether signing is enforced here. The registry lives inside the company DB
// (migration 018) so each company's books are self-contained for audit
// verification — the same keypair can be registered per company.
//
// Revocation keeps history: the row stays, marked with revoked_at/reason, so
// `audit verify` can still validate signatures made while the key was valid
// ("valid at the time, since revoked"). Re-enrolment after revocation is the
// rotation flow (keygen -> revoke old -> register new).
import { isValidActor } from './actor.js';

const ENFORCE_KEY = 'signing_enforce';

export function registryError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/**
 * Register an actor's public key in the current company DB.
 *
 * @param {object} db - open company database.
 * @param {object} input
 * @param {string} input.actor - '<role>:<name>', e.g. 'agent:bartholomeus'.
 * @param {string} input.keyid - 32-hex fingerprint of the public key.
 * @param {string} input.publicKey - SPKI PEM.
 * @param {string} [input.enrolledAt] - ISO timestamp (defaults to now).
 * @returns {object} the stored row.
 * @throws ALREADY_ENROLLED when the actor has an active key; re-enrol after
 *   a revocation (rotation) replaces the revoked key.
 */
export function enrolActor(db, { actor, keyid, publicKey, enrolledAt = null }) {
  if (!isValidActor(actor)) throw registryError('INVALID_ACTOR', `'${actor}' is not a valid '<role>:<name>' actor`);
  if (!keyid || !publicKey) throw registryError('INVALID_KEY', 'keyid and public key are required');
  const existing = getActorKey(db, actor);
  const at = enrolledAt ?? new Date().toISOString();
  if (existing) {
    if (existing.revoked_at === null) {
      throw registryError('ALREADY_ENROLLED', `actor ${actor} already has an active key (${existing.keyid}) — revoke it first to rotate`);
    }
    db.prepare(
      'UPDATE actor_keys SET keyid = ?, public_key = ?, enrolled_at = ?, revoked_at = NULL, revoked_reason = NULL WHERE actor = ?',
    ).run(keyid, publicKey, at, actor);
    return getActorKey(db, actor);
  }
  db.prepare(
    'INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES (?, ?, ?, ?)',
  ).run(actor, keyid, publicKey, at);
  return getActorKey(db, actor);
}

/**
 * Revoke an actor's key: the row is retained with revoked_at + reason, so
 * historical audit verification still works but the key stops authorising
 * new commands.
 *
 * @param {object} db
 * @param {object} input
 * @param {string} input.actor
 * @param {string} input.reason - required; revocation without a reason is
 *   rejected (the audit trail must explain itself).
 * @param {string} [input.revokedAt] - ISO timestamp (defaults to now).
 * @returns {object} the stored (revoked) row.
 * @throws NOT_ENROLLED / ALREADY_REVOKED / INVALID_REASON.
 */
export function revokeActor(db, { actor, reason, revokedAt = null }) {
  if (!reason || !String(reason).trim()) {
    throw registryError('INVALID_REASON', 'a revocation reason is required');
  }
  const existing = getActorKey(db, actor);
  if (!existing) throw registryError('NOT_ENROLLED', `actor ${actor} has no enrolled key in this company DB`);
  if (existing.revoked_at !== null) {
    throw registryError('ALREADY_REVOKED', `actor ${actor} is already revoked`);
  }
  db.prepare('UPDATE actor_keys SET revoked_at = ?, revoked_reason = ? WHERE actor = ?')
    .run(revokedAt ?? new Date().toISOString(), String(reason).trim(), actor);
  return getActorKey(db, actor);
}

/**
 * @param {object} db
 * @param {string} actor
 * @returns {object|null} the registry row (active or revoked), or null.
 */
export function getActorKey(db, actor) {
  return db.prepare('SELECT * FROM actor_keys WHERE actor = ?').get(actor) ?? null;
}

/**
 * "May this actor act?" — true only when the actor has an enrolled key that
 * has not been revoked. The enforcement gate (CLI/MCP) builds on this.
 *
 * @param {object} db
 * @param {string} actor
 * @returns {boolean}
 */
export function canAct(db, actor) {
  const row = getActorKey(db, actor);
  return Boolean(row) && row.revoked_at === null;
}

/**
 * @param {object} db
 * @param {boolean} on - true enforces signed commands for this company.
 */
export function setEnforce(db, on) {
  db.prepare(
    `INSERT INTO settings (key, value) VALUES (?, ?)
     ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
  ).run(ENFORCE_KEY, on ? 'on' : 'off');
}

/**
 * @param {object} db
 * @returns {'on'|'off'} the per-company enforcement flag (default 'off').
 */
export function getEnforce(db) {
  const row = db.prepare('SELECT value FROM settings WHERE key = ?').get(ENFORCE_KEY);
  return row && row.value === 'on' ? 'on' : 'off';
}
