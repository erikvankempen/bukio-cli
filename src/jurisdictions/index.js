/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
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
// (no-VAT, state-level sales tax), BE (./be.js) the fifth (PCN-BE minimum
// plan), DE (./de.js) the sixth (DATEV SKR 03), DK (./dk.js) the seventh
// (Standardkontoplan-aligned, DKK), FI (./fi.js) the eighth (Liikekirjuri
// model chart), NO (./no.js) the ninth (NS 4102, NOK), SE (./se.js) the
// tenth (BAS 2023, SEK), AT (./at.js) the eleventh (EKR, EUR), IE
// (./ie.js) the twelfth (UK-style chart, EUR), IT (./it.js) the thirteenth
// (commercialisti convention, EUR), ES (./es.js) the fourteenth (PGC, EUR),
// PT (./pt.js) the fifteenth (SNC, EUR), BG/HR/SI/EE/LV/LT/MT/CY the
// seventeenth to twenty-fourth (Phase E — eight EUR markets,
// English-document defaults), CZ/SK/GR/PL/HU/RO the twenty-fifth to
// thirtieth (Phase F — the final six EU members, English-document
// English-document defaults); each registers only formats with existing builders —
// anything else fails loudly via the strict dispatch. All thirty-one markets live
// (27/27 EU members + GB/NO/XK/US); CH is parked (CHF base currency, QR-bill,
// not a Peppol country).
//
// Consumers must resolve profiles ONLY through this registry — never read
// company.country directly (see the profile-sprawl rule in the Phase A plan).
import at from './at.js';
import be from './be.js';
import bg from './bg.js';
import cy from './cy.js';
import cz from './cz.js';
import ee from './ee.js';
import es from './es.js';
import de from './de.js';
import dk from './dk.js';
import fi from './fi.js';
import fr from './fr.js';
import gr from './gr.js';
import gb from './gb.js';
import hr from './hr.js';
import hu from './hu.js';
import ie from './ie.js';
import it from './it.js';
import lt from './lt.js';
import lu from './lu.js';
import lv from './lv.js';
import mt from './mt.js';
import nl from './nl.js';
import no from './no.js';
import pl from './pl.js';
import pt from './pt.js';
import ro from './ro.js';
import se from './se.js';
import si from './si.js';
import sk from './sk.js';
import us from './us.js';
import xk from './xk.js';

// every code across ALL registered profiles — the invoice line-spec parser
// uses this union to RECOGNISE a VAT-code token (validation still happens
// against the ACTIVE profile's codes, so a foreign code fails loudly with
// VAT_CODE_NOT_FOUND). Before the multi-jurisdiction expansion the parser
// only knew the NL codes: FR's dotted rates '5.5'/'2.1' were not
// recognised and silently mis-parsed as the line price.
export function allTaxCodes() {
  return [...new Set(Object.values(PROFILES).flatMap((p) => p.tax.codes.map((c) => c.code)))];
}

const PROFILES = {
  NL: deepFreeze(nl), LU: deepFreeze(lu), GB: deepFreeze(gb), FR: deepFreeze(fr), US: deepFreeze(us), BE: deepFreeze(be), DE: deepFreeze(de), DK: deepFreeze(dk), FI: deepFreeze(fi), NO: deepFreeze(no), SE: deepFreeze(se), AT: deepFreeze(at), IE: deepFreeze(ie), ES: deepFreeze(es), IT: deepFreeze(it), PT: deepFreeze(pt), BG: deepFreeze(bg), HR: deepFreeze(hr), SI: deepFreeze(si), EE: deepFreeze(ee), LV: deepFreeze(lv), LT: deepFreeze(lt), MT: deepFreeze(mt), CY: deepFreeze(cy), CZ: deepFreeze(cz), SK: deepFreeze(sk), GR: deepFreeze(gr), PL: deepFreeze(pl), HU: deepFreeze(hu), RO: deepFreeze(ro), XK: deepFreeze(xk),
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
 * Returns the deep-frozen profile module.
 */
export function getProfile(country) {
  const cc = normalizeCountry(country);
  if (!cc || !/^[A-Z]{2}$/.test(cc)) {
    throw jurisdictionError('INVALID_COUNTRY', `country '${country}' must be an ISO 3166-1 alpha-2 code (e.g. NL)`);
  }
  const profile = PROFILES[cc];
  if (!profile) {
    throw jurisdictionError('PROFILE_NOT_FOUND', `no jurisdiction profile for country ${cc}`);
  }
  return profile;
}

/**
 * Resolve the profile for a company DB (reads company.country).
 * - no company row yet (pre-init) or no country column (pre-migration-021)
 *   → 'NL' default
 *   → getProfile(country) (throws PROFILE_NOT_FOUND — never a silent fallback)
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
