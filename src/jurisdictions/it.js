/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across sixteen jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Italy jurisdiction profile (Phase D — IT profile).
//
// Data sources: research brief at docs-research/it-profile.md — IVA rates
// from the 2026 VAT guides (22/10/5/4), liquidazione IVA deadlines from the
// Agenzia Entrate scadenzario (16th of the second month after the quarter
// for quarterly settlements; 16th of the following month for monthly),
// Dichiarazione IVA 30 April, bilancio deposit ~5 months after FYE
// (approval within 120 days + deposit within 30 days, art. 2364 c.c.),
// Partita IVA format, Peppol EAS codelist (0211 = Partita IVA).
//
// Key notes: Italy has NO statutory chart of accounts — the default chart
// follows the common commercialisti convention (1xxx liquidità/crediti,
// 2xxx debiti, 3xxx patrimonio netto, 4xxx ricavi, 5xxx costi) with the
// standard Italian account names (Cassa contanti, Banca c/c, Crediti
// v/clienti, Debiti v/fornitori, IVA a credito, IVA a debito, Erario c/IVA).
// The CNDCEC recommended scheme is a proposal, not a standard — documented
// as a convention chart (same treatment as GB/IE).
//
// Phase D scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (the liquidazione
//                            IVA engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (bilancio,
//                            civil-code layout, is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Italian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is
//                            registered ('eu-invoice-vereisten'); DPR 633/72
//                            additions are a B-milestone
//   - FatturaPA/SdI         domestic e-invoicing is a separate XML schema
//                            (fatturapa_v1.2), NOT Peppol UBL — the existing
//                            builder does not emit it, so the SdI export is
//                            a documented B-milestone; 'peppol-bis-3.0' is
//                            registered for CROSS-BORDER invoices (Italy is
//                            a Peppol participant, EAS 0211)
// Registered: SEPA, CAMT.053, ECB, closing 3200 -> 3100.

