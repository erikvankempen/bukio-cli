/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Estonia jurisdiction profile (Phase E — EE profile).
//
// Data sources: research brief at docs-research/ee-profile.md — käibemaks
// 24% (raised from 22% on 1 Jul 2025) + 9% (accommodation), VAT number EE +
// 9 digits, registrikood 8 digits, registration threshold €40,000, Peppol
// EAS 9931 (official EAS codelist, release 8 Dec 2025), monthly KMD by the
// 20th, annual report to the e-Äriregister within 6 months of FYE (30
// June), CIT on distributions only (10th of the month following the
// distribution), RMP-convention chart (no statutory chart).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (KMD return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (annual
//                            report layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Estonian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Estonian additions
//                            are a B-milestone
//   - national e-invoicing  no domestic XML mandate yet (e-arve ecosystem
//                            is convention-led) — 'peppol-bis-3.0' registered
//                            for cross-border (EE is a Peppol participant,
//                            EAS 9931)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// Documents render in Estonian (languages ['et'], defaultLanguage
// 'et') — full i18n table since 16 Aug 2026.

export default {
  meta: {
    country: 'EE',
    baseCurrency: 'EUR',
    locale: 'et',
    legalForms: ['ou', 'as', 'fie', 'tu'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // registrikood, 8 digits (e-Äriregister)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^EE\d{9}$/i, // EE + registrikood-based 9-digit VAT number
    // 9931 = Estonia VAT number (EAS codelist)
    peppolSchemeId: '9931',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2400,
    // Small business: registration threshold €40,000
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '24', rateBp: 2400, type: 'standard', euReverse: 0, description: '24% käibemaks' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% majutus (hotell)' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% raamatud / eksport / intra-EU' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Maksuvabad käibed' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Pöördmaksustamine (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Ühendusesisene soetamine' },
    ],
    accounts: {
      // RMP convention: 1510 Sisendkäibemaks (input, asset) / 1520
      // Väljundkäibemaks (output, liability); the settlement account 1530
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '1510', name: 'Sisendkäibemaks', type: 'asset', normalBalance: 'debit' },
        { code: '1520', name: 'Väljundkäibemaks', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1530',
      differenceDefault: '1530',
      settlementAccountName: 'Käibemaks — kohustus',
    },
    // returnLayout omitted — the KMD return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for smaller taxpayers
    reverseChargeEffectiveRateBp: 2400,
  },

  reporting: {
    // format omitted — annual report layout is a B-milestone
    taxonomy: null,
    // RMP-convention chart (no statutory chart) — Estonian names
    defaultChart: [
      { code: '1000', name: 'Arvelduskonto', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Kassa', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Ostjate laekumata arved', type: 'asset', normalBalance: 'debit' },
      { code: '1510', name: 'Sisendkäibemaks', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Põhivara', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Akumuleeritud kulum', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Varud', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Hankijatele tasumata arved', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'Väljundkäibemaks', type: 'liability', normalBalance: 'credit' },
      { code: '1400', name: 'Võlad töötajatele', type: 'liability', normalBalance: 'credit' },
      { code: '1530', name: 'Käibemaks — kohustus', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Osakapital', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Eelmiste perioodide jaotamata kasum', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Aruandeaasta kasum/kahjum', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Müügitulu', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Muud äritulud', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Kaubad, toore, materjal ja teenused', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Mitmesugused tegevuskulud', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Tööjõukulud', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Põhivara kulum ja väärtuse langus', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Muud ärikulud', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Intressikulud', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — annual report layout is a B-milestone
  },

  compliance: {
    // Monthly KMD due the 20th of the following month; annual report to the
    // e-Äriregister within 6 months of FYE (30 June). CIT is on
    // distributions only (10th of the month after the distribution) — no
    // annual CIT return, so the calendar carries VAT + annual accounts.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'ee-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'ee-annual-accounts' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Estonia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (EE additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9931); the
    // e-arve ecosystem is convention-led — a domestic mandate engine is a
    // B-milestone
    // auditFile omitted — no Estonian SAF-T
    languages: ['et'],
    defaultLanguage: 'et',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};