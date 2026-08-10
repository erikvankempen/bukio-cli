/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// DB-backed actor key registry (Tier 0 of the actor authentication design):
// which public keys are enrolled for which actor in THIS company database,
// and whether signing is enforced here. The registry lives inside the company
// DB (migrations 018 + 019) so each company's books are self-contained for
// audit verification — the same keypair can be registered per company.
//
// One row per (actor, keyid), migration 019: a revoked key REMAINS in the
// table so `audit verify` can still validate signatures made while the key
// was valid ("valid at the time, since revoked"). Each actor has at most one
// ACTIVE (non-revoked) row; the sign gate and the registry API use only the
// active key. Rotation = keygen (new key) -> revoke (old key, retained) ->
// register (new key row).
import { isValidActor } from './actor.js';

const ENFORCE_KEY = 'signing_enforce';

export function registryError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/**
 * Register an actor's public key in the current company DB. A new key can
 * only be enrolled while no active key exists (revoke first — rotation);
 * revoked key rows are retained as history.
 *
 * @param {object} db - open company database.
 * @param {object} input
 * @param {string} input.actor - '<role>:<name>', e.g. 'agent:bartholomeus'.
 * @param {string} input.keyid - 32-hex fingerprint of the public key.
 * @param {string} input.publicKey - SPKI PEM.
 * @param {string} [input.enrolledAt] - ISO timestamp (defaults to now).
 * @returns {object} the stored (active) row.
 * @throws ALREADY_ENROLLED when the actor has an active key.
 */
export function enrolActor(db, { actor, keyid, publicKey, enrolledAt = null }) {
  if (!isValidActor(actor)) throw registryError('INVALID_ACTOR', `'${actor}' is not a valid '<role>:<name>' actor`);
  if (!keyid || !publicKey) throw registryError('INVALID_KEY', 'keyid and public key are required');
  const active = getActorKey(db, actor);
  if (active) {
    throw registryError('ALREADY_ENROLLED', `actor ${actor} already has an active key (${active.keyid}) — revoke it first to rotate`);
  }
  const at = enrolledAt ?? new Date().toISOString();
  db.prepare(
    'INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES (?, ?, ?, ?)',
  ).run(actor, keyid, publicKey, at);
  return getActorKey(db, actor);
}

/**
 * Revoke the actor's ACTIVE key: the row is retained with revoked_at +
 * reason, so historical audit verification still works but the key stops
 * authorising new commands.
 *
 * @param {object} db
 * @param {object} input
 * @param {string} input.actor
 * @param {string} input.reason - required; revocation without a reason is
 *   rejected (the audit trail must explain itself).
 * @param {string} [input.revokedAt] - ISO timestamp (defaults to now).
 * @returns {object} the stored (revoked) row.
 * @throws NOT_ENROLLED / INVALID_REASON.
 */
export function revokeActor(db, { actor, reason, revokedAt = null }) {
  if (!reason || !String(reason).trim()) {
    throw registryError('INVALID_REASON', 'a revocation reason is required');
  }
  const active = getActorKey(db, actor);
  if (!active) throw registryError('NOT_ENROLLED', `actor ${actor} has no active key in this company DB`);
  db.prepare('UPDATE actor_keys SET revoked_at = ?, revoked_reason = ? WHERE actor = ? AND keyid = ?')
    .run(revokedAt ?? new Date().toISOString(), String(reason).trim(), actor, active.keyid);
  return db.prepare('SELECT * FROM actor_keys WHERE actor = ? AND keyid = ?').get(actor, active.keyid);
}

/**
 * @param {object} db
 * @param {string} actor
 * @returns {object|null} the actor's ACTIVE registry row, or null.
 */
export function getActorKey(db, actor) {
  return db.prepare('SELECT * FROM actor_keys WHERE actor = ? AND revoked_at IS NULL ORDER BY enrolled_at DESC LIMIT 1').get(actor) ?? null;
}

/**
 * Look up a registry row by keyid, active or revoked — used by `audit
 * verify` to re-check historical signatures (a rotated key is still
 * verifiable via its retained row).
 *
 * @param {object} db
 * @param {string} keyid
 * @returns {object|null} the most recent row for that keyid, or null.
 */
export function getKeyByKeyid(db, keyid) {
  return db.prepare('SELECT * FROM actor_keys WHERE keyid = ? ORDER BY enrolled_at DESC LIMIT 1').get(keyid) ?? null;
}

/**
 * Look up the actor's most recent registry row, active or revoked — used to
 * distinguish 'never enrolled' from 'enrolled but since revoked'.
 *
 * @param {object} db
 * @param {string} actor
 * @returns {object|null}
 */
export function getAnyActorKey(db, actor) {
  return db.prepare('SELECT * FROM actor_keys WHERE actor = ? ORDER BY enrolled_at DESC LIMIT 1').get(actor) ?? null;
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
  return getActorKey(db, actor) !== null;
}

/**
 * @param {object} db
 * @returns {Array<object>} all registry rows (active + revoked history),
 *   oldest first.
 */
export function listActors(db) {
  return db.prepare('SELECT actor, keyid, enrolled_at, revoked_at, revoked_reason FROM actor_keys ORDER BY enrolled_at, actor').all();
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
