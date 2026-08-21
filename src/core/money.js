/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Money handling — integer cents everywhere, no floats in financial code paths.

const AMOUNT_RE = /^-?\d+(\.\d{1,2})?$/;

export function moneyError(message) {
  const e = new Error(message);
  e.code = 'INVALID_AMOUNT';
  return e;
}

/**
 * Parse a decimal amount string to integer cents.
 * Strict international format: optional '-', digits, optional single '.'
 * with max 2 decimals. Thousands separators are rejected on purpose
 * (agents output international format; ambiguity is the enemy).
 */
export function parseAmount(input) {
  if (typeof input !== 'string') {
    throw moneyError(`amount must be a string, got ${typeof input}`);
  }
  const s = input.trim();
  if (!AMOUNT_RE.test(s)) {
    throw moneyError(`invalid amount '${input}' — use e.g. 1234.56 (max 2 decimals, no thousands separators)`);
  }
  const negative = s.startsWith('-');
  const [whole, frac = ''] = s.replace('-', '').split('.');
  const cents = parseInt(whole, 10) * 100 + parseInt(frac.padEnd(2, '0') || '0', 10);
  // '-0' must normalize to 0 (never -0 — strict equality and Object.is treat
  // -0 differently, and it leaks into reports as '-0.00' in edge renderers)
  return negative && cents !== 0 ? -cents : cents;
}

/** Format integer cents as "1234.56" (always 2 decimals, '-' prefix for negative). */
export function formatAmount(cents) {
  const abs = Math.abs(cents);
  const sign = cents < 0 ? '-' : '';
  return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, '0')}`;
}
