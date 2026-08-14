/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Default chart of accounts (Phase 1 — expanded, RGS-mapped).
// VAT-agnostic by design: no BTW accounts here — the VAT module (Phase 2)
// adds them when enabled.
//
// taxonomy_code values are RGS *hoofdgroep* (niveau 2) reference codes per the
// official Referentie Grootboekschema (source: RGS documentation, e.g.
// referentiegrootboekschema.nl / RGS hoofdgroepen list). Full rekening-level
// taxonomy sync is a later enhancement; the CSV import (`account import`)
// allows any custom chart with precise RGS codes.
//
// Owner drawings (privé opnamen) book directly against 3000 Eigen vermogen
// (debit 3000, credit bank) — no contra-equity account in the core chart.

import { getProfile } from '../jurisdictions/index.js';

/** Infer an RGS code for an account coming from an external source (audit
 *  files, journal CSVs, chart CSVs without an rgs column). Keyword matching
 *  within the account type first, then a type-based fallback. Imported
 *  charts MUST carry RGS codes — reports (P&L, balans) and the jaarrekening
 *  group by them.
 */
export function inferRgs(type, name) {
  const n = String(name ?? '').toLowerCase();
  const has = (...kws) => kws.some((k) => n.includes(k));
  switch (type) {
    case 'income':
      if (has('diensten', 'service')) return 'WOVB.82';
      if (has('omzet', 'verkopen', 'verkoop')) return 'WOMZ.80';
      return 'WOVB.82'; // overige opbrengsten / rentebaten / subsidies
    case 'expense':
      if (has('inkoop', 'voorraad', 'uitbesteed')) return 'WKPR.70';
      if (has('personeel', 'loon', 'salaris', 'sociale', 'pensioen')) return 'WPER.40';
      if (has('afschrijving', 'afschr')) return 'WAFS.41';
      if (has('rente', 'financie', 'interest', 'bankkosten')) return 'WFBE.84';
      return 'WBED.42'; // all other operating costs
    case 'asset':
      if (has('bank', 'kas', 'geld', 'tegoed', 'spaar', 'sumup', 'onefor', 'business')) return 'BLIM.10';
      if (has('debiteur', 'vorderen', 'vraagpost', 'kruispost', 'vooruitbetaald', 'nog te ontvangen')) return 'BVOR.11';
      if (has('voorraad')) return 'BVRD.30';
      return 'BMVA.02'; // materiële vaste activa (incl. contra afschrijvingen)
    case 'liability':
      return 'BSCH.12';
    case 'equity':
      return 'BEIV.05';
    default:
      return null;
  }
}

// NL profile data is the single source of truth (Phase A M5) — the legacy
// literals moved verbatim into src/jurisdictions/nl.js (reporting.defaultChart
// + reporting.labels); these exports keep the rest of the codebase untouched
export const DEFAULT_CHART = getProfile('NL').reporting.defaultChart;

/** RGS hoofdgroep labels (official RGS nomenclature, Dutch). */
export const RGS_LABELS = getProfile('NL').reporting.labels;

export function rgsLabel(code) {
  return RGS_LABELS[code] ?? code ?? 'Overig';
}
