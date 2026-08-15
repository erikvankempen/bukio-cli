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
  // Phase B2: Luxembourg LSC abridged layout (docs-research/lu-pcn-2020.md §8)
  'lu-lsc': buildJaarrekeningLu,
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

export function jaarrekening(db, { year, model = null }) {
  const { reporting } = resolveProfile(db);
  const builder = JAARREKENING_FORMATS[reporting.format];
  if (!builder) {
    throw jaarrekeningError('FORMAT_NOT_SUPPORTED', `financial statements format '${reporting.format}' has no builder (registered: ${Object.keys(JAARREKENING_FORMATS).join(', ')})`);
  }
  // the model default is profile-driven: the last registered model (NL
  // 'klein', LU 'abrege') — the SME default per country
  const MODELS = reporting.statutoryAccounts.models;
  return builder(db, { year, model: model ?? MODELS[MODELS.length - 1], reporting });
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

/**
 * Luxembourg LSC annual accounts (Phase B2) — the abridged (abrégé) scheme
 * per the PCN 2020 tableau de passage (docs-research/lu-pcn-2020.md §8),
 * which is what most SMEs file. PCN accounts carry no taxonomy codes, so the
 * statutory lines are grouped by PCN class PREFIXES (imputation accounts,
 * codes verbatim from the official RGD annex); asset/liability overlap is
 * resolved by the balans engine's side separation. The unclosed year result
 * folds into Capitaux propres like the NL 'Onverdeeld resultaat' line.
 */
function buildJaarrekeningLu(db, { year, model, reporting }) {
  const MODELS = reporting.statutoryAccounts.models;
  if (!MODELS.includes(model)) throw jaarrekeningError('INVALID_MODEL', `model must be one of ${MODELS.join(', ')}`);
  if (!/^\d{4}$/.test(String(year))) throw jaarrekeningError('INVALID_YEAR', `year '${year}' must be YYYY`);
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  if (!company) throw jaarrekeningError('NOT_INITIALISED', 'company database not initialised');

  const fyeParts = String(company.fiscal_year_end || '12-31').split('-');
  const asOf = `${year}-${String(fyeParts[fyeParts.length - 2]).padStart(2, '0')}-${String(fyeParts[fyeParts.length - 1]).padStart(2, '0')}`;
  const b = balans(db, { asOf });

  const flatten = (sections) => sections
    .flatMap((s) => (s.accounts ?? []).map((a) => ({ code: a.code, name: a.name, balance_cents: a.balance_cents ?? 0 })));

  const groupByPrefix = (accounts, lines) => {
    const out = [];
    for (const line of lines) {
      const hits = accounts.filter((a) => line.prefixes.some((p) => String(a.code).startsWith(p)));
      if (!hits.length) continue;
      out.push({
        label: line.label,
        prefixes: line.prefixes,
        accounts: hits.map((a) => ({ code: a.code, name: a.name, amount_cents: a.balance_cents })),
        total_cents: hits.reduce((s, a) => s + a.balance_cents, 0),
      });
    }
    const known = lines.flatMap((l) => l.prefixes);
    const leftover = accounts.filter((a) => !known.some((p) => String(a.code).startsWith(p)));
    if (leftover.length) {
      out.push({
        label: 'Autres', prefixes: [], accounts: leftover.map((a) => ({ code: a.code, name: a.name, amount_cents: a.balance_cents })),
        total_cents: leftover.reduce((s, a) => s + a.balance_cents, 0),
      });
    }
    return out;
  };

  const LINES = reporting.statutoryAccounts.lines;
  const activa = groupByPrefix(flatten(b.assets.sections), LINES.activa);
  const passiva = groupByPrefix(flatten(b.liabilities_and_equity.sections), LINES.passiva);

  // unclosed result folds into Capitaux propres (pre-close)
  const resultCents = b.liabilities_and_equity.result_cents ?? 0;
  if (resultCents !== 0) {
    const cp = passiva.find((l) => l.label === 'Capitaux propres');
    const entry = { code: '142', name: 'Résultat de l\'exercice (non affecté)', amount_cents: resultCents };
    if (cp) {
      cp.total_cents += resultCents;
      cp.accounts.push(entry);
    } else {
      passiva.push({
        label: 'Capitaux propres', prefixes: [], accounts: [entry], total_cents: resultCents,
      });
    }
  }
  const totalActiva = activa.reduce((s, g) => s + g.total_cents, 0);
  const totalPassiva = passiva.reduce((s, g) => s + g.total_cents, 0);

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
      passiva,
      total_activa_cents: totalActiva,
      total_passiva_cents: totalPassiva,
      balanced: totalActiva === totalPassiva,
    },
  };

  // P&L over the fiscal year (shared fiscalYearWindow with the balans close)
  const [pnlFrom, pnlTo] = fiscalYearWindow(db, year);
  const p = pnl(db, { from: pnlFrom, to: pnlTo });
  const pnlAccounts = p.sections.flatMap((s) => (s.accounts ?? []).map((a) => ({ code: a.code, name: a.name, type: a.type, amount_cents: a.amount_cents ?? 0 })));
  const pnlLines = [];
  for (const line of LINES.pnl) {
    const hits = pnlAccounts.filter((a) => line.prefixes.some((x) => String(a.code).startsWith(x)));
    if (!hits.length) continue;
    pnlLines.push({
      label: line.label,
      sign: line.sign ?? 1,
      accounts: hits,
      total_cents: hits.reduce((s, a) => s + a.amount_cents, 0),
    });
  }
  const knownPnl = LINES.pnl.flatMap((l) => l.prefixes);
  const leftoverPnl = pnlAccounts.filter((a) => !knownPnl.some((x) => String(a.code).startsWith(x)));
  if (leftoverPnl.length) {
    // per-account sign: pnl exposes the account type, so a leftover custom
    // expense (the realistic case — PCN classes 70-75 cover every standard
    // income class incl. 73x subventions) subtracts, while a custom income
    // account (e.g. 76x/77x, not standard PCN) adds. The line total stays
    // the raw amount sum (display convention: charge lines positive, the
    // sign does the math); for mixed leftovers the net decides.
    const net = leftoverPnl.reduce((s, a) => s + (a.type === 'income' ? a.amount_cents : -a.amount_cents), 0);
    pnlLines.push({
      label: 'Autres', sign: net < 0 ? -1 : 1, accounts: leftoverPnl,
      total_cents: leftoverPnl.reduce((s, a) => s + a.amount_cents, 0),
    });
  }
  // resultat: produits (sign +1) minus charges (sign -1)
  const resultatCents = pnlLines
    .filter((l) => l.sign !== undefined)
    .reduce((s, l) => s + l.sign * l.total_cents, 0);
  report.pnl = {
    lines: pnlLines,
    resultat_cents: resultatCents,
    resultat: formatAmount(resultatCents),
  };
  return report;
}
