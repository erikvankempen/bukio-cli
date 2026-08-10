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
  db.prepare(
    `INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids,
                            digest_hash, sig_keyid, sig_nonce, sig_ts, sig, sig_status)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  ).run(
    String(actor),
    String(action),
    command,
    args == null ? null : JSON.stringify(args),
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
