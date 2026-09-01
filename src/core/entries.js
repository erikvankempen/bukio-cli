/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Posting engine — the heart of bukio-cli.
// Lifecycle: draft -> posted -> reversed. Posted entries are never deleted.
import { parseAmount } from './money.js';
import { getAccountByCode, getAccount } from './accounts.js';
import { resolveCostCenterIds } from './cost-centers.js';
import { record } from '../audit/index.js';

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

export function entryError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function validateDate(date) {
  if (!DATE_RE.test(date)) throw entryError('INVALID_DATE', `date '${date}' must be yyyy-mm-dd`);
  const d = new Date(`${date}T00:00:00Z`);
  // ISO round-trip: JS rolls day-overflow (2026-02-30 -> Mar 2) with a valid
  // getTime() — only exact calendar dates may reach the ledger
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
    throw entryError('INVALID_DATE', `date '${date}' is not a valid calendar date`);
  }
}

export function nowIso() {
  return new Date().toISOString();
}

/**
 * Parse a posting spec "CODE:AMOUNT" (repeatable, comma-splittable) into
 * [{ code, amountCents }]. Pure — no DB access.
 */
export function parsePostingSpecs(raw) {
  const specs = [];
  for (const item of Array.isArray(raw) ? raw : [raw]) {
    for (const token of String(item).split(',')) {
      const t = token.trim();
      if (!t) continue;
      const m = t.match(/^(\d{1,6}):(.+)$/);
      if (!m) throw entryError('INVALID_POSTING', `posting '${t}' must be CODE:AMOUNT (e.g. 1100:1234.56)`);
      specs.push({ code: m[1], amountCents: parseAmount(m[2]) });
    }
  }
  return specs;
}

/**
 * Resolve posting specs against the DB: verifies accounts exist & are active,
 * amounts are non-zero integers, and returns
 * [{ accountId, code, amountCents, vatCodeId, vatAmountCents }].
 * Optional VAT fields are persisted as-is (nullable, inert when the VAT
 * module is off); if a vatCode is given it must exist in vat_codes.
 * Throws on any violation. Pure validation — no writes.
 */
export function resolvePostings(db, postings) {
  return postings.map((p) => {
    if (!Number.isInteger(p.amountCents) || p.amountCents === 0) {
      throw entryError('INVALID_AMOUNT_CENTS', 'posting amounts must be non-zero integers (cents)');
    }
    const account = getAccountByCode(db, p.code);
    if (!account) throw entryError('ACCOUNT_NOT_FOUND', `account ${p.code} does not exist`);
    if (!account.active) throw entryError('ACCOUNT_INACTIVE', `account ${p.code} is inactive`);

    let vatCodeId = null;
    let vatAmountCents = null;
    if (p.vatCode != null) {
      const vatRow = db.prepare('SELECT id FROM vat_codes WHERE code = ?').get(p.vatCode);
      if (!vatRow) throw entryError('VAT_CODE_NOT_FOUND', `vat code '${p.vatCode}' does not exist`);
      vatCodeId = vatRow.id;
    }
    if (p.vatAmountCents != null) {
      if (!Number.isInteger(p.vatAmountCents)) {
        throw entryError('INVALID_VAT_AMOUNT', 'vat amounts must be integers (cents)');
      }
      vatAmountCents = p.vatAmountCents;
    }

    // FX passthrough (Phase 5, FR5.X): the ledger amount is EUR; the original
    // foreign amount + currency are kept for the audit trail.
    let fxCurrency = null;
    let fxAmountCents = null;
    if (p.fxCurrency != null || p.fxAmountCents != null) {
      if (!/^[A-Z]{3}$/.test(p.fxCurrency)) throw entryError('INVALID_FX_CURRENCY', `fx currency '${p.fxCurrency}' must be ISO 4217`);
      if (!Number.isInteger(p.fxAmountCents)) throw entryError('INVALID_FX_AMOUNT', 'fx amounts must be integers (cents)');
      fxCurrency = p.fxCurrency;
      fxAmountCents = p.fxAmountCents;
    }

    // Cost center: optional analytical dimension. Null code -> null id (no
    // constraint, never blocks a posting). Named code must exist & be active.
    const { costCenterId } = p.costCenterCode != null
      ? resolveCostCenterIds(db, p)
      : { costCenterId: null };

    return {
      accountId: account.id,
      code: account.code,
      amountCents: p.amountCents,
      vatCodeId,
      vatAmountCents,
      fxCurrency,
      fxAmountCents,
      costCenterId,
    };
  });
}

/**
 * Create a journal entry (state: draft) with its postings.
 * Validates: date, description, >= 2 postings, non-zero amounts, existing &
 * active accounts, sum == 0. All inside one transaction.
 */
