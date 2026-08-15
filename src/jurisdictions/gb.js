/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// United Kingdom jurisdiction profile (Phase B — GB profile).
//
// Data sources: GOV.UK/HMRC/ICAEW/FRC research brief at
// docs-research/gb-profile.md (every item source-verified, confidence per
// item). The UK has NO statutory chart of accounts — the default chart
// follows the dominant QuickBooks/Xero 4-digit convention (1000s assets,
// 2000s liabilities, 3000s equity, 4000s income, 5000s+ expenses).
//
// Phase B scope discipline (same contract as LU B1): register ONLY what
// maps to existing generic engines. Everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (UK 9-box VAT
//                            return engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (FRS 102
//                            §1A / FRS 105 iXBRL filing is a B-milestone)
//   - documents.eInvoicing  omitted → UBL export fails (mandatory
//                            e-invoicing from 2029 per Budget 2025; the UK
//                            Peppol scheme ID is not yet assigned — roadmap
//                            due Budget 2026)
//   - documents.auditFile   omitted → XAF export fails (no UK SAF-T; MTD
//                            VAT filing is API-based)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (VAT Regulations 1995 reg. 14 full-invoice
//                            rule set is a B-milestone)
//   - exchange.paymentFormats [] → SEPA batch export fails (SEPA is NOT a
//                            domestic GBP rail — BACS/Faster Payments
//                            formats are a B-milestone; UK remains a SEPA
//                            member for EUR transfers only)

