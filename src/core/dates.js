/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Shared date helpers. One canonical implementation of the patterns that were
// previously re-implemented per module (entries, assets, import, ubl-invoice,
// reports, compliance, recurring): shape check + ISO round-trip calendar
// validation, today in UTC, month arithmetic with day clamping.

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

/** True when s is a real calendar date in yyyy-mm-dd form (no overflow). */
export function isValidDate(s) {
  if (typeof s !== 'string' || !DATE_RE.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

/** Throw INVALID_DATE unless date is a valid yyyy-mm-dd calendar date. */
export function validateDate(date, { code = 'INVALID_DATE' } = {}) {
  if (!DATE_RE.test(date)) {
    const e = new Error(`date '${date}' must be yyyy-mm-dd`);
    e.code = code;
    throw e;
  }
  const d = new Date(`${date}T00:00:00Z`);
  // ISO round-trip: JS rolls day-overflow (2026-02-30 -> Mar 2) with a valid
  // getTime() — only exact calendar dates may reach the ledger
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
    const e = new Error(`date '${date}' is not a valid calendar date`);
    e.code = code;
    throw e;
  }
}

/** Today in UTC as yyyy-mm-dd. */
export function todayIso() {
  return new Date().toISOString().slice(0, 10);
}

/**
 * Add n months to a yyyy-mm-dd string; day is clamped to the target month's
 * last day (2026-01-31 + 1 month = 2026-02-28).
 */
export function addMonths(dateStr, n) {
  const [y, m, d] = dateStr.split('-').map(Number);
  const total = y * 12 + (m - 1) + n;
  const yy = Math.floor(total / 12);
  const mm = (total % 12) + 1;
  const last = new Date(Date.UTC(yy, mm, 0)).getUTCDate();
  return `${String(yy).padStart(4, '0')}-${String(mm).padStart(2, '0')}-${String(Math.min(d, last)).padStart(2, '0')}`;
}

/** validateDate with a field label: `${label} '${v}' must be YYYY-MM-DD`. */
export function validateLabeledDate(value, label) {
  if (typeof value !== 'string' || !DATE_RE.test(value)) {
    const e = new Error(`${label} '${value}' must be YYYY-MM-DD`);
    e.code = 'INVALID_DATE';
    throw e;
  }
  const d = new Date(`${value}T00:00:00Z`);
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== value) {
    const e = new Error(`${label} '${value}' is not a valid calendar date`);
    e.code = 'INVALID_DATE';
    throw e;
  }
}
