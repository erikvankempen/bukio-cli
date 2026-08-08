/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// FX translation (Phase 5, FR5.X) — vreemde valuta purchase invoices.
// Rates are stored per currency per date as rate_x10000 (1 EUR = N units of
// foreign currency, scaled by 10000). Conversion is integer math,
// round-half-up: eur_cents = round(fx_cents * 10000 / rate_x10000).
// Missing rates fall back to the ECB reference rates (source='ECB');
// disable the network fallback with BUKIO_FX_NO_FETCH=1.
import { record } from '../audit/index.js';
import { parseAmount } from '../core/money.js';
import { fetchEcbRate } from './ecb.js';

export function fxError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

const ISO4217 = /^[A-Z]{3}$/;

/** Validate + normalise a rate: '1.0875' -> 10875 (x10000, 4 decimals max). */
export function parseRate(rate) {
  const m = String(rate).match(/^(\d+)(?:\.(\d{1,4}))?$/);
  if (!m) throw fxError('INVALID_RATE', `rate '${rate}' must be a positive number (e.g. 1.0875)`);
  const whole = parseInt(m[1], 10);
  const frac = (m[2] ?? '').padEnd(4, '0');
  const rateX10000 = whole * 10000 + parseInt(frac, 10);
  if (!Number.isInteger(rateX10000) || rateX10000 <= 0 || rateX10000 > 100000000) {
    throw fxError('INVALID_RATE', `rate '${rate}' out of range`);
  }
  return rateX10000;
}

/** Convert a foreign-currency amount (integer cents) to EUR cents. */
export function convertFx(fxCents, rateX10000) {
  if (!Number.isInteger(fxCents)) throw fxError('INVALID_FX_AMOUNT', 'fx amounts must be integers (cents)');
  return Math.round((fxCents * 10000) / rateX10000);
}

export function setFxRate(db, { currency, date, rate, source = 'manual', actor = 'human', dryRun = false }) {
  if (!ISO4217.test(currency)) throw fxError('INVALID_CURRENCY', `currency '${currency}' must be ISO 4217 (3 letters)`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw fxError('INVALID_DATE', `date '${date}' must be YYYY-MM-DD`);
  {
    // ISO round-trip: JS rolls day-overflow (2026-02-30 -> Mar 2) — an
    // impossible rate date would never be found by a real booking lookup
    const d = new Date(`${date}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
      throw fxError('INVALID_DATE', `date '${date}' is not a valid calendar date`);
    }
  }
  const rateX10000 = typeof rate === 'number' ? rate : parseRate(rate);
  if (dryRun) {
    return { action: 'fx.set', currency, date, rate: (rateX10000 / 10000).toFixed(4), rate_x10000: rateX10000, dryRun: true };
  }
  db.prepare(`
    INSERT INTO fx_rates (currency, date, rate_x10000, source, created_by)
    VALUES (?, ?, ?, ?, ?)
    ON CONFLICT(currency, date) DO UPDATE SET rate_x10000 = excluded.rate_x10000, source = excluded.source
  `).run(currency, date, rateX10000, source, actor);
  record(db, {
    actor, action: 'fx.set', command: 'fx set',
    args: { currency, date, rate: (rateX10000 / 10000).toFixed(4) }, outcome: 'ok',
  });
  return { currency, date, rate: (rateX10000 / 10000).toFixed(4), rate_x10000: rateX10000 };
}

/**
 * Rate lookup for a booking date: exact date first, else the latest rate
 * on or before that date. Returns rate_x10000 or null.
 */
export function getFxRate(db, { currency, date }) {
  const exact = db.prepare('SELECT rate_x10000 FROM fx_rates WHERE currency = ? AND date = ?').get(currency, date);
  if (exact) return exact.rate_x10000;
  const latest = db.prepare(
    'SELECT rate_x10000 FROM fx_rates WHERE currency = ? AND date <= ? ORDER BY date DESC LIMIT 1',
  ).get(currency, date);
  return latest ? latest.rate_x10000 : null;
}

export function listFxRates(db, { currency = null, limit = 50 } = {}) {
  if (!Number.isInteger(limit) || limit < 0) {
    throw fxError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
  }
  const rows = currency
    ? db.prepare('SELECT * FROM fx_rates WHERE currency = ? ORDER BY date DESC LIMIT ?').all(currency, limit)
    : db.prepare('SELECT * FROM fx_rates ORDER BY date DESC LIMIT ?').all(limit);
  return rows.map((r) => ({
    currency: r.currency, date: r.date,
    rate: (r.rate_x10000 / 10000).toFixed(4), rate_x10000: r.rate_x10000,
    source: r.source, created_by: r.created_by,
  }));
}

/**
 * Resolve the rate for a booking: explicit --rate wins, then the stored rate
 * (exact date, else latest on/before), then — when allowed — a live ECB fetch
 * (stored as source='ECB' for reuse). Throws FX_RATE_NOT_FOUND /
 * ECB_RATE_NOT_AVAILABLE when nothing is available.
 */
export async function resolveRate(db, { currency, rate, date, actor = 'human', noFetch = false }) {
  if (!currency) return null;
  if (rate != null) return parseRate(rate);
  const stored = getFxRate(db, { currency, date });
  if (stored) return stored;
  if (noFetch || process.env.BUKIO_FX_NO_FETCH === '1') {
    throw fxError('FX_RATE_NOT_FOUND', `no FX rate for ${currency} on/before ${date} — set one with 'bukio fx set', pass --rate, or allow the ECB fetch`);
  }
  const fetched = await fetchEcbRate({ currency, date });
  if (!fetched) {
    throw fxError('ECB_RATE_NOT_AVAILABLE', `no ECB reference rate for ${currency} on/before ${date} (not in the ECB set, or before 1999)`);
  }
  setFxRate(db, { currency, date: fetched.date, rate: fetched.rateX10000, source: 'ECB', actor });
  return fetched.rateX10000;
}

/**
 * Convert a posting spec list from a foreign currency into EUR, attaching the
 * original amounts for the audit trail. The vat leg computed later is EUR.
 *   [{ code, amountCents (FX), vatCode? }] ->
 *   [{ code, amountCents (EUR), vatCode?, fxCurrency, fxAmountCents }]
 */
export function toEurPostings(postings, { currency, rateX10000 }) {
  return postings.map((p) => ({
    ...p,
    amountCents: convertFx(p.amountCents, rateX10000),
    fxCurrency: currency,
    fxAmountCents: p.amountCents,
  }));
}
