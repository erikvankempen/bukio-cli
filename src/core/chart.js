// Default chart of accounts (Phase 1 — expanded, RGS-mapped).
// VAT-agnostic by design: no BTW accounts here — the VAT module (Phase 2)
// adds them when enabled.
//
// rgs_code values are RGS *hoofdgroep* (niveau 2) reference codes per the
// official Referentie Grootboekschema (source: RGS documentation, e.g.
// referentiegrootboekschema.nl / RGS hoofdgroepen list). Full rekening-level
// taxonomy sync is a later enhancement; the CSV import (`account import`)
// allows any custom chart with precise RGS codes.
//
// Owner drawings (privé opnamen) book directly against 3000 Eigen vermogen
// (debit 3000, credit bank) — no contra-equity account in the core chart.

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

export const DEFAULT_CHART = [
  // Activa
  { code: '1000', name: 'Kas', type: 'asset', normalBalance: 'debit', rgsCode: 'BLIM.10' },
  { code: '1100', name: 'Bank', type: 'asset', normalBalance: 'debit', rgsCode: 'BLIM.10' },
  { code: '1200', name: 'Debiteuren', type: 'asset', normalBalance: 'debit', rgsCode: 'BVOR.11' },
  { code: '1400', name: 'Voorraad', type: 'asset', normalBalance: 'debit', rgsCode: 'BVRD.30' },
  { code: '1600', name: 'Overige vorderingen', type: 'asset', normalBalance: 'debit', rgsCode: 'BVOR.11' },
  { code: '1700', name: 'Vooruitbetaalde kosten', type: 'asset', normalBalance: 'debit', rgsCode: 'BVOR.11' },
  { code: '1800', name: 'Materiële vaste activa', type: 'asset', normalBalance: 'debit', rgsCode: 'BMVA.02' },
  { code: '1850', name: 'Vervoermiddelen', type: 'asset', normalBalance: 'debit', rgsCode: 'BMVA.02' },
  // Passiva
  { code: '2000', name: 'Crediteuren', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
  { code: '2100', name: 'Overige schulden', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
  { code: '2300', name: 'Vooruitontvangen bedragen', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
  { code: '2400', name: 'Nog te betalen kosten', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
  { code: '2900', name: 'Rekening-courant', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
  // Eigen vermogen
  { code: '3000', name: 'Eigen vermogen', type: 'equity', normalBalance: 'credit', rgsCode: 'BEIV.05' },
  // Kosten
  { code: '4000', name: 'Inkoopwaarde', type: 'expense', normalBalance: 'debit', rgsCode: 'WKPR.70' },
  { code: '4100', name: 'Huisvestingskosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4200', name: 'Autokosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4300', name: 'Kantoor- en algemene kosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4310', name: 'Accountants- en administratiekosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4320', name: 'Verzekeringen', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4330', name: 'Telecommunicatie', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4340', name: 'Software en internetdiensten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  { code: '4400', name: 'Personeelskosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WPER.40' },
  { code: '4500', name: 'Financiële baten en lasten', type: 'expense', normalBalance: 'debit', rgsCode: 'WFBE.84' },
  { code: '4600', name: 'Afschrijvingen', type: 'expense', normalBalance: 'debit', rgsCode: 'WAFS.41' },
  { code: '4700', name: 'Overige bedrijfskosten', type: 'expense', normalBalance: 'debit', rgsCode: 'WBED.42' },
  // Opbrengsten
  { code: '8000', name: 'Omzet', type: 'income', normalBalance: 'credit', rgsCode: 'WOMZ.80' },
  { code: '8100', name: 'Overige opbrengsten', type: 'income', normalBalance: 'credit', rgsCode: 'WOVB.82' },
];

/** RGS hoofdgroep labels (official RGS nomenclature, Dutch). */
export const RGS_LABELS = {
  'BMVA.02': 'Materiële vaste activa',
  'BFVA.03': 'Financiële vaste activa',
  'BVRD.30': 'Voorraden',
  'BVOR.11': 'Vorderingen',
  'BLIM.10': 'Liquide middelen',
  'BEIV.05': 'Eigen vermogen',
  'BVRZ.07': 'Voorzieningen',
  'BLAS.08': 'Langlopende schulden',
  'BSCH.12': 'Kortlopende schulden',
  'WKPR.70': 'Inkoopwaarde van de omzet',
  'WPER.40': 'Personeelskosten',
  'WAFS.41': 'Afschrijvingen',
  'WBED.42': 'Overige bedrijfskosten',
  'WOMZ.80': 'Omzet',
  'WOVB.82': 'Overige bedrijfsopbrengsten',
  'WFBE.84': 'Financiële baten en lasten',
};

export function rgsLabel(code) {
  return RGS_LABELS[code] ?? code ?? 'Overig';
}
