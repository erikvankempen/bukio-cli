/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Audit log — append-only record of every mutation (human or agent).

export function record(db, { actor, action, command = null, args = null, outcome = 'ok', entryIds = [] }) {
  db.prepare(
    'INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids) VALUES (?, ?, ?, ?, ?, ?)',
  ).run(
    String(actor),
    String(action),
    command,
    args == null ? null : JSON.stringify(args),
    String(outcome),
    entryIds.length ? JSON.stringify(entryIds) : null,
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
