/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Ireland jurisdiction profile (Phase C — IE profile).
//
// Data sources: research brief at docs-research/ie-profile.md — VAT rates
// from the Revenue current-rates table (23/13.5/9/4.8 from 1 Jan 2026),
// registration thresholds €85,000 goods / €42,500 services (Finance Act
// 2024, effective 1 Jan 2025), VAT3 bi-monthly returns due the 23rd of the
// month after the period end (Revenue), company number + VAT number formats
// from the CRO/Revenue, Peppol EAS codelist (9935 = Ireland VAT number).
//
// Key notes: Ireland has NO statutory chart of accounts — the default chart
// follows the UK-style QuickBooks/Xero convention (1000s assets, 2000s
// liabilities, 3000s equity, 4000s income, 5000s+ expenses), adapted for
// Irish VAT control accounts and EUR. The VAT3 return is bi-monthly (six
// two-month periods per year) — the YYYY-Pn period shape Norway uses, with
// an Irish deadline schedule (23rd of the month after the period).
//
// Phase C scope discipline (same contract as the Phase B markets): register
// ONLY what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (the VAT3 9-box
//                            return engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Companies
//                            Act 2014 / FRS 102 accounts are a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Irish SAF-T;
//                            Revenue e-audit via ROS is API-based)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); VAT Consolidation Act
//                            2010 s. 108B/113 additions are a B-milestone
// Registered: e-invoicing 'peppol-bis-3.0' (IE is a Peppol participant;
// no B2B mandate yet — EU directive rolling out 2027+; scheme 9935 = VAT
// number), SEPA, CAMT.053, ECB, closing 3300 -> 3200.

export default {
  meta: {
    country: 'IE',
    baseCurrency: 'EUR',
    locale: 'en',
    legalForms: ['ltd', 'dac', 'plc', 'unlimited-company', 'sole-trader', 'partnership'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // CRO company number (6-7 digits)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^IE\d{7}[A-Z]{1,2}$/i, // IE + 7 digits + 1-2 letters (IE1234567T, IE1234567TW)
    // 9935 = Ireland VAT number (EAS)
    peppolSchemeId: '9935',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2300,
    // VAT registration thresholds (Finance Act 2024, from 1 Jan 2025):
    // €85,000 goods / €42,500 services annual turnover
    smallBusinessScheme: 'threshold',
    // IE rates (Revenue, 1 Jan 2026): 23% standard / 13.5% reduced /
    // 9% second reduced (tourism & hospitality) / 4.8% livestock /
    // 0% zero rate (food, books, exports). Flat-rate farmer compensation
    // is 4.5% (2026). EU reverse charge (VATCA 2010 s. 108) applies.
    codes: [
      { code: '23', rateBp: 2300, type: 'standard', euReverse: 0, description: '23% standard rate' },
      { code: '13.5', rateBp: 1350, type: 'standard', euReverse: 0, description: '13.5% reduced rate' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% second reduced rate (tourism & hospitality)' },
      { code: '4.8', rateBp: 480, type: 'standard', euReverse: 0, description: '4.8% livestock rate' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% zero rate' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exempt (VATCA 2010 Schedule 1)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Reverse charge (s. 108)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-Community acquisition' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Margin scheme' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Private use' },
    ],
    accounts: {
      // VAT control accounts per the UK-style convention (research brief
      // §6): 2110 input (asset) / 2100 output (liability); the settlement
      // account 2120 absorbs the VAT3 return balance + rounding differences
      ledger: [
        { code: '2110', name: 'VAT on purchases (reclaimable)', type: 'asset', normalBalance: 'debit' },
        { code: '2100', name: 'VAT on sales (output)', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2120',
      differenceDefault: '2120', // return balance/rounding lands on the settlement account
      settlementAccountName: 'VAT payable to Revenue',
    },
    // returnLayout omitted — the VAT3 9-box return is a B-milestone
    filingPeriodicity: 'bimonthly', // VAT3: six 2-month periods per year
    reverseChargeEffectiveRateBp: 2300,
  },

  reporting: {
    // format omitted — Companies Act 2014 / FRS 102 accounts are a
    // B-milestone (filed with the annual return to the CRO)
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
      { code: '2100', name: 'VAT on sales (output)', type: 'liability', normalBalance: 'credit' },
      { code: '2110', name: 'VAT on purchases (reclaimable)', type: 'asset', normalBalance: 'debit' },
      { code: '2120', name: 'VAT — balance due to/from Revenue (settlement)', type: 'liability', normalBalance: 'credit' },
      { code: '2200', name: 'PAYE / PRSI / USC control', type: 'liability', normalBalance: 'credit' },
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
    // debtors account for invoice postings (UK-style convention)
    debtorsAccount: '1100',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Companies Act 2014 layouts are a
    // B-milestone
  },

  compliance: {
    // VAT3: six bi-monthly periods (Jan/Feb ... Nov/Dec), due the 23rd of
    // the month after the period end (Revenue). Annual accounts + Form B1
    // annual return filed with the CRO within 9 months of the FYE (private
    // limited companies); Form CT1 corporation tax return 9 months after
    // the accounting period end.
    filingTypes: [
      { type: 'VAT3', periodShape: 'YYYY-Pn', deadlineRule: 'ie-bimonthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'ie-9-months' },
      { type: 'CT1', periodShape: 'YYYY', deadlineRule: 'ie-9-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // IE is SEPA, EUR
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (VATCA 2010 s. 108B/113 additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // Peppol participant; no B2B mandate yet
    // (EU e-invoicing directive rolling out 2027+); scheme 9935 = VAT number
    // auditFile omitted — no Irish SAF-T; Revenue e-audit via ROS is
    // API-based
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '3300', equityAccount: '3200' },
};
