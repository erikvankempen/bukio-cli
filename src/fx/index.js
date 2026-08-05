// FX translation (Phase 5, FR5.X) — vreemde valuta purchase invoices.
// Rates are stored per currency per date as rate_x10000 (1 EUR = N units of
// foreign currency, scaled by 10000). Conversion is integer math,
// round-half-up: eur_cents = round(fx_cents * 10000 / rate_x10000).
import { record } from '../audit/index.js';
import { parseAmount } from '../core/money.js';

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

export function setFxRate(db, { currency, date, rate, source = 'manual', actor = 'human' }) {
  if (!ISO4217.test(currency)) throw fxError('INVALID_CURRENCY', `currency '${currency}' must be ISO 4217 (3 letters)`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw fxError('INVALID_DATE', `date '${date}' must be YYYY-MM-DD`);
  const rateX10000 = typeof rate === 'number' ? rate : parseRate(rate);
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
