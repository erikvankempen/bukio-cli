/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Lithuania jurisdiction profile (Phase E — LT profile).
//
// Data sources: research brief at docs-research/lt-profile.md — PVM 21/9/5,
// VAT number LT + 9 or 12 digits, Įmonės kodas 9 digits, registration
// threshold €45,000, Peppol EAS 9937 (official EAS codelist, release 8 Dec
// 2025), monthly PVM by the 25th (i.SAF), annual accounts approved within
// 4 months + filed shortly after (≈ 30 April), CIT return due 1 October,
// Įmonių sąskaitų planas statutory-standard chart (MF order, simplified
// skeleton).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (PVM return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (annual
//                            statements layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Lithuanian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Lithuanian additions
//                            are a B-milestone
//   - national e-invoicing  e-Sąskaita B2B framework (2025 mandate) is a
//                            B-milestone; 'peppol-bis-3.0' registered for
//                            cross-border (LT is a Peppol participant,
//                            EAS 9937)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'LT',
    baseCurrency: 'EUR',
    locale: 'lt',
    legalForms: ['uab', 'ab', 'ii', 'mb'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Įmonės kodas (legal entity code), 9 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^LT\d{9,12}$/i, // LT + 9 (legal entity) or 12 digits
    // 9937 = Lithuania VAT number (EAS codelist; 0200 = legal entity code)
    peppolSchemeId: '9937',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2100,
    // Small business: registration threshold €45,000
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% pridėtinės vertės mokestis' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% lengvatinis tarifas' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% lengvatinis tarifas' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% eksportas / intra-EU tiekimas' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Neapmokestinami tiekimai' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Atvirkštinis apmokestinimas' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU įsigijimas' },
    ],
    accounts: {
      // Įmonių sąskaitų planas: 1500 Pirkimo PVM (input, asset) / 1510
      // Pardavimo PVM (output, liability); the settlement account 1520
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '1500', name: 'Pirkimo PVM', type: 'asset', normalBalance: 'debit' },
        { code: '1510', name: 'Pardavimo PVM', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1520',
      differenceDefault: '1520',
      settlementAccountName: 'PVM — atsiskaitymai su biudžetu',
    },
    // returnLayout omitted — the PVM return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for smaller taxpayers
    reverseChargeEffectiveRateBp: 2100,
  },

  reporting: {
    // format omitted — annual statements layout is a B-milestone
    taxonomy: null,
    // Įmonių sąskaitų planas (MF order) — Lithuanian names, simplified
    defaultChart: [
      { code: '1000', name: 'Pinigai banko sąskaitoje', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Kasa', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Pirkėjų skolos', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Pirkimo PVM', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Ilgalaikis materialusis turtas', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Sukauptas nusidėvėjimas', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Atsargos', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Skolos tiekėjams', type: 'liability', normalBalance: 'credit' },
      { code: '1510', name: 'Pardavimo PVM', type: 'liability', normalBalance: 'credit' },
      { code: '1400', name: 'Skolos darbuotojams', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'PVM — atsiskaitymai su biudžetu', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Įstatinis kapitalas', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Nepaskirstytasis pelnas', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Ataskaitinių metų pelnas (nuostoliai)', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Pardavimo pajamos', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Kitos pajamos', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Parduotų prekių savikaina', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Veiklos sąnaudos', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Darbo užmokesčio sąnaudos', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Ilgalaikio turto nusidėvėjimas', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Kitos veiklos sąnaudos', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Finansinės sąnaudos', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — annual statements layout is a B-milestone
  },

  compliance: {
    // Monthly PVM due the 25th of the following month (i.SAF; payment same
    // day); annual accounts approved within 4 months of FYE and filed
    // shortly after (~30 April); CIT annual return due 1 October of the
    // following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'lt-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'lt-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'lt-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Lithuania is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (LT additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9937); the
    // e-Sąskaita B2B mandate (2025 framework) is a B-milestone
    // auditFile omitted — no Lithuanian SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};