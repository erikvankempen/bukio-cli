/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Belgium jurisdiction profile (Phase B — BE profile).
//
// Data sources: research brief at docs-research/be-profile.md — every
// account code from the PCN-BE minimum plan (plan comptable minimum
// normalisé / minimumrekeningenstelsel, AR 12-09-1983), VAT facts from
// FPS Finance + PwC/meridian calendars, identifiers from KBO/BCE, e-invoice
// mandate from the European Commission + Peppol sources. FR labels primary,
// NL labels secondary (the plan's bilingual reality).
//
// Key corrections the research made to initial assumptions (why research
// comes first): Peppol scheme 0208 = KBO (0106 is the DUTCH KvK); the
// franchise scheme took effect 1 Jan 2025 (EU directive 2020/285, art.
// 56bis WBTW), not 2024; the result closes to 140/141 (overgedragen
// winst/verlies), NOT 12x (12 = revaluation surpluses); PCMN codes are
// 3-4 digit, not 6.
//
// Phase B scope discipline (same contract as LU/GB/FR/US): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (Intervat return
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (NBB
//                            schema verkort / full layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Belgian SAF-T;
//                            CODA statement import is a B-milestone, not
//                            registered in bankStatementFormats)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (WBTW art. 56bis exemption-wording + reg. 14
//                            style rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (CONFIRMED: mandatory B2B
// e-invoicing via Peppol since 1 Jan 2026, Peppol BIS 3.0 = EN 16931 UBL
// 2.1 — exactly what the UBL builder emits; scheme 0208 KBO), SEPA,
// CAMT.053, ECB, closing 140 -> 140.

export default {
  meta: {
    country: 'BE',
    baseCurrency: 'EUR',
    locale: 'nl-BE',
    // post-2019 WVV/CSA forms (research §4)
    legalForms: ['bv', 'nv', 'vzw', 'cv', 'eenmanszaak'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // KBO/BCE enterprise number, 10 digits (0xxx.xxx.xxx)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^BE\d{10}$/i, // BE + 10 digits (BE0 or BE1 prefixes both valid)
    peppolSchemeId: '0208', // KBO/BCE enterprise number (NOT 0106 — that is NL KvK)
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2100,
    // franchise scheme (art. 56bis WBTW, EU dir. 2020/285): effective
    // 1 Jan 2025, turnover ≤ €25,000 excl. VAT (10% tolerance)
    smallBusinessScheme: 'franchise',
    // BE rates 2026 (research §1): 21% standard / 12% + 6% reduced /
    // 0% exceptional. EU reverse charge applies (RE).
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% taux normal' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12% taux réduit' },
      { code: '6', rateBp: 600, type: 'standard', euReverse: 0, description: '6% taux réduit' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% taux zéro' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exonéré' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Autoliquidation (nationale)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Autoliquidation (UE)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marge' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Usage privé' },
    ],
    accounts: {
      // PCMN VAT accounts (official, research §6): 411 TVA à récupérer
      // (input, asset) / 451 TVA à payer (output, liability); the Intervat
      // return nets input vs output — no dedicated settlement account
      ledger: [
        { code: '411', name: 'TVA à récupérer — Te recupereren BTW', type: 'asset', normalBalance: 'debit' },
        { code: '451', name: 'TVA à payer — Te betalen BTW', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '451',
      // the minimum plan designates no rounding-difference account —
      // 648 (Charges d'exploitation diverses) is the per-firm convention
      // (brief, flagged unverified); 648 is seeded in the chart so
      // `vat settle` posts to a real account
      differenceDefault: '648',
      // 451 is BOTH the output clearing account (ledger) and the settlement
      // position (fileDefault): settle a filed balance before new output VAT
      // accrues on the same account — the NL design keeps these separate
      // (output clearing 2500 vs settlement 2510)
      settlementAccountName: 'TVA à payer',
    },
    // returnLayout omitted — the Intervat return engine is a B-milestone
    filingPeriodicity: 'monthly', // default; quarterly option for turnover ≤ €2.5M
    reverseChargeEffectiveRateBp: 2100,
  },

  reporting: {
    // format omitted — NBB schema verkort layout is a B-milestone
    taxonomy: null,
    // PCN-BE minimum plan (AR 12-09-1983), research §5 — FR primary /
    // NL secondary labels, 3-4 digit codes as in the plan
    defaultChart: [
      { code: '100', name: 'Capital souscrit — Geplaatst kapitaal', type: 'equity', normalBalance: 'credit' },
      { code: '130', name: 'Réserve légale — Wettelijke reserve', type: 'equity', normalBalance: 'credit' },
      { code: '133', name: 'Réserves disponibles — Beschikbare reserves', type: 'equity', normalBalance: 'credit' },
      { code: '140', name: 'Bénéfice reporté — Overgedragen winst', type: 'equity', normalBalance: 'credit' },
      { code: '141', name: 'Perte reportée — Overgedragen verlies', type: 'equity', normalBalance: 'debit' },
      { code: '160', name: 'Provisions pour risques et charges — Voorzieningen voor risico\'s en kosten', type: 'liability', normalBalance: 'credit' },
      { code: '172', name: 'Dettes de location-financement — Leasingschulden en soortgelijke', type: 'liability', normalBalance: 'credit' },
      { code: '173', name: 'Etablissements de crédit (>1 an) — Kredietinstellingen (>1 jaar)', type: 'liability', normalBalance: 'credit' },
      { code: '200', name: 'Frais d\'établissement — Oprichtingskosten', type: 'asset', normalBalance: 'debit' },
      { code: '211', name: 'Concessions, brevets, licences — Concessies, octrooien, licenties', type: 'asset', normalBalance: 'debit' },
      { code: '221', name: 'Constructions — Gebouwen', type: 'asset', normalBalance: 'debit' },
      { code: '230', name: 'Installations — Installaties', type: 'asset', normalBalance: 'debit' },
      { code: '231', name: 'Machines — Machines', type: 'asset', normalBalance: 'debit' },
      { code: '240', name: 'Mobilier, matériel de bureau et informatique — Meubilair, kantooruitrusting en informatica', type: 'asset', normalBalance: 'debit' },
      { code: '241', name: 'Matériel roulant — Rollend materieel', type: 'asset', normalBalance: 'debit' },
      { code: '280', name: 'Immobilisations financières — Financiële vaste activa', type: 'asset', normalBalance: 'debit' },
      { code: '300', name: 'Matières premières — Grond- en hulpstoffen', type: 'asset', normalBalance: 'debit' },
      { code: '340', name: 'Marchandises — Handelsgoederen', type: 'asset', normalBalance: 'debit' },
      { code: '400', name: 'Clients — Handelsdebiteuren', type: 'asset', normalBalance: 'debit' },
      { code: '407', name: 'Créances douteuses — Dubieuze debiteuren', type: 'asset', normalBalance: 'debit' },
      { code: '411', name: 'TVA à récupérer — Te recupereren BTW', type: 'asset', normalBalance: 'debit' },
      { code: '412', name: 'Impôts et précomptes à récupérer — Te recupereren belastingen en voorheffingen', type: 'asset', normalBalance: 'debit' },
      { code: '416', name: 'Créances diverses — Diverse vorderingen', type: 'asset', normalBalance: 'debit' },
      { code: '490', name: 'Charges à reporter — Over te dragen kosten', type: 'asset', normalBalance: 'debit' },
      { code: '422', name: 'Dettes à plus d\'un an échéant dans l\'année — Schulden >1 jaar vervallend binnen het jaar', type: 'liability', normalBalance: 'credit' },
      { code: '433', name: 'Etablissements de crédit – dettes en compte courant — Kredietinstellingen – schulden in rekening-courant', type: 'liability', normalBalance: 'credit' },
      { code: '440', name: 'Fournisseurs — Leveranciers', type: 'liability', normalBalance: 'credit' },
      { code: '444', name: 'Factures à recevoir — Te ontvangen facturen', type: 'liability', normalBalance: 'credit' },
      { code: '451', name: 'TVA à payer — Te betalen BTW', type: 'liability', normalBalance: 'credit' },
      { code: '452', name: 'Impôts et taxes à payer — Te betalen belastingen en taksen', type: 'liability', normalBalance: 'credit' },
      { code: '454', name: 'ONSS — RSZ', type: 'liability', normalBalance: 'credit' },
      { code: '455', name: 'Rémunérations — Bezoldigingen', type: 'liability', normalBalance: 'credit' },
      { code: '492', name: 'Charges à imputer — Toe te rekenen kosten', type: 'liability', normalBalance: 'credit' },
      { code: '493', name: 'Produits à reporter — Over te dragen opbrengsten', type: 'liability', normalBalance: 'credit' },
      { code: '550', name: 'Banques – comptes courants — Kredietinstellingen – zichtrekeningen', type: 'asset', normalBalance: 'debit' },
      { code: '570', name: 'Caisses-espèces — Kas', type: 'asset', normalBalance: 'debit' },
      { code: '600', name: 'Achats de matières premières — Aankopen van grondstoffen', type: 'expense', normalBalance: 'debit' },
      { code: '604', name: 'Achats de marchandises — Aankopen van handelsgoederen', type: 'expense', normalBalance: 'debit' },
      { code: '620', name: 'Rémunérations — Bezoldigingen', type: 'expense', normalBalance: 'debit' },
      { code: '621', name: 'Cotisations patronales d\'assurances sociales — Werkgeversbijdragen sociale verzekeringen', type: 'expense', normalBalance: 'debit' },
      { code: '630', name: 'Dotations aux amortissements — Afschrijvingen', type: 'expense', normalBalance: 'debit' },
      { code: '648', name: 'Charges d\'exploitation diverses — Diverse exploitatiekosten', type: 'expense', normalBalance: 'debit' },
      { code: '650', name: 'Charges des dettes — Kosten van schulden', type: 'expense', normalBalance: 'debit' },
      { code: '700', name: 'Ventes et prestations de services — Verkopen en dienstverleningen', type: 'income', normalBalance: 'credit' },
      { code: '74', name: 'Autres produits d\'exploitation — Andere bedrijfsopbrengsten', type: 'income', normalBalance: 'credit' },
      { code: '75', name: 'Produits financiers — Financiële opbrengsten', type: 'income', normalBalance: 'credit' },
    ],
    // debtors account for invoice postings (PCMN 400 Clients)
    debtorsAccount: '400',
    bankAccountDefault: '550',
    inferTaxonomy: null,
    // statutoryAccounts omitted — NBB layout is a B-milestone
  },

  compliance: {
    // research §10: VAT monthly (20th of the following month) or quarterly
    // (25th, ≤ €2.5M turnover); annual accounts filed with the NBB within
    // 7 months of FYE; the annual client listing (31 March) is a filing
    // obligation but not a period return — documented, not calendarised
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'be-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'be-7-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'], // CODA is a B-milestone, not registered
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // BE is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (WBTW additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // CONFIRMED: BE mandates B2B e-invoicing
    // via Peppol since 1 Jan 2026; Peppol BIS 3.0 = EN 16931 UBL 2.1
    // auditFile omitted — no Belgian SAF-T
    languages: ['nl', 'fr', 'en'],
    defaultLanguage: 'nl',
  },

  // The BE minimum plan has no separate current-year result account: the
  // result closes straight into overgedragen winst (140). The generic
  // year-end engine always posts the appropriation on resultAccount 140
  // (a loss as a debit on 140; 141 is never referenced), and with
  // resultAccount == equityAccount the +result/-result legs cancel on the
  // same account (a harmless net-zero no-op) — the result lands in 140 via
  // the close itself.
  closing: { resultAccount: '140', equityAccount: '140' },
};
