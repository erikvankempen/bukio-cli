// Jaarrekening (Phase 4) — statutory annual accounts for micro (art. 2:395a)
// and klein (art. 2:396) BVs, in the Dutch prescribed layout. Built from the
// RGS-mapped chart via the balans/pnl engines, with an Overig catch-all so
// custom accounts are never silently dropped.
import { balans } from './balans.js';
import { pnl } from './pnl.js';
import { formatAmount } from '../core/money.js';

export function jaarrekeningError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

const MODELS = ['micro', 'klein'];

// statutory line labels per RGS hoofdgroep
const ACTIVA_LINES = [
  { rgs: 'BMVA.02', label: 'Materiële vaste activa' },
  { rgs: 'BIVA.04', label: 'Immateriële vaste activa' },
  { rgs: 'BFVA.03', label: 'Financiële vaste activa' },
  { rgs: 'BVRD.30', label: 'Voorraden' },
  { rgs: 'BVOR.11', label: 'Vorderingen' },
  { rgs: 'BLIM.10', label: 'Liquide middelen' },
];
const PASSIVA_LINES = [
  { rgs: 'BEIV.05', label: 'Eigen vermogen' },
  { rgs: 'BVRZ.07', label: 'Voorzieningen' },
  { rgs: 'BLAS.08', label: 'Langlopende schulden' },
  { rgs: 'BSCH.12', label: 'Kortlopende schulden' },
];
const PNL_LINES = [
  { rgs: 'WOMZ.80', label: 'Netto-omzet' },
  { rgs: 'WOVB.82', label: 'Overige bedrijfsopbrengsten' },
  { rgs: 'WKPR.70', label: 'Inkoopwaarde van de omzet' },
  { rgs: 'WBED.42', label: 'Overige bedrijfskosten' },
  { rgs: 'WAFS.41', label: 'Afschrijvingen' },
  // the chart system tags financiële baten en lasten as WFBE.84 (rgsLabel);
  // WBEL.60 is kept for imported charts that use the official code
  { rgs: 'WFBE.84', label: 'Financiële baten en lasten' },
  { rgs: 'WBEL.60', label: 'Belastingen' },
];

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
    const hits = sections.filter((s) => s.rgs_code === line.rgs);
    if (!hits.length) continue;
    out.push({
      label: line.label,
      rgs_code: line.rgs,
      sections: hits.map(normalize),
      total_cents: hits.reduce((s, g) => s + g.total_cents, 0),
    });
  }
  const leftover = sections.filter((s) => !known.has(s.rgs_code));
  if (leftover.length) {
    out.push({
      label: 'Overig',
      rgs_code: null,
      sections: leftover.map(normalize),
      total_cents: leftover.reduce((s, g) => s + g.total_cents, 0),
    });
  }
  return out;
}

export function jaarrekening(db, { year, model = 'klein' }) {
  if (!MODELS.includes(model)) throw jaarrekeningError('INVALID_MODEL', `model must be one of ${MODELS.join(', ')}`);
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  if (!company) throw jaarrekeningError('NOT_INITIALISED', 'company database not initialised');

  const asOf = `${year}-${company.fiscal_year_end || '12-31'}`;
  const b = balans(db, { asOf });

  const activa = groupSections(b.assets.sections, ACTIVA_LINES);
  const passivaSections = groupSections(b.liabilities_and_equity.sections, PASSIVA_LINES);
  // onverdeeld resultaat folds into Eigen vermogen (pre-close) as its own line
  if (b.liabilities_and_equity.result_cents !== 0) {
    const ev = passivaSections.find((s) => s.rgs_code === 'BEIV.05');
    if (ev) {
      ev.total_cents += b.liabilities_and_equity.result_cents;
      ev.sections.push({
        rgs_code: null, label: 'Onverdeeld resultaat',
        accounts: [{ code: '—', name: 'Resultaat boekjaar', amount_cents: b.liabilities_and_equity.result_cents }],
        total_cents: b.liabilities_and_equity.result_cents,
      });
    } else {
      passivaSections.push({
        label: 'Eigen vermogen', rgs_code: 'BEIV.05',
        sections: [{
          rgs_code: null, label: 'Onverdeeld resultaat',
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
      name: company.name, kvk: company.kvk, btw_id: company.btw_id,
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
    const p = pnl(db, { from: `${year}-01-01`, to: `${year}-12-31` });
    const pnlLines = groupSections(p.sections, PNL_LINES);
    const omzet = pnlLines.find((l) => l.rgs_code === 'WOMZ.80')?.total_cents ?? 0;
    const inkoop = pnlLines.find((l) => l.rgs_code === 'WKPR.70')?.total_cents ?? 0;
    const kosten = pnlLines.filter((l) => !['WOMZ.80', 'WOVB.82'].includes(l.rgs_code))
      .reduce((s, l) => s + l.total_cents, 0);
    report.pnl = {
      lines: pnlLines,
      omzet_cents: omzet,
      inkoop_cents: inkoop,
      bruto_marge_cents: omzet - inkoop,
      kosten_cents: kosten,
      resultaat_cents: omzet - inkoop - kosten,
      resultaat: formatAmount(omzet - inkoop - kosten),
    };
  }
  return report;
}