export function createEntry(db, {
  date, description, postings, source = 'manual', sourceRef = null, actor = 'human',
}) {
  validateDate(date);
  if (!description || !String(description).trim()) {
    throw entryError('INVALID_DESCRIPTION', 'description is required');
  }
  if (!Array.isArray(postings) || postings.length < 2) {
    throw entryError('TOO_FEW_POSTINGS', 'an entry needs at least 2 postings');
  }
  if (!['manual', 'bank', 'invoice', 'agent', 'reversal', 'recurring', 'closing', 'import', 'xaf', 'assets'].includes(source)) {
    throw entryError('INVALID_SOURCE', `source '${source}' is not allowed`);
  }
  if (typeof actor !== 'string' || !actor.trim()) {
    throw entryError('INVALID_ACTOR', 'actor is required (human or agent:<name>)');
  }

  const resolved = resolvePostings(db, postings);

  const sum = resolved.reduce((acc, p) => acc + p.amountCents, 0);
  if (sum !== 0) throw entryError('UNBALANCED', `postings do not sum to zero (sum = ${sum} cents)`);

  const insertEntry = db.prepare(
    'INSERT INTO journal_entries (date, description, source, source_ref, state, created_by) VALUES (?, ?, ?, ?, ?, ?)',
  );
  const insertPosting = db.prepare(
    'INSERT INTO postings (entry_id, account_id, amount_cents, vat_code_id, vat_amount_cents, fx_currency, fx_amount_cents, cost_center_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
  );

  const tx = db.transaction(() => {
    const info = insertEntry.run(date, String(description).trim(), source, sourceRef, 'draft', actor);
    const entryId = info.lastInsertRowid;
    for (const p of resolved) {
      insertPosting.run(entryId, p.accountId, p.amountCents, p.vatCodeId, p.vatAmountCents, p.fxCurrency, p.fxAmountCents, p.costCenterId);
    }
    record(db, {
      actor, action: 'entry.create', command: 'entry add',
      args: { date, description, postings: resolved.map((p) => `${p.code}:${p.amountCents}`), source, sourceRef },
      outcome: 'ok', entryIds: [entryId],
    });
    return entryId;
  });

  return getEntry(db, tx());
}

/** Transition draft -> posted. DB trigger backstops >= 2 postings and balance. */
export function postEntry(db, { id, actor = 'human' }) {
  const entry = getEntry(db, id);
  if (!entry) throw entryError('NOT_FOUND', `entry ${id} does not exist`);
  if (entry.state === 'posted') throw entryError('ALREADY_POSTED', `entry ${id} is already posted`);
  if (entry.state === 'reversed') throw entryError('ALREADY_REVERSED', `entry ${id} is reversed`);

  const tx = db.transaction(() => {
    // an account deactivated AFTER the draft was created must not receive
    // money — resolvePostings checks activity only at create time, so a
    // draft made while active could be posted after deactivation
    const inactive = db.prepare(`
      SELECT a.code FROM postings p
      JOIN accounts a ON a.id = p.account_id
      WHERE p.entry_id = ? AND a.active = 0
    `).all(id);
    if (inactive.length > 0) {
      throw entryError('ACCOUNT_INACTIVE', `entry ${id} postings reference deactivated account(s): ${inactive.map((r) => r.code).join(', ')}`);
    }
    db.prepare("UPDATE journal_entries SET state = 'posted', posted_at = ? WHERE id = ?")
      .run(nowIso(), id);
    record(db, { actor, action: 'entry.post', command: 'entry post', args: { id }, outcome: 'ok', entryIds: [id] });
  });
  tx();
  return getEntry(db, id);
}

/**
 * Reverse a posted entry: posts a linked contra-entry (negated postings).
 * The original entry REMAINS posted — the contra-entry cancels it, so the
 * net effect on the books is zero. Linkage: the contra-entry's
 * reversed_from_id points at the original; the audit log records the action.
 * Posted entries are never deleted — they are reversed.
 */
