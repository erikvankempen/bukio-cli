/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Kosovo jurisdiction profile (XK, Phase G — 31st market, 16 Aug 2026).
// Research: docs-research/xk-profile.md. NOT an EU member, NOT a Peppol
// participant — no EAS scheme; cross-border EN 16931 UBL emission only.
// Documents render in Albanian (sq) — full i18n table since 16 Aug 2026.
export default {
  meta: {
    country: 'XK',
    name: 'Kosovo',
    baseCurrency: 'EUR', // unilateral euro adoption (2002; no local central bank)
    locale: 'sq',
    legalForms: ['shpk', 'sha', 'op', 'kp', 'bi'],
    defaultFiscalYearEnd: '12-31',
  },
  identifiers: {
    accountNumber: { kind: 'iban' }, // Kosovo uses IBAN (XKxx format)
    companyIdLabel: 'NBR',
    companyIdFormat: /^\d{8}$/,
    vatIdLabel: 'Numri i TVSH-së',
    vatIdFormat: /^K\d{8,10}$/,
    peppolSchemeId: null, // not a Peppol participant
  },
  tax: {
    system: 'vat',
    codes: [
      { code: '18', rateBp: 1800, type: 'standard', euReverse: 0, description: '18% tatim mbi vlerën e shtuar' },
      { code: '8', rateBp: 800, type: 'reduced', euReverse: 0, description: '8% TVSH e reduktuar' },
      { code: '0', rateBp: 0, type: 'zero', euReverse: 0, description: '0% TVSH' },
      { code: 'V', rateBp: 0, type: 'outside', euReverse: 0, description: 'jashtë fushëveprimit' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 1, description: 'marrësi paguan tatimin' },
      { code: 'RE', rateBp: 0, type: 'exempt', euReverse: 0, description: 'i përjashtuar' },
    ],
    allTaxCodes: ['18', '8', '0', 'V', 'R', 'RE'],
    accounts: {
      ledger: [
        { code: '2210', name: 'TVSH e hyrshme', type: 'asset', normalBalance: 'debit' },
        { code: '2220', name: 'TVSH e dalëshme', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2230',
      differenceDefault: '2230',
    },
  },
  reporting: {
    format: 'convention',
    chart: 'SKRFI convention (Albanian)',
    defaultChart: [
      { code: '1010', name: 'Arka', type: 'asset', normalBalance: 'debit' },
      { code: '1020', name: 'Banka', type: 'asset', normalBalance: 'debit' },
      { code: '2010', name: 'Klientët', type: 'asset', normalBalance: 'debit' },
      { code: '2020', name: 'Furnitorët', type: 'liability', normalBalance: 'credit' },
      { code: '2210', name: 'TVSH e hyrshme', type: 'asset', normalBalance: 'debit' },
      { code: '2220', name: 'TVSH e dalëshme', type: 'liability', normalBalance: 'credit' },
      { code: '2230', name: 'Zgjidhja e TVSH-së', type: 'liability', normalBalance: 'credit' },
      { code: '3010', name: 'Kapitali', type: 'equity', normalBalance: 'credit' },
      { code: '3020', name: 'Rezultatet e pashpërndara', type: 'equity', normalBalance: 'credit' },
      { code: '3030', name: 'Rezultati i vitit', type: 'equity', normalBalance: 'credit' },
      { code: '4010', name: 'Të hyrat nga shitjet', type: 'income', normalBalance: 'credit' },
      { code: '4020', name: 'Të hyra të tjera', type: 'income', normalBalance: 'credit' },
      { code: '5010', name: 'Shpenzimet e mallrave', type: 'expense', normalBalance: 'debit' },
      { code: '5020', name: 'Shpenzimet e personelit', type: 'expense', normalBalance: 'debit' },
      { code: '5030', name: 'Shpenzimet operative', type: 'expense', normalBalance: 'debit' },
      { code: '5040', name: 'Amortizimi', type: 'expense', normalBalance: 'debit' },
      { code: '5050', name: 'Shpenzimet financiare', type: 'expense', normalBalance: 'debit' },
      { code: '5060', name: 'Tatimi mbi të ardhurat', type: 'expense', normalBalance: 'debit' },
      { code: '5070', name: 'Shpenzime të tjera', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '2010',
    bankAccountDefault: '1020',  },
  compliance: {
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'xk-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'xk-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'xk-cit' },
    ],
  },
  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // generic art. 226-style baseline (non-EU)
    eInvoicing: 'peppol-bis-3.0', // cross-border EN 16931 emission; XK has no national mandate
    languages: ['sq'],
    defaultLanguage: 'sq',
  },
  exchange: {
    bankStatementFormats: ['camt.053'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'],
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },
  closing: { resultAccount: '3030', equityAccount: '3020' },
};
