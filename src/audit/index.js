/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Audit log — append-only record of every mutation (human or agent).
//
// Signature plumbing (Tier 0): the CLI sign gate runs before every action
// and sets the signature bundle for the command; the first record() call of
// that command picks it up so the audit row carries digest/signature/
// keyid/nonce/timestamp. Commands without a signature stay 'unsigned' —
// an honest "claimed, not yet provable" label.
import { buildDigest } from '../core/canonical.js';
import { verify } from '../core/sign.js';
import { getKeyByKeyid } from '../core/actor-registry.js';

let pendingSignature = null;

/** Set the signature bundle for the command about to run (called by the
 * sign gate before dispatch). */
export function setPendingSignature(sig) {
  pendingSignature = sig ?? null;
}

export function record(db, {
  actor, action, command = null, args = null, outcome = 'ok', entryIds = [],
  digestHash = null, sigKeyid = null, sigNonce = null, sigTs = null, sig = null,
  sigStatus = null,
}) {
  const s = pendingSignature ?? {};
  const fields = {
    digestHash: digestHash ?? s.digestHash ?? null,
    sigKeyid: sigKeyid ?? s.sigKeyid ?? null,
    sigNonce: sigNonce ?? s.sigNonce ?? null,
    sigTs: sigTs ?? s.sigTs ?? null,
    sig: sig ?? s.sig ?? null,
    sigStatus: sigStatus ?? s.sigStatus ?? 'unsigned',
  };
  // signed rows store the EXACT args + command path that were signed (so
  // `audit verify` can recompute the digest); unsigned rows keep the
  // command's own labels
  const storedArgs = s.signedArgs !== undefined ? s.signedArgs : args;
  const storedCommand = s.signedCommand !== undefined ? s.signedCommand : command;
  db.prepare(
    `INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids,
                            digest_hash, sig_keyid, sig_nonce, sig_ts, sig, sig_status)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  ).run(
    String(actor),
    String(action),
    storedCommand,
    storedArgs == null ? null : JSON.stringify(storedArgs),
    String(outcome),
    entryIds.length ? JSON.stringify(entryIds) : null,
    fields.digestHash, fields.sigKeyid, fields.sigNonce, fields.sigTs, fields.sig, fields.sigStatus,
  );
}

export function list(db, { since = null, actor = null, limit = 50 } = {}) {
  if (!Number.isInteger(limit) || limit < 0) {
    const e = new Error(`limit must be a non-negative integer, got '${limit}'`);
    e.code = 'INVALID_LIMIT';
    throw e;
  }
  const clauses = [];
  const params = [];
  if (since) {
    clauses.push('ts >= ?');
    params.push(since);
  }
  if (actor) {
    clauses.push('actor = ?');
    params.push(actor);
  }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  const rows = db.prepare(
    `SELECT * FROM audit_log ${where} ORDER BY id DESC LIMIT ?`,
  ).all(...params, limit);
  return rows.map((r) => ({
    ...r,
    args: r.args_json ? JSON.parse(r.args_json) : null,
    entry_ids: r.entry_ids ? JSON.parse(r.entry_ids) : [],
  }));
}

// --- trail verification (Task 7) ------------------------------------------
//
// `bukio audit verify` re-checks every row against the company registry,
// purely from what is stored in the company DB (self-contained — a copied
// DB file verifies with no external files). Per-row status:
//   ok                 signature verifies, digest matches, key active
//   unsigned           no signature (legacy / record-mode rows) — not an error
//   revoked            signature verifies, key since revoked (valid at the
//                      time) — informational
//   tampered           args no longer produce the signed digest
//   invalid-signature  digest matches but the signature fails
//   unknown-key        sig_keyid is not in this company's registry

/** Classify one audit row. @returns {string} one of the statuses above. */
function classifyRow(db, row) {
  if (row.sig_status === 'unsigned' || (!row.digest_hash && !row.sig)) return 'unsigned';
  let recomputed = null;
  try {
    const args = row.args_json ? JSON.parse(row.args_json) : null;
    recomputed = buildDigest({
      actor: row.actor, cmd: row.command, args, ts: row.sig_ts, nonce: row.sig_nonce,
    });
  } catch {
    return 'tampered';
  }
  if (recomputed !== row.digest_hash) return 'tampered';
  const key = getKeyByKeyid(db, row.sig_keyid);
  if (!key) return 'unknown-key';
  if (!verify(recomputed, row.sig, key.public_key)) return 'invalid-signature';
  if (key.revoked_at !== null) return 'revoked';
  return 'ok';
}

/**
 * Verify the audit trail of a company DB. Reads only the DB itself.
 *
 * @param {object} db - open company database.
 * @param {object} [opts]
 * @param {string} [opts.since] - ISO timestamp; only rows at/after it.
 * @param {number} [opts.limit] - check only the newest N rows (default: all).
 * @returns {{rows: Array<object>, summary: object}} per-row status + counts.
 */
export function verifyTrail(db, { since = null, limit = null } = {}) {
  // a negative limit must not silently slice from the wrong end (slice(-(-5))
  // == slice(5) — it would DROP the oldest 5 rows and check the rest); match
  // the audit list() guard so the CLI surfaces INVALID_LIMIT consistently
  if (limit !== null && (!Number.isInteger(limit) || limit < 0)) {
    const e = new Error(`limit must be a non-negative integer, got '${limit}'`);
    e.code = 'INVALID_LIMIT';
    throw e;
  }
  let rows = since
    ? db.prepare('SELECT * FROM audit_log WHERE ts >= ? ORDER BY id').all(since)
    : db.prepare('SELECT * FROM audit_log ORDER BY id').all();
  if (limit) rows = rows.slice(-Number(limit));
  const summary = { total: rows.length, ok: 0, unsigned: 0, revoked: 0, tampered: 0, invalid_signature: 0, unknown_key: 0 };
  const checked = rows.map((row) => {
    const status = classifyRow(db, row);
    // statuses are hyphenated ('invalid-signature'); summary keys are
    // underscored ('invalid_signature')
    summary[status.replaceAll('-', '_')] += 1;
    return {
      id: row.id, ts: row.ts, actor: row.actor, action: row.action,
      command: row.command, sig_status: row.sig_status, status,
    };
  });
  return { rows: checked, summary };
}