export function reverseEntry(db, { id, actor = 'human', reason = null, dryRun = false }) {
  const entry = getEntry(db, id);
  if (!entry) throw entryError('NOT_FOUND', `entry ${id} does not exist`);
  if (entry.state !== 'posted') {
    throw entryError(entry.state === 'draft' ? 'NOT_POSTED' : 'ALREADY_REVERSED',
      entry.state === 'draft' ? `entry ${id} must be posted before it can be reversed` : `entry ${id} is already reversed`);
  }
  const existingReversal = db.prepare(
    "SELECT COUNT(*) AS c FROM journal_entries WHERE reversed_from_id = ? AND state = 'posted'",
  ).get(id);
  if (existingReversal.c > 0) {
    throw entryError('ALREADY_REVERSED', `entry ${id} already has a posted reversal`);
  }

  if (dryRun) {
    // same checks as the real path (all above), nothing written
    return { action: 'entry.reverse', id, reason, dryRun: true };
  }

  const description = `Reversal of entry ${id}${reason ? ` — ${reason}` : ''}`;

  const insertEntry = db.prepare(
    'INSERT INTO journal_entries (date, description, source, source_ref, state, reversed_from_id, created_by) VALUES (?, ?, ?, ?, ?, ?, ?)',
  );
  const insertPosting = db.prepare(
    'INSERT INTO postings (entry_id, account_id, amount_cents, vat_code_id, vat_amount_cents, fx_currency, fx_amount_cents, cost_center_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
  );

  const tx = db.transaction(() => {
    // Re-check the reversal count INSIDE the transaction: two concurrent
    // reverses (two processes, WAL) could both pass the pre-check above and
    // post two contra-entries, moving the books twice. No DB constraint
    // backstops reversed_from_id uniqueness, so the count must be re-read
    // atomically with the insert.
    const inside = db.prepare(
      "SELECT COUNT(*) AS c FROM journal_entries WHERE reversed_from_id = ? AND state = 'posted'",
    ).get(id);
    if (inside.c > 0) {
      throw entryError('ALREADY_REVERSED', `entry ${id} already has a posted reversal`);
    }
    // Create the contra-entry as draft, add its (negated) postings, then post
    // it through the normal validated transition (count >= 2, sum == 0).
    // VAT fields are carried over (negated) so the OB readout cancels the
    // original entry — a reversal without vat_code_id would leave the
    // readout counting VAT that is no longer due. Cost center is carried over
    // (NOT negated) so the reversal still rolls up under the same cost center.
    const info = insertEntry.run(entry.date, description, 'reversal', null, 'draft', id, actor);
    const reversalId = info.lastInsertRowid;
    for (const p of entry.postings) {
      insertPosting.run(reversalId, p.account_id, -p.amount_cents,
        p.vat_code_id, p.vat_amount_cents == null ? null : -p.vat_amount_cents,
        p.fx_currency, p.fx_amount_cents == null ? null : -p.fx_amount_cents,
        p.cost_center_id == null ? null : p.cost_center_id);
    }
    db.prepare("UPDATE journal_entries SET state = 'posted', posted_at = ? WHERE id = ?")
      .run(nowIso(), reversalId);
    record(db, {
      actor, action: 'entry.reverse', command: 'entry reverse',
      args: { id, reason }, outcome: 'ok', entryIds: [reversalId, id],
    });
    return reversalId;
  });

  return getEntry(db, tx());
}

export function listEntries(db, { state = null, dateFrom = null, dateTo = null, limit = 100 } = {}) {
  if (!Number.isInteger(limit) || limit < 0) {
    throw entryError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
  }
  // garbage bounds would silently match everything/ nothing ('2026-01-15' <=
  // 'garbage' is TRUE in string comparison) — same class as the balans as-of
  if (dateFrom != null) validateDate(dateFrom);
  if (dateTo != null) validateDate(dateTo);
  const clauses = [];
  const params = [];
  if (state) {
    clauses.push('state = ?');
    params.push(state);
  }
  if (dateFrom) {
    clauses.push('date >= ?');
    params.push(dateFrom);
  }
  if (dateTo) {
    clauses.push('date <= ?');
    params.push(dateTo);
  }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  const rows = db.prepare(
    `SELECT * FROM journal_entries ${where} ORDER BY date DESC, id DESC LIMIT ?`,
  ).all(...params, limit);
  return rows;
}

export function getEntry(db, id) {
  const entry = db.prepare('SELECT * FROM journal_entries WHERE id = ?').get(id);
  if (!entry) return null;
  entry.postings = db.prepare(
    `SELECT p.id, p.account_id, p.amount_cents, p.document_id, p.vat_code_id, p.vat_amount_cents,
            p.fx_currency, p.fx_amount_cents, p.cost_center_id,
            a.code AS account_code, a.name AS account_name, a.type AS account_type,
            vc.code AS vat_code, cc.code AS cost_center_code, cc.name AS cost_center_name
     FROM postings p JOIN accounts a ON a.id = p.account_id
     LEFT JOIN vat_codes vc ON vc.id = p.vat_code_id
     LEFT JOIN cost_centers cc ON cc.id = p.cost_center_id
     WHERE p.entry_id = ? ORDER BY p.id`,
  ).all(id);
  // surface EUR amount for FX postings
  for (const p of entry.postings) {
    p.fx_amount = p.fx_amount_cents == null ? null : (p.fx_amount_cents / 100).toFixed(2);
  }
  return entry;
}

export function getAccountById(db, id) {
  return getAccount(db, id);
}
