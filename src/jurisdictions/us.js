/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// United States jurisdiction profile (Phase B — US profile).
//
// Data sources: research brief at docs-research/us-profile.md (IRS, SBA,
// PwC tax summaries, QuickBooks convention — every item source-verified,
// confidence per item). The US has NO federal VAT/sales tax (state-level),
// NO statutory chart (QuickBooks numbering convention), NO national company
// registration number (state-level), NO e-invoicing mandate and NO SEPA.
//
// Phase B scope discipline (same contract as LU/GB/FR): register ONLY what
// maps to existing generic engines; everything else fails loudly:
//   - tax.system 'none' — no federal VAT; state sales tax is per-state
//     registration + a control account (2100), not a bukio engine concern
//   - reporting.format       omitted → financial statements fail (US GAAP
//                             presentation is a B-milestone)
//   - documents.eInvoicing   omitted → UBL export fails (no mandate; EDI is
//                             the legacy large-company standard)
//   - documents.auditFile    omitted → XAF export fails (no US SAF-T)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                             (no federal invoice rules — a US rule set is
//                             a B-milestone)
//   - exchange.paymentFormats [] → SEPA batch export fails (no SEPA; ACH
//                             export is a B-milestone)
//   - exchange.bankStatementFormats ['csv'] — CAMT.053 availability at US
//                             banks is unverified; only CSV is registered
//   - compliance.filingTypes: FEDERAL_INCOME_TAX (1120, 15th of the 4th
//                             month after FYE) + PAYROLL_941 (quarterly,
//                             month-end after the quarter); ESTIMATED_TAX
//                             (4 fixed dates) is not calendarisable yet