export default {
  meta: {
    country: 'IT',
    baseCurrency: 'EUR',
    locale: 'it',
    legalForms: ['srl', 'srl-semplificata', 'spa', 'snc', 'sas', 'ditta-individuale'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // REA number (Registro Imprese / Camera di Commercio)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^IT\d{11}$/i, // Partita IVA: IT + 11 digits
    // 0211 = Partita IVA (EAS; 0210 is the individual Codice Fiscale)
    peppolSchemeId: '0211',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2200,
    // Regime forfettario: flat-rate regime for turnover ≤ €85,000
    smallBusinessScheme: 'forfettario',
    // IT rates (DPR 633/72): 22% standard / 10% / 5% / 4%. The domestic
    // reverse-charge list and intra-Community acquisitions (§ art. 17/19)
    // apply. No general 0% category (exports are non-impugnabile/0-rated
    // via the V exempt code path).
    codes: [
      { code: '22', rateBp: 2200, type: 'standard', euReverse: 0, description: '22% aliquota ordinaria' },
      { code: '10', rateBp: 1000, type: 'standard', euReverse: 0, description: '10% aliquota ridotta' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% aliquota ridotta' },
      { code: '4', rateBp: 400, type: 'standard', euReverse: 0, description: '4% aliquota minima' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Esente / non imponibile (art. 8-10 DPR 633/72)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Reverse charge (art. 17)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Acquisto intracomunitario (art. 38-40)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Regime del margine' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Uso privato / autoconsumo' },
    ],
    accounts: {
      // convention chart: 1300 IVA a credito (input, asset) / 2100 IVA a
      // debito (output, liability); the settlement account 2400 Erario
      // c/IVA absorbs the liquidazione balance + rounding differences
      ledger: [
        { code: '1300', name: 'IVA a credito', type: 'asset', normalBalance: 'debit' },
        { code: '2100', name: 'IVA a debito', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2400',
      differenceDefault: '2400', // liquidazione balance/rounding lands on Erario c/IVA
      settlementAccountName: 'Erario c/IVA (liquidazione)',
    },
    // returnLayout omitted — the liquidazione IVA engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly when prior-year turnover > €400K
    reverseChargeEffectiveRateBp: 2200,
  },

  reporting: {
    // format omitted — bilancio (civil-code layout) is a B-milestone
    taxonomy: null,
    // commercialisti convention chart (no statutory chart in Italy) —
    // 1xxx liquidità/crediti, 2xxx debiti, 3xxx patrimonio netto,
    // 4xxx ricavi, 5xxx costi
    defaultChart: [
      { code: '1100', name: 'Banca c/c', type: 'asset', normalBalance: 'debit' },
      { code: '1000', name: 'Cassa contanti', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Crediti v/clienti', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'IVA a credito', type: 'asset', normalBalance: 'debit' },
      { code: '1400', name: 'Altri crediti', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Immobilizzazioni materiali', type: 'asset', normalBalance: 'debit' },
      { code: '1600', name: 'Fondo ammortamento immobilizzazioni', type: 'asset', normalBalance: 'credit' },
      { code: '2000', name: 'Debiti v/fornitori', type: 'liability', normalBalance: 'credit' },
      { code: '2100', name: 'IVA a debito', type: 'liability', normalBalance: 'credit' },
      { code: '2200', name: 'Debiti v/istituti previdenziali (INPS)', type: 'liability', normalBalance: 'credit' },
      { code: '2300', name: 'Altri debiti', type: 'liability', normalBalance: 'credit' },
      { code: '2400', name: 'Erario c/IVA', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Capitale sociale', type: 'equity', normalBalance: 'credit' },
      { code: '3100', name: 'Utili (perdite) a nuovo', type: 'equity', normalBalance: 'credit' },
      { code: '3200', name: 'Utile (perdita) dell\'esercizio', type: 'equity', normalBalance: 'credit' },
      // 4000 first: postingDefaults picks the first income account — the
      // standard sales account (not a 4% minima one)
      { code: '4000', name: 'Ricavi delle vendite e delle prestazioni', type: 'income', normalBalance: 'credit' },
      { code: '4100', name: 'Altri ricavi', type: 'income', normalBalance: 'credit' },
      { code: '5000', name: 'Acquisti di materie prime e merci', type: 'expense', normalBalance: 'debit' },
      { code: '5100', name: 'Costi per servizi', type: 'expense', normalBalance: 'debit' },
      { code: '5200', name: 'Costi per il personale', type: 'expense', normalBalance: 'debit' },
      { code: '5300', name: 'Oneri diversi di gestione', type: 'expense', normalBalance: 'debit' },
      { code: '5400', name: 'Ammortamenti', type: 'expense', normalBalance: 'debit' },
      { code: '5500', name: 'Oneri finanziari', type: 'expense', normalBalance: 'debit' },
      { code: '5600', name: 'Imposte sul reddito (IRES/IRAP)', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (Crediti v/clienti)
    debtorsAccount: '1200',
    bankAccountDefault: '1100',
    inferTaxonomy: null,
    // statutoryAccounts omitted — bilancio layout is a B-milestone
  },

  compliance: {
    // Liquidazione IVA: quarterly versamento by the 16th of the SECOND month
    // after the quarter (Q1 -> 16 May; +1% interest vs monthly); monthly
    // when prior-year turnover > €400K (16th of the following month). The
    // annual Dichiarazione IVA is due 30 April of the following year. The
    // bilancio is approved within 120 days of FYE and deposited at the
    // Registro Imprese within 30 days of approval (~5 months after FYE).
    filingTypes: [
      { type: 'LIQUIDAZIONE_IVA', periodShape: 'YYYY-Qn', deadlineRule: 'it-liquidazione-quarterly' },
      { type: 'DICHIARAZIONE_IVA', periodShape: 'YYYY', deadlineRule: 'it-dichiarazione-iva' },
      { type: 'BILANCIO', periodShape: 'YYYY', deadlineRule: 'it-bilancio' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Italy is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (DPR 633/72 additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 0211); domestic
    // e-invoicing is FatturaPA via SdI — a separate XML schema, B-milestone
    // (the existing UBL builder does NOT emit fatturapa_v1.2)
    // auditFile omitted — no Italian SAF-T
    languages: ['it', 'en'],
    defaultLanguage: 'it',
  },

  closing: { resultAccount: '3200', equityAccount: '3100' },
};