export default {
  meta: {
    country: 'GB',
    baseCurrency: 'GBP',
    locale: 'en-GB',
    // legal forms per the GB research brief §4 (GOV.UK business structures)
    legalForms: ['sole-trader', 'private-limited-company', 'partnership', 'llp', 'cic', 'charity'],
    // 31 March is tax-year aligned (HMRC treats 31 Mar-5 Apr as aligned
    // under basis period reform); any year end is legal
    defaultFiscalYearEnd: '03-31',
  },

  identifiers: {
    companyIdLabel: 'company_number', // Companies House CRN: 8 chars (8 digits, or 2-letter prefix + 6)
    vatIdLabel: 'vat_number',
    vatIdFormat: /^(GB|XI)\d{9}$/i, // GB or XI (Northern Ireland) + 9 digits
    // peppolSchemeId: not yet assigned for the UK (2026 roadmap) — UBL
    // export fails loudly until then
    accountNumber: { kind: 'sort-code-account' }, // sort code (6) + account number (8); IBAN for international only
  },

  tax: {
    system: 'vat',
    standardRateBp: 2000, // 20% since 4 Jan 2011
    smallBusinessScheme: 'flat-rate', // Flat Rate Scheme: join ≤£150K turnover
    // HMRC VAT rates (GOV.UK): 20% standard / 5% reduced / 0% zero / exempt.
    // No EU reverse-charge code (post-Brexit: exports are zero-rated; the
    // domestic reverse charge exists for construction etc.)
    codes: [
      { code: '20', rateBp: 2000, type: 'standard', euReverse: 0, description: '20% standard rate' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% reduced rate' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% zero rate' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exempt' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Reverse charge' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Margin scheme' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Private use' },
    ],
    accounts: {
      // VAT control accounts per the UK chart convention (research §7):
      // 2110 input (asset) / 2100 output (liability); the settlement
      // account 2120 absorbs the return balance + rounding differences
      ledger: [
        { code: '2110', name: 'VAT input (reclaimable VAT)', type: 'asset', normalBalance: 'debit' },
        { code: '2100', name: 'VAT control (output VAT)', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2120',
      differenceDefault: '2120', // return balance/rounding lands on the settlement account
      settlementAccountName: 'VAT payable to HMRC',
    },
    // returnLayout omitted — the UK 9-box VAT return is a B-milestone
    filingPeriodicity: 'quarterly', // default; annual/cash accounting schemes exist
    reverseChargeEffectiveRateBp: 2000,
  },

  reporting: {
    // format omitted — FRS 102 §1A / FRS 105 statutory accounts are a
    // B-milestone (iXBRL filing to Companies House)
    taxonomy: null, // no statutory taxonomy — chart is software convention
    defaultChart: [
      { code: '1000', name: 'Bank — current account', type: 'asset', normalBalance: 'debit' },
      { code: '1010', name: 'Cash (petty cash)', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Trade debtors (accounts receivable)', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Prepayments', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Stock / inventory', type: 'asset', normalBalance: 'debit' },
      { code: '1400', name: 'Other debtors', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Office equipment (fixed asset)', type: 'asset', normalBalance: 'debit' },
      { code: '1600', name: 'Accumulated depreciation — office equipment', type: 'asset', normalBalance: 'credit' },
      { code: '1700', name: 'Motor vehicles (fixed asset)', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Accumulated depreciation — motor vehicles', type: 'asset', normalBalance: 'credit' },
      { code: '2000', name: 'Trade creditors (accounts payable)', type: 'liability', normalBalance: 'credit' },
      { code: '2100', name: 'VAT control (output VAT)', type: 'liability', normalBalance: 'credit' },
      { code: '2110', name: 'VAT input (reclaimable VAT)', type: 'asset', normalBalance: 'debit' },
      { code: '2120', name: 'VAT — balance due to/from HMRC (settlement)', type: 'liability', normalBalance: 'credit' },
      { code: '2200', name: 'PAYE / National Insurance control', type: 'liability', normalBalance: 'credit' },
      { code: '2300', name: 'Accruals', type: 'liability', normalBalance: 'credit' },
      { code: '2400', name: 'Directors\' loan account', type: 'liability', normalBalance: 'credit' },
      { code: '2500', name: 'Corporation tax payable', type: 'liability', normalBalance: 'credit' },
      { code: '2600', name: 'Other creditors', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Called-up share capital', type: 'equity', normalBalance: 'credit' },
      { code: '3100', name: 'Share premium', type: 'equity', normalBalance: 'credit' },
      { code: '3200', name: 'Profit and loss account (retained earnings)', type: 'equity', normalBalance: 'credit' },
      { code: '3300', name: 'Profit / (loss) for the year', type: 'equity', normalBalance: 'credit' },
      { code: '4000', name: 'Sales — goods', type: 'income', normalBalance: 'credit' },
      { code: '4100', name: 'Sales — services', type: 'income', normalBalance: 'credit' },
      { code: '4200', name: 'Other income', type: 'income', normalBalance: 'credit' },
      { code: '5000', name: 'Purchases (cost of goods)', type: 'expense', normalBalance: 'debit' },
      { code: '5100', name: 'Cost of sales — stock movements', type: 'expense', normalBalance: 'debit' },
      { code: '6000', name: 'Wages and salaries', type: 'expense', normalBalance: 'debit' },
      { code: '6100', name: 'Rent and rates', type: 'expense', normalBalance: 'debit' },
      { code: '6200', name: 'Utilities (electricity, gas, water)', type: 'expense', normalBalance: 'debit' },
      { code: '6300', name: 'Telephone and internet', type: 'expense', normalBalance: 'debit' },
      { code: '6400', name: 'Insurance', type: 'expense', normalBalance: 'debit' },
      { code: '6500', name: 'Motor and travel expenses', type: 'expense', normalBalance: 'debit' },
      { code: '6600', name: 'Repairs and maintenance', type: 'expense', normalBalance: 'debit' },
      { code: '6700', name: 'Printing, postage and stationery', type: 'expense', normalBalance: 'debit' },
      { code: '6800', name: 'Professional fees (accountant, legal)', type: 'expense', normalBalance: 'debit' },
      { code: '6900', name: 'Advertising and marketing', type: 'expense', normalBalance: 'debit' },
      { code: '7000', name: 'Bank charges and interest', type: 'expense', normalBalance: 'debit' },
      { code: '7100', name: 'Depreciation', type: 'expense', normalBalance: 'debit' },
      { code: '7200', name: 'Bad debts written off', type: 'expense', normalBalance: 'debit' },
      { code: '7300', name: 'Miscellaneous expenses', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (research §7 convention)
    debtorsAccount: '1100',
    inferTaxonomy: null,
    // statutoryAccounts omitted — FRS 102/105 layouts are a B-milestone
  },

  compliance: {
    // Companies House / HMRC deadlines (research §8): annual accounts
    // 9 months after FYE (CA 2006 s.442), CT600 12 months after period
    // end. The confirmation statement (CS01, within 14 days of the review
    // period end) needs the incorporation anniversary — no schema field,
    // documented not calendarised.
    filingTypes: [
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'gb-9-months' },
      { type: 'CT600', periodShape: 'YYYY', deadlineRule: 'gb-ct600' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    // SEPA is NOT a domestic GBP rail — BACS/Faster Payments formats are a
    // B-milestone; the strict dispatch fails loudly until then
    paymentFormats: [],
    fxSource: 'ecb', // ECB publishes GBP rates
    baseCurrency: 'GBP',
  },

  documents: {
    // invoiceCompliance omitted — VAT Regulations 1995 reg. 14 rule set
    // is a B-milestone (invoice finalization fails loudly until then)
    // eInvoicing omitted — 2029 mandate, Peppol scheme ID unassigned
    // auditFile omitted — no UK SAF-T; MTD filing is API-based
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '3300', equityAccount: '3200' },
};
