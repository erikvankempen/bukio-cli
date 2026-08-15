/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Jaarrekening (Phase 4) — statutory annual accounts for micro (art. 2:395a)
// and klein (art. 2:396) BVs, in the Dutch prescribed layout. Built from the
// RGS-mapped chart via the balans/pnl engines, with an Overig catch-all so
// custom accounts are never silently dropped.
import { balans } from './balans.js';
import { pnl } from './pnl.js';
import { formatAmount } from '../core/money.js';
import { resolveProfile } from '../jurisdictions/index.js';
import { fiscalYearWindow } from '../year-end/index.js';

export function jaarrekeningError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// Jaarrekening builders keyed by profile.reporting.format. NL is the only
// format in Phase A ('auto'); future markets register their own layout.
const JAARREKENING_FORMATS = {
  auto: buildJaarrekeningAuto,
};

function groupSections(sections, lines) {
  const known = new Set(lines.map((l) => l.rgs));
  const normalize = (g) => ({
    ...g,
    // stable account shape: amount_cents everywhere (balans sections carry
    // balance_cents internally — the PDF/XLSX renderers must never see NaN)
    accounts: (g.accounts ?? []).map((a) => ({
      code: a.code,
      name: a.name,
      amount_cents: Number.isFinite(a.balance_cents) ? a.balance_cents : (a.amount_cents ?? 0),
    })),
  });
  const out = [];
  for (const line of lines) {
    const hits = sections.filter((s) => s.taxonomy_code === line.rgs);
    if (!hits.length) continue;
    out.push({
      label: line.label,
      taxonomy_code: line.rgs,
      sections: hits.map(normalize),
      total_cents: hits.reduce((s, g) => s + g.total_cents, 0),
    });
  }
  const leftover = sections.filter((s) => !known.has(s.taxonomy_code));
  if (leftover.length) {
    out.push({
      label: 'Overig',
      taxonomy_code: null,
      sections: leftover.map(normalize),
      total_cents: leftover.reduce((s, g) => s + g.total_cents, 0),
    });
  }
  return out;
}

export function jaarrekening(db, { year, model = 'klein' }) {
  const { reporting } = resolveProfile(db);
  const builder = JAARREKENING_FORMATS[reporting.format];
  if (!builder) {
    throw jaarrekeningError('FORMAT_NOT_SUPPORTED', `financial statements format '${reporting.format}' has no builder (registered: ${Object.keys(JAARREKENING_FORMATS).join(', ')})`);
  }
  return builder(db, { year, model, reporting });
}

function buildJaarrekeningAuto(db, { year, model, reporting }) {
  const MODELS = reporting.statutoryAccounts.models;
  const LINES = reporting.statutoryAccounts.lines;
  if (!MODELS.includes(model)) throw jaarrekeningError('INVALID_MODEL', `model must be one of ${MODELS.join(', ')}`);
  if (!/^\d{4}$/.test(String(year))) throw jaarrekeningError('INVALID_YEAR', `year '${year}' must be YYYY`);
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  if (!company) throw jaarrekeningError('NOT_INITIALISED', 'company database not initialised');

  // tolerant like fiscalYearWindow/jaarrekeningDeadline: fiscal_year_end may
  // be 'MM-DD' or a full 'YYYY-MM-DD' — concatenating blindly would produce
  // '2026-2026-12-31' for the full-date form and break the whole report
  const fyeParts = String(company.fiscal_year_end || '12-31').split('-');
  const fyeMonth = fyeParts[fyeParts.length - 2];
  const fyeDay = fyeParts[fyeParts.length - 1];
  const asOf = `${year}-${String(fyeMonth).padStart(2, '0')}-${String(fyeDay).padStart(2, '0')}`;
  const b = balans(db, { asOf });

  const activa = groupSections(b.assets.sections, LINES.activa);
  const passivaSections = groupSections(b.liabilities_and_equity.sections, LINES.passiva);
  // onverdeeld resultaat folds into Eigen vermogen (pre-close) as its own line
  if (b.liabilities_and_equity.result_cents !== 0) {
    const ev = passivaSections.find((s) => s.taxonomy_code === 'BEIV.05');
    if (ev) {
      ev.total_cents += b.liabilities_and_equity.result_cents;
      ev.sections.push({
        taxonomy_code: null, label: 'Onverdeeld resultaat',
        accounts: [{ code: '—', name: 'Resultaat boekjaar', amount_cents: b.liabilities_and_equity.result_cents }],
        total_cents: b.liabilities_and_equity.result_cents,
      });
    } else {
      passivaSections.push({
        label: 'Eigen vermogen', taxonomy_code: 'BEIV.05',
        sections: [{
          taxonomy_code: null, label: 'Onverdeeld resultaat',
          accounts: [{ code: '—', name: 'Resultaat boekjaar', amount_cents: b.liabilities_and_equity.result_cents }],
          total_cents: b.liabilities_and_equity.result_cents,
        }],
        total_cents: b.liabilities_and_equity.result_cents,
      });
    }
  }
  const totalActiva = activa.reduce((s, g) => s + g.total_cents, 0);
  const totalPassiva = passivaSections.reduce((s, g) => s + g.total_cents, 0);

  const report = {
    year,
    model,
    company: {
      name: company.name, kvk: company.registration_id, btw_id: company.tax_id,
      legal_form: company.legal_form, address: company.address,
      postal_code: company.postal_code, city: company.city,
    },
    as_of: asOf,
    balans: {
      activa,
      passiva: passivaSections,
      total_activa_cents: totalActiva,
      total_passiva_cents: totalPassiva,
      balanced: totalActiva === totalPassiva,
    },
  };

  if (model === 'klein') {
    // The P&L covers the FISCAL year, not the calendar year: a company with
    // fiscal_year_end 03-31 must report 2025-04-01..2026-03-31 for year
    // 2026 — the same period the balans peildatum closes and the year-end
    // close covers (shared fiscalYearWindow). Calendar-year hardcoding
    // showed 9 wrong months and missed 3 months of the FY.
    const [pnlFrom, pnlTo] = fiscalYearWindow(db, year);
    const p = pnl(db, { from: pnlFrom, to: pnlTo });
    const pnlLines = groupSections(p.sections, LINES.pnl);
    const omzet = pnlLines.find((l) => l.taxonomy_code === 'WOMZ.80')?.total_cents ?? 0;
    const overige = pnlLines.find((l) => l.taxonomy_code === 'WOVB.82')?.total_cents ?? 0;
    const inkoop = pnlLines.find((l) => l.taxonomy_code === 'WKPR.70')?.total_cents ?? 0;
    // pure operating costs: everything except omzet (80), inkoopwaarde (70)
    // and overige bedrijfsopbrengsten (82) — WKPR.70 must NOT be inside
    // kosten or resultaat would subtract it twice
    const kosten = pnlLines.filter((l) => !['WOMZ.80', 'WKPR.70', 'WOVB.82'].includes(l.taxonomy_code))
      .reduce((s, l) => s + l.total_cents, 0);
    report.pnl = {
      lines: pnlLines,
      omzet_cents: omzet,
      overige_opbrengsten_cents: overige,
      inkoop_cents: inkoop,
      bruto_marge_cents: omzet - inkoop,
      kosten_cents: kosten,
      resultaat_cents: omzet + overige - inkoop - kosten,
      resultaat: formatAmount(omzet + overige - inkoop - kosten),
    };
  }
  return report;
}
