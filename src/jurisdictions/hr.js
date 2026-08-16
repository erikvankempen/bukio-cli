/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Croatia jurisdiction profile (Phase E — HR profile).
//
// Data sources: research brief at docs-research/hr-profile.md — PDV 25/13/5,
// VAT number HR + 11 digits (OIB with ISO 7064 Mod 11,10 check digit),
// Peppol EAS 9934 (official EAS codelist, release 8 Dec 2025), monthly PDV
// return by the 20th of the following month (quarterly option for small
// taxpayers), annual accounts (RGFI via FINA) + CIT return both due 30
// April, standard Računski plan convention chart (no statutory chart).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (PDV return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (RGFI
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Croatian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Croatian additions
//                            are a B-milestone
//   - national e-invoicing  no domestic XML mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (HR is a Peppol
//                            participant, EAS 9934)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'HR',
    baseCurrency: 'EUR', // eurozone since 1 Jan 2023 (conversion 7.53450 HRK/EUR)
    locale: 'hr',
    legalForms: ['doo', 'jdoo', 'dd', 'obrt'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // OIB (osobni identifikacijski broj), 11 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^HR\d{11}$/i, // HR + OIB (11 digits)
    // 9934 = Croatia VAT number (EAS codelist)
    peppolSchemeId: '9934',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2500,
    // Small business: registration threshold ~€40,000 (HRK 300,000 converted)
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '25', rateBp: 2500, type: 'standard', euReverse: 0, description: '25% porez na dodanu vrijednost' },
      { code: '13', rateBp: 1300, type: 'standard', euReverse: 0, description: '13% smanjena stopa' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% smanjena stopa' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% izvoz / intra-EU isporuke' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Oslobođene isporuke' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Prijenos porezne obveze (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU stjecanje' },
    ],
    accounts: {
      // Računski plan convention: 1500 Potraživanja za PDV (input, asset) /
      // 1510 Obveze za PDV (output, liability); the settlement account 1520
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '1500', name: 'Potraživanja za PDV', type: 'asset', normalBalance: 'debit' },
        { code: '1510', name: 'Obveze za PDV', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1520',
      differenceDefault: '1520',
      settlementAccountName: 'PDV — obračun s državom',
    },
    // returnLayout omitted — the PDV return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for small taxpayers
    reverseChargeEffectiveRateBp: 2500,
  },

  reporting: {
    // format omitted — RGFI layout is a B-milestone
    taxonomy: null,
    // Računski plan convention chart (no statutory chart) — Croatian names
    defaultChart: [
      { code: '1000', name: 'Banka — žiro račun', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Blagajna', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Kupci', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Potraživanja za PDV', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Dugotrajna imovina', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Ispravak vrijednosti DIM', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Zalihe', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Dobavljači', type: 'liability', normalBalance: 'credit' },
      { code: '1510', name: 'Obveze za PDV', type: 'liability', normalBalance: 'credit' },
      { code: '1400', name: 'Obveze za plaće', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'PDV — obračun s državom', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Temeljni kapital', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Zadržana dobit', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Dobit/gubitak tekuće godine', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Prihodi od prodaje', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Ostali prihodi', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Troškovi sirovina i materijala', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Troškovi usluga', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Troškovi zaposlenih', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Amortizacija', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Financijski rashodi', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Porez na dobit', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — RGFI layout is a B-milestone
  },

  compliance: {
    // Monthly PDV return due the 20th of the following month (quarterly
    // option for small taxpayers); annual accounts (RGFI) + CIT return
    // both due 30 April of the following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'hr-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'hr-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'hr-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Croatia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (HR additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9934); no
    // domestic XML mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Croatian SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};