/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Cost centers — an analytical (management-reporting) dimension on postings.
// Optional and inert unless a company creates cost centers. Mirrors how VAT and
// FX already attach per-posting: a nullable FK on the postings row.
import { parseAmount } from './money.js';

const CODE_RE = /^[A-Z0-9][A-Z0-9 ._-]{0,31}$/;

export function costCenterError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function createCostCenter(db, { code, name }) {
  if (!CODE_RE.test(code)) {
    throw costCenterError('INVALID_CODE', `cost center code '${code}' must be 1-32 chars: letters, digits, space, . _ - (no leading punctuation)`);
  }
  if (!name || !String(name).trim()) {
    throw costCenterError('INVALID_NAME', 'cost center name is required');
  }
  try {
    const info = db.prepare(
      'INSERT INTO cost_centers (code, name, active) VALUES (?, ?, 1)',
    ).run(code, String(name).trim());
    return getCostCenter(db, info.lastInsertRowid);
  } catch (err) {
    if (String(err.message).includes('UNIQUE constraint failed')) {
      throw costCenterError('COST_CENTER_EXISTS', `cost center '${code}' already exists`);
    }
    throw err;
  }
}

export function getCostCenter(db, id) {
  return db.prepare('SELECT * FROM cost_centers WHERE id = ?').get(id) ?? null;
}

export function getCostCenterByCode(db, code) {
  return db.prepare('SELECT * FROM cost_centers WHERE code = ?').get(code) ?? null;
}

export function listCostCenters(db, { includeInactive = false } = {}) {
  const where = includeInactive ? '' : 'WHERE active = 1';
  return db.prepare(`SELECT * FROM cost_centers ${where} ORDER BY code`).all();
}

export function deactivateCostCenter(db, code) {
  const cc = getCostCenterByCode(db, code);
  if (!cc) throw costCenterError('COST_CENTER_NOT_FOUND', `cost center '${code}' does not exist`);
  if (!cc.active) throw costCenterError('ALREADY_INACTIVE', `cost center '${code}' is already inactive`);
  db.prepare('UPDATE cost_centers SET active = 0 WHERE id = ?').run(cc.id);
  return getCostCenter(db, cc.id);
}

export function reactivateCostCenter(db, code) {
  const cc = getCostCenterByCode(db, code);
  if (!cc) throw costCenterError('COST_CENTER_NOT_FOUND', `cost center '${code}' does not exist`);
  if (cc.active) throw costCenterError('ALREADY_ACTIVE', `cost center '${code}' is already active`);
  db.prepare('UPDATE cost_centers SET active = 1 WHERE id = ?').run(cc.id);
  return getCostCenter(db, cc.id);
}

/**
 * Parse entry posting specs with an optional cost-center suffix:
 *   "CODE:AMOUNT@CC"  or  "CODE:AMOUNT"  (no CC)
 * Returns [{ code, amountCents, costCenterCode|null }]. Pure — no DB access.
 * The '@' suffix is safe to reuse here: VAT's '@VATCODE' lives only in the
 * `vat book` command; plain `entry add` has never parsed a suffix, so there is
 * no conflict.
 */
export function parsePostingSpecsWithCostCenter(raw) {
  const out = [];
  for (const item of Array.isArray(raw) ? raw : [raw]) {
    for (const token of String(item).split(',')) {
      const t = token.trim();
      if (!t) continue;
      const m = t.match(/^(\d{1,6}):(.+?)(?:@([A-Z0-9][A-Z0-9 ._-]{0,31}))?$/);
      if (!m) throw costCenterError('INVALID_POSTING', `posting '${t}' must be CODE:AMOUNT[@COSTCENTER] (e.g. 8000:-100.00@HQ)`);
      const cents = parseAmount(m[2]);
      out.push({ code: m[1], amountCents: cents, costCenterCode: m[3] ?? null });
    }
  }
  return out;
}

/** Resolve cost-center codes to ids; null code stays null. Throws if a named code doesn't exist. */
export function resolveCostCenterIds(db, spec) {
  if (spec.costCenterCode == null) return { costCenterId: null };
  const cc = getCostCenterByCode(db, spec.costCenterCode);
  if (!cc) throw costCenterError('COST_CENTER_NOT_FOUND', `cost center '${spec.costCenterCode}' does not exist`);
  if (!cc.active) throw costCenterError('COST_CENTER_INACTIVE', `cost center '${spec.costCenterCode}' is inactive`);
  return { costCenterId: cc.id };
}