export default {
  meta: {
    country: 'US',
    baseCurrency: 'USD',
    locale: 'en-US',
    // SBA business structures (research §4)
    legalForms: ['sole-proprietorship', 'partnership', 'llc', 's-corp', 'c-corp', 'non-profit'],
    // calendar year is the norm for SMEs; any tax year is allowed
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    // no national company number — registration is state-level (Secretary
    // of State); the generic column holds the state registration number
    companyIdLabel: 'registration_id',
    vatIdLabel: 'ein', // EIN is the federal tax ID (XX-XXXXXXX), not a VAT number
    vatIdFormat: /^\d{2}-\d{7}$/, // EIN format (advisory validation when set)
    // peppolSchemeId omitted — no e-invoicing mandate, no Peppol scheme
    accountNumber: { kind: 'routing-account' }, // ABA routing (9) + account (8-12)
  },

  tax: {
    system: 'none', // no federal VAT; sales tax is state-level
    standardRateBp: null,
    smallBusinessScheme: null,
    codes: [], // no federal VAT codes; state sales tax is per-state
    accounts: {
      // no VAT ledger — the US books no input/output VAT. postingDefaults
      // only requires a VAT liability when the VAT module is enabled.
      ledger: [],
      // fileDefault/differenceDefault/settlementAccountName omitted — there is no
      // federal VAT return or settlement in the US
    },
    // returnLayout omitted — no federal VAT return engine
    filingPeriodicity: null,
    reverseChargeEffectiveRateBp: null,
  },

  reporting: {
    // format omitted — US GAAP financial statements are a B-milestone
    taxonomy: null,
    // default chart per the dominant QuickBooks numbering convention
    // (research §7): 1000s assets, 2000s liabilities, 3000s equity, 4000s
    // income, 5000s COGS, 6000s+ expenses. 1410 is a contra-asset (credit
    // normal balance — enabled by migration 023).
    defaultChart: [
      { code: '1000', name: 'Checking account', type: 'asset', normalBalance: 'debit' },
      { code: '1010', name: 'Savings account', type: 'asset', normalBalance: 'debit' },
      { code: '1020', name: 'Cash on hand (petty cash)', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Accounts receivable', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Inventory', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Prepaid expenses', type: 'asset', normalBalance: 'debit' },
      { code: '1400', name: 'Fixed assets (equipment, vehicles)', type: 'asset', normalBalance: 'debit' },
      { code: '1410', name: 'Accumulated depreciation (contra-asset)', type: 'asset', normalBalance: 'credit' },
      { code: '1500', name: 'Other assets / deposits', type: 'asset', normalBalance: 'debit' },
      { code: '2000', name: 'Accounts payable', type: 'liability', normalBalance: 'credit' },
      { code: '2100', name: 'Sales tax payable (state/local)', type: 'liability', normalBalance: 'credit' },
      { code: '2200', name: 'Payroll liabilities (941 withholding)', type: 'liability', normalBalance: 'credit' },
      { code: '2300', name: 'Accrued expenses', type: 'liability', normalBalance: 'credit' },
      { code: '2400', name: 'Federal income tax payable', type: 'liability', normalBalance: 'credit' },
      { code: '2500', name: 'State income tax payable', type: 'liability', normalBalance: 'credit' },
      { code: '2600', name: 'Loans payable / credit cards', type: 'liability', normalBalance: 'credit' },
      { code: '2700', name: 'Other liabilities', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Owner\'s equity (members\' equity)', type: 'equity', normalBalance: 'credit' },
      { code: '3100', name: 'Owner\'s draws', type: 'equity', normalBalance: 'debit' },
      { code: '3200', name: 'Retained earnings', type: 'equity', normalBalance: 'credit' },
      { code: '3300', name: 'Current year earnings (profit/loss)', type: 'equity', normalBalance: 'credit' },
      { code: '4000', name: 'Sales — goods', type: 'income', normalBalance: 'credit' },
      { code: '4100', name: 'Sales — services', type: 'income', normalBalance: 'credit' },
      { code: '4200', name: 'Other income (interest, misc.)', type: 'income', normalBalance: 'credit' },
      { code: '5000', name: 'Cost of goods sold', type: 'expense', normalBalance: 'debit' },
      { code: '5100', name: 'Purchases / materials', type: 'expense', normalBalance: 'debit' },
      { code: '6000', name: 'Rent', type: 'expense', normalBalance: 'debit' },
      { code: '6100', name: 'Utilities', type: 'expense', normalBalance: 'debit' },
      { code: '6200', name: 'Wages and salaries', type: 'expense', normalBalance: 'debit' },
      { code: '6300', name: 'Payroll taxes (employer FICA, FUTA, SUTA)', type: 'expense', normalBalance: 'debit' },
      { code: '6400', name: 'Insurance', type: 'expense', normalBalance: 'debit' },
      { code: '6500', name: 'Office expenses and supplies', type: 'expense', normalBalance: 'debit' },
      { code: '6600', name: 'Professional fees (accounting, legal)', type: 'expense', normalBalance: 'debit' },
      { code: '6700', name: 'Travel and meals', type: 'expense', normalBalance: 'debit' },
      { code: '6800', name: 'Depreciation expense', type: 'expense', normalBalance: 'debit' },
      { code: '6900', name: 'Advertising and marketing', type: 'expense', normalBalance: 'debit' },
      { code: '7000', name: 'Repairs and maintenance', type: 'expense', normalBalance: 'debit' },
      { code: '7100', name: 'Bank charges and merchant fees', type: 'expense', normalBalance: 'debit' },
      { code: '7200', name: 'Miscellaneous expenses', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings
    debtorsAccount: '1100',
    inferTaxonomy: null,
    // statutoryAccounts omitted — US GAAP presentation is a B-milestone
  },

  compliance: {
    // FEDERAL_INCOME_TAX: Form 1120/1120-S due the 15th day of the 4th
    // month after the tax year end (April 15 calendar) — research §5.1.
    // PAYROLL_941: quarterly, due the last day of the month after the
    // quarter (standard IRS practice; the brief confirms quarterly).
    // ESTIMATED_TAX (15th of Apr/Jun/Sep/Jan) is not calendarisable with
    // the current period shapes — documented, not listed.
    filingTypes: [
      { type: 'FEDERAL_INCOME_TAX', periodShape: 'YYYY', deadlineRule: 'us-1120' },
      { type: 'PAYROLL_941', periodShape: 'YYYY-Qn', deadlineRule: 'us-941' },
    ],
  },

  exchange: {
    bankStatementFormats: ['csv'], // CAMT.053 unverified at US banks — not registered
    paymentFormats: [], // no SEPA; ACH export is a B-milestone
    fxSource: 'ecb', // ECB publishes USD rates
    baseCurrency: 'USD',
  },

  documents: {
    // invoiceCompliance omitted — no federal invoice rules; a US rule set
    // is a B-milestone
    // eInvoicing omitted — no mandate, no Peppol scheme
    // auditFile omitted — no US SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '3300', equityAccount: '3200' },
};
