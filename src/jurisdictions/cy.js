/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Cyprus jurisdiction profile (Phase E — CY profile).
//
// Data sources: research brief at docs-research/cy-profile.md — VAT 19% +
// reduced 9/5/3/0 (3% super-reduced only since Jul 2023), VAT number CY +
// 8 digits + 1 letter, HE number from DRCOR, small undertakings exemption
// €15,600, Peppol EAS 9928 (official EAS codelist, release 8 Dec 2025),
// quarterly VAT 4 return by the 10th of the SECOND month after the quarter
// (monthly above €1M turnover), annual accounts 42 days after AGM
// (practically ~10-13 months), CIT TD4: from TY2026 due 31 January of the
// second year following the tax year (transitional 2026 dates for TY2023/24),
// convention-based international chart in English (no statutory chart).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (VAT 4 return
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (annual
//                            accounts layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Cypriot SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Cypriot additions
//                            are a B-milestone
//   - national e-invoicing  no domestic mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (CY is a Peppol
//                            participant, EAS 9928)
// Registered: SEPA, CAMT.053, ECB, closing 3300 -> 3200.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), natural fit for CY (English-language books).

export default {
  meta: {
    country: 'CY',
    baseCurrency: 'EUR',
    locale: 'cy',
    legalForms: ['ltd', 'plc', 'sole-trader', 'partnership', 'branch'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // HE number (Registrar of Companies)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^CY\d{8}[A-Z]$/i, // CY + 8 digits + 1 letter
    // 9928 = Cyprus VAT number (EAS codelist)
    peppolSchemeId: '9928',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 1900,
    // Small undertakings scheme: €15,600 exemption from registration/charging
    smallBusinessScheme: 'small-undertakings',
    codes: [
      { code: '19', rateBp: 1900, type: 'standard', euReverse: 0, description: '19% ΦΠΑ' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% reduced (accommodation, restaurants, transport)' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% reduced (foodstuffs, pharmaceuticals, first 130 m² residence)' },
      { code: '3', rateBp: 300, type: 'standard', euReverse: 0, description: '3% super-reduced (cultural goods, waste services, books)' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% exports / intra-EU B2B / basic goods (renewed to 31 Dec 2026)' },
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
    // returnLayout omitted — the VAT 4 return engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly above €1M turnover
    reverseChargeEffectiveRateBp: 1900,
  },

  reporting: {
    // format omitted — annual accounts layout is a B-milestone
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
      { code: '2300', name: 'Employee taxes payable (PAYE/GHS)', type: 'liability', normalBalance: 'credit' },
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
    // statutoryAccounts omitted — annual accounts layout is a B-milestone
  },

  compliance: {
    // Quarterly VAT 4 return due the 10th of the SECOND month after the
    // quarter (Q1 -> 10 May; monthly above €1M turnover); annual accounts
    // 42 days after the AGM (~10-13 months after FYE in practice); CIT TD4
    // — from TY2026 permanently due 31 January of the second year
    // following the tax year (TY2026 -> 31 Jan 2028).
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Qn', deadlineRule: 'cy-vat-quarterly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'cy-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'cy-td4' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Cyprus is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (CY additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9928); no
    // domestic mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Cypriot SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '3300', equityAccount: '3200' },
};