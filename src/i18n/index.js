/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Central i18n mechanism (S1, owner decision 15 Aug 2026):
//   - t(key, params, locale) resolves a key against the locale table with
//     fallbacks: exact locale -> language part -> 'en' -> the key itself.
//     Regional overrides (nl-be, fr-lu) hold only the keys that differ and
//     fall back to their base language (nl, fr).
//   - resolveLocale(ctx, db) picks the active UI locale: --locale flag (or
//     BUKIO_LOCALE env) -> 'en'. UI text is English by default — the
//     company's stored locale drives DOCUMENTS (invoice.language), not the
//     CLI surface; localization is an explicit opt-in.
//   - Supported tables (one module per language under ./locales/): en
//     (default; also serves en-GB/en-US), nl (NL), nl-be (BE Dutch),
//     de (DE), fr (FR; base for fr-lu), fr-lu (LU), da (DK), fi (FI),
//     nb (NO), sv (SE), it (IT), es (ES), pt (PT), bg (BG), hr (HR),
//     sl (SI), et (EE), lv (LV), lt (LT), mt (MT), cy (CY), cs (CZ),
//     sk (SK), el (GR), pl (PL), hu (HU), ro (RO), sq (XK) — 26 full
//     tables + 2 regional overrides.
//   - Statutory artifacts (XAF/FAIA, jaarrekening models, OB readout),
//     JSON output keys, error codes and MCP descriptions never localize
//     (per the Aug 14 decision: documents localize, UI/JSON stay English).
// Line descriptions and account names are data, never auto-translated.

export const UNIT_CODES = ['h', 'day', 'month', 'unit', 'session', 'km', 'kg', 'project'];

import bg from './locales/bg.js';
import cs from './locales/cs.js';
import cy from './locales/cy.js';
import da from './locales/da.js';
import de from './locales/de.js';
import el from './locales/el.js';
import en from './locales/en.js';
import es from './locales/es.js';
import et from './locales/et.js';
import fi from './locales/fi.js';
import fr from './locales/fr.js';
import frLu from './locales/fr-lu.js';
import hr from './locales/hr.js';
import hu from './locales/hu.js';
import it from './locales/it.js';
import lt from './locales/lt.js';
import lv from './locales/lv.js';
import mt from './locales/mt.js';
import nb from './locales/nb.js';
import nl from './locales/nl.js';
import nlBe from './locales/nl-be.js';
import pl from './locales/pl.js';
import pt from './locales/pt.js';
import ro from './locales/ro.js';
import sq from './locales/sq.js';
import sk from './locales/sk.js';
import sl from './locales/sl.js';
import sv from './locales/sv.js';

export const TABLES = { en, nl, 'nl-be': nlBe, de, fr, 'fr-lu': frLu, da, fi, nb, sv, it, es, pt, bg, hr, sl, et, lv, lt, mt, cy, cs, sk, el, pl, hu, ro, sq };


/** Resolve the active UI locale: --locale flag > BUKIO_LOCALE env > 'en'.
 *
 * UI text is English by default (Aug 14 decision: UI/JSON stay English;
 * localization is an explicit opt-in via --locale / BUKIO_LOCALE). The
 * company's stored locale drives DOCUMENTS (invoice.language), not the CLI
 * surface — a company locale of 'nl' (the migration default) must not flip
 * the whole UI back to Dutch. */
export function resolveLocale(ctx = {}, db = null) {
  ctx = ctx ?? {};
  if (ctx.locale) return ctx.locale;
  if (process.env.BUKIO_LOCALE) return process.env.BUKIO_LOCALE;
  return 'en';
}

/** Translate key with {param} interpolation; fallback: exact locale ->
 *  base language (nl-be -> nl, fr-lu -> fr, de-DE -> de, en-GB -> en) ->
 *  'en' -> the key itself. */
export function t(key, params = {}, locale = 'en') {
  const loc = String(locale || 'en').toLowerCase();
  const base = loc.includes('-') ? loc.split('-')[0] : null;
  const s0 = TABLES[loc]?.[key] ?? (base ? TABLES[base]?.[key] : undefined) ?? TABLES.en[key] ?? key;
  let s = s0;
  for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** Legacy invoice-label API (was src/invoice/i18n.js) — label(k, 'nl'|'en'). */
export function label(key, language = 'nl') {
  return t(`pdf.${key}`, {}, language);
}

/** Legacy unit-label API (was src/invoice/i18n.js). */
export function unitLabel(code, language = 'nl') {
  const s = t(`unit.${code}`, {}, language);
  return s === `unit.${code}` ? (code ?? '') : s;
}

// Backwards-compatible exports for importers of src/invoice/i18n.js.
export const LABELS = {
  nl: Object.fromEntries(Object.entries(TABLES.nl).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
  en: Object.fromEntries(Object.entries(TABLES.en).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
};
export const UNITS = Object.fromEntries(
  UNIT_CODES.map((c) => [c, { nl: TABLES.nl[`unit.${c}`], en: TABLES.en[`unit.${c}`] }]),
);
