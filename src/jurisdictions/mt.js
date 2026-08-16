/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Malta jurisdiction profile (Phase E — MT profile).
//
// Data sources: research brief at docs-research/mt-profile.md — VAT 18% +
// reduced 12/7/5/0 (four reduced bands — do NOT assume 18/7/5/0 alone),
// VAT number MT + 8 digits, company number from MBR, small undertaking
// exemption €35,000 uniform (Art. 11 VAT Act), Peppol EAS 9943 (official
// EAS codelist, release 8 Dec 2025), quarterly VAT return by the 15th/22nd
// of the second month after the quarter (≈45 days), annual accounts with
// MBR within 10 months of FYE, CIT form C on self-assessment 9 months
// after FYE, convention-based international chart in English (no statutory
// chart).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (myTax VAT return
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (MBR
//                            annual accounts are a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Maltese SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Maltese additions
//                            are a B-milestone
//   - national e-invoicing  no domestic B2B mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (MT is a Peppol
//                            participant, EAS 9943)
// Registered: SEPA, CAMT.053, ECB, closing 3300 -> 3200.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), natural fit for MT (English-language books).

export default {
  meta: {
    country: 'MT',
    baseCurrency: 'EUR',
    locale: 'mt',
    legalForms: ['ltd', 'plc', 'partnership', 'sole-trader'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // MBR company number (classic C-prefix or numeric)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^MT\d{8}$/i, // MT + 8 digits
    // 9943 = Malta VAT number (EAS codelist)
    peppolSchemeId: '9943',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 1800,
    // Small undertaking exemption (Art. 11 VAT Act): €35,000 uniform from
    // 1 Jan 2025 (cross-border Art. 11A: €100,000 Union turnover)
    smallBusinessScheme: 'small-undertaking',
    codes: [
      { code: '18', rateBp: 1800, type: 'standard', euReverse: 0, description: '18% VAT' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12% reduced (securities custody, credit mgmt, pleasure-boat hire, body-care)' },
      { code: '7', rateBp: 700, type: 'standard', euReverse: 0, description: '7% reduced (licensed tourist accommodation, sporting facilities)' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% reduced (electricity, confectionery, books, medical devices)' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% exports / intra-EU B2B' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exempt supplies' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Reverse charge' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-Community acquisition' },
    ],
    accounts: {
      // Convention chart (English): 2410 VAT input (asset) / 2420 VAT
      // output (liability); the settlement account 2430 absorbs the return
      // balance + rounding differences
      ledger: [
        { code: '2410', name: 'VAT input (on purchases)', type: 'asset', normalBalance: 'debit' },
        { code: '2420', name: 'VAT output (on sales)', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2430',
      differenceDefault: '2430',
      settlementAccountName: 'VAT settlement (net payable/recoverable)',
    },
    // returnLayout omitted — the myTax VAT return engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly cycles exist for some traders
    reverseChargeEffectiveRateBp: 1800,
  },

  reporting: {
    // format omitted — MBR annual accounts layout is a B-milestone
    taxonomy: null,
    // Convention-based international chart (IFRS practice, English books —
    // no statutory chart)
    defaultChart: [
      { code: '1000', name: 'Bank — current account', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Trade debtors', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Other receivables', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Prepayments', type: 'asset', normalBalance: 'debit' },
      { code: '1400', name: 'Inventory', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Fixed assets', type: 'asset', normalBalance: 'debit' },
      { code: '1510', name: 'Accumulated depreciation', type: 'asset', normalBalance: 'credit' },
      { code: '2410', name: 'VAT input (on purchases)', type: 'asset', normalBalance: 'debit' },
      { code: '2000', name: 'Trade creditors', type: 'liability', normalBalance: 'credit' },
      { code: '2100', name: 'Other payables', type: 'liability', normalBalance: 'credit' },
      { code: '2200', name: 'Accruals', type: 'liability', normalBalance: 'credit' },
      { code: '2300', name: 'Employee taxes payable (PAYE/SS)', type: 'liability', normalBalance: 'credit' },
      { code: '2420', name: 'VAT output (on sales)', type: 'liability', normalBalance: 'credit' },
      { code: '2430', name: 'VAT settlement', type: 'liability', normalBalance: 'credit' },
      { code: '2500', name: 'Loans (long-term)', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Share capital', type: 'equity', normalBalance: 'credit' },
      { code: '3100', name: 'Share premium', type: 'equity', normalBalance: 'credit' },
      { code: '3200', name: 'Retained earnings', type: 'equity', normalBalance: 'credit' },
      { code: '3300', name: 'Profit/(loss) for the year', type: 'equity', normalBalance: 'credit' },
      // 4000 first: postingDefaults picks the first income account
      { code: '4000', name: 'Sales revenue', type: 'income', normalBalance: 'credit' },
      { code: '4100', name: 'Other income', type: 'income', normalBalance: 'credit' },
      { code: '4200', name: 'Purchases / cost of sales', type: 'expense', normalBalance: 'debit' },
      { code: '5000', name: 'Wages and salaries', type: 'expense', normalBalance: 'debit' },
      { code: '5100', name: 'Rent and rates', type: 'expense', normalBalance: 'debit' },
      { code: '5200', name: 'Utilities', type: 'expense', normalBalance: 'debit' },
      { code: '5300', name: 'Professional fees', type: 'expense', normalBalance: 'debit' },
      { code: '5400', name: 'Repairs and maintenance', type: 'expense', normalBalance: 'debit' },
      { code: '5500', name: 'Depreciation', type: 'expense', normalBalance: 'debit' },
      { code: '5600', name: 'Other operating expenses', type: 'expense', normalBalance: 'debit' },
      { code: '5700', name: 'Finance costs', type: 'expense', normalBalance: 'debit' },
      { code: '5800', name: 'Tax expense (CIT)', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1100',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — MBR layout is a B-milestone
  },

  compliance: {
    // Quarterly VAT return e-filed via myTax, due the 15th (22nd online)
    // of the SECOND month after the quarter (Q1 -> 15/22 May); annual
    // accounts with MBR within 10 months of FYE (31 Oct); CIT form C on
    // self-assessment 9 months after FYE (30 Sep).
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Qn', deadlineRule: 'mt-vat-quarterly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'mt-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'mt-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Malta is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (MT additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9943); no
    // domestic B2B mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Maltese SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '3300', equityAccount: '3200' },
};