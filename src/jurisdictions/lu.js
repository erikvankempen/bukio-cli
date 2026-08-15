/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Luxembourg jurisdiction profile (Phase B, milestone B1 — profile data).
//
// Data sources: official PCN 2020 annex (RGD 12 Sept 2019, Legilux Mémorial A
// n°631), KPMG PCN guide, Guichet.lu, Odoo l10n_lu chart — see
// docs-research/lu-pcn-2020.md for the full source table and confidence
// flags per item.
//
// Phase B1 scope discipline: this profile registers ONLY what maps to
// existing generic engines. NL-specific builders are deliberately NOT
// registered — the strict dispatch (FORMAT_NOT_SUPPORTED) then fails loudly
// instead of silently producing Dutch output:
//   - tax.returnLayout            omitted → OB readout fails (LU eCDF returns
//                                  are a B-milestone)
// Registered since B2: reporting.format 'lu-lsc' + statutoryAccounts
// (LSC abridged layout).
// Registered since B3: documents.auditFile 'faia-2.01-reduced-b'.
// Registered since B6: documents.invoiceCompliance 'lu-invoice-vereisten'
// (loi TVA art. 66 + RCS rule set).
//   - compliance.filingTypes      [] → calendar shows nothing (turnover-band
//                                  TVA frequencies need YYYY-MM period shapes,
//                                  B5)
// Registered because the engine is jurisdiction-agnostic and correct for LU:
//   - documents.eInvoicing        'peppol-bis-3.0' (B2G mandatory in LU since
//                                  18 Mar 2023; scheme 0195 = RCS)
//   - exchange (SEPA, CAMT.053, ECB), closing accounts, VAT codes, identifiers

