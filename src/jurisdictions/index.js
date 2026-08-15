/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Jurisdiction profile registry (Phase A — jurisdiction-profile layer).
//
// A jurisdiction profile ("country pack") is a static, versioned ES module
// per ISO 3166-1 alpha-2 country code that carries every jurisdiction-specific
// constant and rule the engine needs: tax codes/accounts, chart taxonomy and
// labels, compliance filing types, identifiers, e-invoicing profile, closing
// accounts. The NL profile (./nl.js) is the reference implementation — cut
// verbatim from the previously hardcoded module constants, so NL behavior is
// byte-identical. LU (./lu.js) is the first Phase B market profile (PCN 2020
// chart, French labels), GB (./gb.js) the second (QuickBooks/Xero-style
// chart, GBP), FR (./fr.js) the third (PCG chart), US (./us.js) the fourth
// (no-VAT, state-level sales tax) and BE (./be.js) the fifth (PCN-BE
// minimum plan); each registers only formats with existing builders —
// anything else fails loudly via the strict dispatch.
//
// Consumers must resolve profiles ONLY through this registry — never read
// company.country directly (see the profile-sprawl rule in the Phase A plan).
import be from './be.js';
import fr from './fr.js';
import gb from './gb.js';
import lu from './lu.js';
import nl from './nl.js';
import us from './us.js';

/** ISO 3166-1 alpha-2 country codes that are valid but not implemented yet. */
export const PLANNED = ['DE', 'DK', 'FI', 'NO', 'SE'];

const PROFILES = {
  NL: deepFreeze(nl), LU: deepFreeze(lu), GB: deepFreeze(gb), FR: deepFreeze(fr), US: deepFreeze(us), BE: deepFreeze(be),
};

export function jurisdictionError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** Normalise a country input: trim + uppercase. Null/empty → null. */
export function normalizeCountry(country) {
  if (country == null) return null;
  const s = String(country).trim().toUpperCase();
  return s === '' ? null : s;
}

/** Deep-freeze an object graph (profiles must stay static). */
function deepFreeze(obj) {
  for (const v of Object.values(obj)) {
    if (v && typeof v === 'object') deepFreeze(v);
  }
  return Object.freeze(obj);
}

/**
 * Get a profile by country code (pure — no DB).
 * - malformed input (not a 2-letter code) → INVALID_COUNTRY
 * - valid code with no profile → PROFILE_NOT_FOUND
 * - valid code in PLANNED (implemented later) → COUNTRY_NOT_SUPPORTED
 * Returns the deep-frozen profile module.
 */
export function getProfile(country) {
  const cc = normalizeCountry(country);
  if (!cc || !/^[A-Z]{2}$/.test(cc)) {
    throw jurisdictionError('INVALID_COUNTRY', `country '${country}' must be an ISO 3166-1 alpha-2 code (e.g. NL)`);
  }
  const profile = PROFILES[cc];
  if (!profile) {
    if (PLANNED.includes(cc)) {
      throw jurisdictionError('COUNTRY_NOT_SUPPORTED', `country ${cc} is not supported yet — supported: ${Object.keys(PROFILES).join(', ')}`);
    }
    throw jurisdictionError('PROFILE_NOT_FOUND', `no jurisdiction profile for country ${cc}`);
  }
  return profile;
}

/**
 * Resolve the profile for a company DB (reads company.country).
 * - no company row yet (pre-init) or no country column (pre-migration-021)
 *   → 'NL' default
 * - company.country set → getProfile(country) (throws PROFILE_NOT_FOUND /
 *   COUNTRY_NOT_SUPPORTED per decision §9.1.6 — never a silent fallback)
 * Consumers must use THIS resolver — never read company.country directly.
 */
export function resolveProfile(db) {
  let country = null;
  try {
    const row = db.prepare('SELECT country FROM company WHERE id = 1').get();
    country = row ? row.country : null;
  } catch (err) {
    // pre-021 schema: no country column — NL default. Anything else
    // (locked DB, corruption) must propagate, not silently default to NL.
    if (!String(err.message).includes('no such column: country')) throw err;
    country = null;
  }
  return getProfile(country ?? 'NL');
}
