/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Netherlands jurisdiction profile — the REFERENCE profile.
//
// Every value below is moved VERBATIM from the previously hardcoded module
// constants (src/vat/index.js, src/core/chart.js, src/cli/init.js,
// src/report/jaarrekening.js, src/compliance/index.js, src/invoice/ubl.js,
// src/year-end/index.js). "Moved, not changed": NL behavior must stay
// byte-identical. Consumers resolve this profile via the registry
// (src/jurisdictions/index.js) — never import it directly.

export default {
  meta: {
    country: 'NL',
    baseCurrency: 'EUR',
    locale: 'nl',
    // legal forms per src/cli/init.js LEGAL_FORMS
    legalForms: ['eenmanszaak', 'vof', 'bv', 'nv', 'stichting', 'vereniging'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    // generic company columns (migration 021 renames kvk → registration_id,
    // btw_id → tax_id); contacts keep their own kvk column in Phase A
    companyIdLabel: 'registration_id',
    vatIdLabel: 'tax_id',
    vatIdFormat: /^NL\d{9}B\d{2}$/i, // advisory validation when set
    peppolSchemeId: '9944', // KVK registry code (Peppol BIS 3.0 BT-34/BT-49)
    accountNumber: { kind: 'iban' }, // iban | sort-code | aba
  },

  tax: {
    system: 'vat', // vat | sales-tax | none
    standardRateBp: 2100,
    smallBusinessScheme: 'kor', // kor | flat-rate | franchise | null
    // current VAT_CODES verbatim (src/vat/index.js)
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% hoog tarief' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% laag tarief' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% nultarief' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Vrijgesteld' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Verlegd (binnenland)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Verlegd (EU)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marge' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privégebruik' },
    ],
    accounts: {
      // current VAT_ACCOUNTS verbatim (rgsCode → taxonomyCode per decision §9.1.1)
      ledger: [
        { code: '1500', name: 'Te vorderen omzetbelasting', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BVOR.11' },
        { code: '2500', name: 'Te betalen omzetbelasting', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      ],
      fileDefault: '2510',
      differenceDefault: '4700',
      settlementAccountName: 'Af te dragen omzetbelasting',
    },
    returnLayout: 'ob-1a-5d', // declarative layout id; NL mapper keyed by this id
    filingPeriodicity: 'quarterly',
    rounding: 'nl-per-line-whole-euro',
    reverseChargeEffectiveRateBp: 2100, // replaces the hardcoded effectiveRateBp
  },

  reporting: {
    format: 'auto',
    taxonomy: 'rgs',
    // current RGS_LABELS verbatim (src/core/chart.js)
    labels: {
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
    },
    // current DEFAULT_CHART verbatim (src/core/chart.js); rgsCode → taxonomyCode
    defaultChart: [
      { code: '1000', name: 'Kas', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BLIM.10' },
      { code: '1100', name: 'Bank', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BLIM.10' },
      { code: '1200', name: 'Debiteuren', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BVOR.11' },
      { code: '1400', name: 'Voorraad', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BVRD.30' },
      { code: '1600', name: 'Overige vorderingen', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BVOR.11' },
      { code: '1700', name: 'Vooruitbetaalde kosten', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BVOR.11' },
      { code: '1800', name: 'Materiële vaste activa', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BMVA.02' },
      { code: '1850', name: 'Vervoermiddelen', type: 'asset', normalBalance: 'debit', taxonomyCode: 'BMVA.02' },
      { code: '2000', name: 'Crediteuren', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      { code: '2100', name: 'Overige schulden', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      { code: '2300', name: 'Vooruitontvangen bedragen', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      { code: '2400', name: 'Nog te betalen kosten', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      { code: '2900', name: 'Rekening-courant', type: 'liability', normalBalance: 'credit', taxonomyCode: 'BSCH.12' },
      { code: '3000', name: 'Eigen vermogen', type: 'equity', normalBalance: 'credit', taxonomyCode: 'BEIV.05' },
      { code: '4000', name: 'Inkoopwaarde', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WKPR.70' },
      { code: '4100', name: 'Huisvestingskosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4200', name: 'Autokosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4300', name: 'Kantoor- en algemene kosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4310', name: 'Accountants- en administratiekosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4320', name: 'Verzekeringen', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4330', name: 'Telecommunicatie', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4340', name: 'Software en internetdiensten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4400', name: 'Personeelskosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WPER.40' },
      { code: '4500', name: 'Financiële baten en lasten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WFBE.84' },
      { code: '4600', name: 'Afschrijvingen', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WAFS.41' },
      { code: '4700', name: 'Overige bedrijfskosten', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WBED.42' },
      { code: '4840', name: 'Koersverschillen', type: 'expense', normalBalance: 'debit', taxonomyCode: 'WFBE.84' },
      { code: '8000', name: 'Omzet', type: 'income', normalBalance: 'credit', taxonomyCode: 'WOMZ.80' },
      { code: '8100', name: 'Overige opbrengsten', type: 'income', normalBalance: 'credit', taxonomyCode: 'WOVB.82' },
    ],
    // additive (Phase B6): debtors account for invoice postings — was
    // hardcoded '1200' in buildInvoicePostings; value unchanged for NL
    debtorsAccount: '1200',
    inferTaxonomy: null, // NL keyword inference stays in code keyed by taxonomy='rgs'
    // current jaarrekening line lists verbatim (src/report/jaarrekening.js)
    statutoryAccounts: {
      models: ['micro', 'klein'],
      lines: {
        activa: [
          { rgs: 'BMVA.02', label: 'Materiële vaste activa' },
          { rgs: 'BIVA.04', label: 'Immateriële vaste activa' },
          { rgs: 'BFVA.03', label: 'Financiële vaste activa' },
          { rgs: 'BVRD.30', label: 'Voorraden' },
          { rgs: 'BVOR.11', label: 'Vorderingen' },
          { rgs: 'BLIM.10', label: 'Liquide middelen' },
        ],
        passiva: [
          { rgs: 'BEIV.05', label: 'Eigen vermogen' },
          { rgs: 'BVRZ.07', label: 'Voorzieningen' },
          { rgs: 'BLAS.08', label: 'Langlopende schulden' },
          { rgs: 'BSCH.12', label: 'Kortlopende schulden' },
        ],
        pnl: [
          { rgs: 'WOMZ.80', label: 'Netto-omzet' },
          { rgs: 'WOVB.82', label: 'Overige bedrijfsopbrengsten' },
          { rgs: 'WKPR.70', label: 'Inkoopwaarde van de omzet' },
          { rgs: 'WBED.42', label: 'Overige bedrijfskosten' },
          { rgs: 'WAFS.41', label: 'Afschrijvingen' },
          { rgs: 'WFBE.84', label: 'Financiële baten en lasten' },
          { rgs: 'WBEL.60', label: 'Belastingen' },
        ],
      },
    },
  },

  compliance: {
    // replaces the hardcoded OB/ICP/JAARREKENING trio (src/compliance/index.js)
    filingTypes: [
      { type: 'OB', periodShape: 'YYYY-Qn', deadlineRule: 'nl-quarterly' },
      { type: 'ICP', periodShape: 'YYYY-Qn', deadlineRule: 'nl-quarterly' },
      { type: 'JAARREKENING', periodShape: 'YYYY', deadlineRule: 'nl-13-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'],
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'nl-12-vereisten', // rule-set id (art. 35c/35d Wet OB + KVK)
    eInvoicing: 'peppol-bis-3.0',
    auditFile: 'xaf-auditfile-4.0',
    languages: ['nl', 'en'],
    defaultLanguage: 'nl',
  },

  closing: { resultAccount: '9900', equityAccount: '3000' },
};