export default {
  meta: {
    country: 'LU',
    baseCurrency: 'EUR',
    locale: 'fr-lu',
    // legal forms per the LU research brief (memo §7.5); slugs follow the NL
    // lowercase convention. Sàrl-S = simplified SARL.
    legalForms: ['entreprise-individuelle', 'sarl', 'sarl-s', 'sa', 'sas', 'scs'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // RCS B-number (companies) / A-number (sole traders)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^LU\d{8}$/i, // TVA = LU + 8 digits (advisory validation when set)
    peppolSchemeId: '0195', // RCS registry code (Peppol BIS 3.0 BT-34/BT-49)
    accountNumber: { kind: 'iban' }, // LU IBAN: LU + 2 check + 3 bank code + 13 digits
  },

  tax: {
    system: 'vat',
    standardRateBp: 1700, // lowest standard rate in the EU
    smallBusinessScheme: 'franchise', // franchise en base: €50K turnover threshold (2025+)
    // LU VAT rates (AED): 17% normal / 14% intermediate / 8% reduced /
    // 3% super-reduced. Code strings follow the NL convention (rate-keyed).
    codes: [
      { code: '17', rateBp: 1700, type: 'standard', euReverse: 0, description: '17% taux normal' },
      { code: '14', rateBp: 1400, type: 'standard', euReverse: 0, description: '14% taux intermédiaire' },
      { code: '8', rateBp: 800, type: 'standard', euReverse: 0, description: '8% taux réduit' },
      { code: '3', rateBp: 300, type: 'standard', euReverse: 0, description: '3% taux super-réduit' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% taux zéro' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exonéré' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Auto-liquidation (nationale)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Auto-liquidation (UE)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marge' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Usage privé' },
    ],
    accounts: {
      // PCN 2020 classe 4 (S1 annex, verified): 421611 TVA en amont (input,
      // asset) / 461411 TVA en aval (output, liability). No RGS taxonomy —
      // taxonomyCode stays null until the PCN taxonomy engine (B-milestone).
      ledger: [
        { code: '421611', name: 'TVA en amont', type: 'asset', normalBalance: 'debit' },
        { code: '461411', name: 'TVA en aval', type: 'liability', normalBalance: 'credit' },
      ],
      // settlement accounts per LU practice: a credit balance → 461412 TVA à
      // payer; a debit balance → 421612 TVA à recevoir (research §6)
      fileDefault: '461412',
      // the PCN designates NO rounding-difference account (research §6, low
      // confidence): default to the umbrella 461418 TVA – Autres dettes; the
      // choice is a per-firm convention, documented not mandated
      differenceDefault: '461418',
      settlementAccountName: 'TVA à payer',
    },
    // returnLayout omitted — LU eCDF (CA3-style) return layout is a
    // B-milestone; OB readout fails loudly for LU until then
    filingPeriodicity: 'mixed', // annual (<€112K) / quarterly (€112K–€620K) / monthly (>€620K)
    reverseChargeEffectiveRateBp: 1700,
  },

  reporting: {
    format: 'lu-lsc', // B2: LSC abridged (abrégé) layout — the SME default
    taxonomy: 'pcn', // PCN 2020 taxonomy discriminator (account rows)
    // defaultChart per docs-research/lu-pcn-2020.md §9 — imputation (I)
    // accounts only, codes verbatim from the official annex (2–6 digits;
    // the "5-digit" claim is a simplification). French labels = the plan's
    // "fr labels" hook. taxonomyCode null until the PCN taxonomy engine.
    defaultChart: [
      { code: '101', name: 'Capital souscrit', type: 'equity', normalBalance: 'credit' },
      { code: '131', name: 'Réserve légale', type: 'equity', normalBalance: 'credit' },
      { code: '133', name: 'Réserves statutaires', type: 'equity', normalBalance: 'credit' },
      { code: '1381', name: 'Autres réserves disponibles', type: 'equity', normalBalance: 'credit' },
      { code: '1411', name: 'Résultats reportés en instance d\'affectation', type: 'equity', normalBalance: 'credit' },
      { code: '1412', name: 'Résultats reportés (affectés)', type: 'equity', normalBalance: 'credit' },
      { code: '142', name: 'Résultat de l\'exercice', type: 'equity', normalBalance: 'credit' },
      { code: '1941', name: 'Dettes envers établissements de crédit (≤ 1 an)', type: 'liability', normalBalance: 'credit' },
      { code: '4011', name: 'Clients', type: 'asset', normalBalance: 'debit' },
      { code: '421611', name: 'TVA en amont', type: 'asset', normalBalance: 'debit' },
      { code: '421612', name: 'TVA à recevoir', type: 'asset', normalBalance: 'debit' },
      { code: '44111', name: 'Fournisseurs', type: 'liability', normalBalance: 'credit' },
      { code: '44112', name: 'Fournisseurs – Factures non parvenues', type: 'liability', normalBalance: 'credit' },
      { code: '461411', name: 'TVA en aval', type: 'liability', normalBalance: 'credit' },
      { code: '461412', name: 'TVA à payer', type: 'liability', normalBalance: 'credit' },
      { code: '461418', name: 'TVA – Autres dettes', type: 'liability', normalBalance: 'credit' },
      { code: '4621', name: 'CCSS – dettes sécurité sociale', type: 'liability', normalBalance: 'credit' },
      { code: '4714', name: 'Dettes envers le personnel', type: 'liability', normalBalance: 'credit' },
      { code: '4712', name: 'Dettes envers associés et actionnaires', type: 'liability', normalBalance: 'credit' },
      { code: '481', name: 'Charges à reporter', type: 'asset', normalBalance: 'debit' },
      { code: '482', name: 'Produits à reporter', type: 'liability', normalBalance: 'credit' },
      { code: '5131', name: 'Banques et CCP : avoirs', type: 'asset', normalBalance: 'debit' },
      { code: '5132', name: 'Banques et CCP : découverts', type: 'liability', normalBalance: 'credit' },
      { code: '516', name: 'Caisse', type: 'asset', normalBalance: 'debit' },
      { code: '601', name: 'Achats de matières premières', type: 'expense', normalBalance: 'debit' },
      { code: '6061', name: 'Achats de marchandises', type: 'expense', normalBalance: 'debit' },
      { code: '61112', name: 'Loyers – Constructions / Bâtiments', type: 'expense', normalBalance: 'debit' },
      { code: '61342', name: 'Honoraires comptables, fiscaux, d\'audit', type: 'expense', normalBalance: 'debit' },
      { code: '62111', name: 'Salaires de base', type: 'expense', normalBalance: 'debit' },
      { code: '6232', name: 'Autres charges sociales', type: 'expense', normalBalance: 'debit' },
      { code: '6333', name: 'DCV sur autres installations, outillage et mobilier', type: 'expense', normalBalance: 'debit' },
      { code: '6462', name: 'TVA non récupérable', type: 'expense', normalBalance: 'debit' },
      { code: '6488', name: 'Charges d\'exploitation diverses', type: 'expense', normalBalance: 'debit' },
      { code: '65521', name: 'Intérêts sur comptes bancaires', type: 'expense', normalBalance: 'debit' },
      { code: '65582', name: 'Intérêts sur autres emprunts et dettes', type: 'expense', normalBalance: 'debit' },
      { code: '6711', name: 'IRC – exercice courant', type: 'expense', normalBalance: 'debit' },
      { code: '6721', name: 'ICC – exercice courant', type: 'expense', normalBalance: 'debit' },
      { code: '7021', name: 'Ventes de produits finis', type: 'income', normalBalance: 'credit' },
      { code: '7033', name: 'Prestations de services', type: 'income', normalBalance: 'credit' },
      { code: '7061', name: 'Ventes de marchandises', type: 'income', normalBalance: 'credit' },
      { code: '708', name: 'Autres éléments du chiffre d\'affaires', type: 'income', normalBalance: 'credit' },
      { code: '7488', name: 'Produits d\'exploitation divers', type: 'income', normalBalance: 'credit' },
    ],
    // debtors account for invoice postings (PCN 4011 Clients)
    debtorsAccount: '4011',
    bankAccountDefault: '5131',
    // LSC statutory layout (B2, abridged) — lines grouped by PCN class
    // PREFIXES per the official tableau de passage (docs-research/
    // lu-pcn-2020.md §8); side overlap resolved by the balans engine.
    // P&L signs: produits +1, charges -1.
    statutoryAccounts: {
      models: ['abrege'],
      lines: {
        activa: [
          { label: 'Capital souscrit non versé', prefixes: ['102', '103'] },
          { label: 'Frais d\'établissement', prefixes: ['20'] },
          { label: 'Actif immobilisé', prefixes: ['21', '22', '23', '24', '25'] },
          { label: 'Actif circulant', prefixes: ['3', '40', '41', '42', '50', '513', '516', '517', '518'] },
          { label: 'Comptes de régularisation', prefixes: ['481', '484', '486'] },
        ],
        passiva: [
          { label: 'Capitaux propres', prefixes: ['10', '11', '12', '13', '141', '142', '15', '16'] },
          { label: 'Provisions', prefixes: ['18'] },
          { label: 'Dettes', prefixes: ['192', '193', '194', '431', '432', '441', '442', '451', '452', '46', '47'] },
          { label: 'Comptes de régularisation', prefixes: ['482', '483', '485', '487'] },
        ],
        pnl: [
          { label: 'Chiffre d\'affaires net', prefixes: ['70'], sign: 1 },
          { label: 'Autres produits d\'exploitation', prefixes: ['71', '72', '73', '74'], sign: 1 },
          { label: 'Charges d\'exploitation', prefixes: ['60', '61', '62', '63', '64'], sign: -1 },
          { label: 'Produits financiers', prefixes: ['75'], sign: 1 },
          { label: 'Charges financières', prefixes: ['65'], sign: -1 },
          { label: 'Impôts sur le résultat', prefixes: ['67'], sign: -1 },
          { label: 'Autres impôts', prefixes: ['68'], sign: -1 },
        ],
      },
    },
    inferTaxonomy: null, // PCN keyword inference is a B-milestone
    // statutoryAccounts registered below (format 'lu-lsc' + models)
  },

  compliance: {
    // B5: TVA quarterly is the DEFAULT band (€112K–€620K turnover — the
    // middle band most SMEs sit in); the annual (<€112K, lu-annual) and
    // monthly (>€620K, lu-monthly, YYYY-MM shape) bands are registered in
    // the DEADLINE_RULES engine but need a per-company turnover band
    // selection (later refinement). The annual informative return
    // (1 May) is not calendarised yet.
    filingTypes: [
      { type: 'TVA', periodShape: 'YYYY-Qn', deadlineRule: 'lu-quarterly' },
      { type: 'COMPTES_ANNUELS', periodShape: 'YYYY', deadlineRule: 'lu-7-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'],
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'lu-invoice-vereisten', // B6: loi TVA art. 66 + RCS
    // (autorisation d'établissement has no schema field — documented only)
    eInvoicing: 'peppol-bis-3.0', // B2G mandatory since 18 Mar 2023; the UBL
    // builder is EU-generic and reads identifiers.peppolSchemeId (0195 = RCS)
    auditFile: 'faia-2.01-reduced-b', // B3: FAIA 2.01 reduced version B
    // (AED electronic audit file; the PCN-levied audit-file standard)
    languages: ['fr', 'en', 'de'],
    defaultLanguage: 'fr',
  },

  closing: { resultAccount: '142', equityAccount: '1411' },
};
