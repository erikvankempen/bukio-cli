/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Latvia jurisdiction profile (Phase E — LV profile).
//
// Data sources: research brief at docs-research/lv-profile.md — PVN 21/12/5,
// VAT number LV + 11 digits (unified registration number), registration
// threshold €50,000, Peppol EAS 9939 (official EAS codelist, release 8 Dec
// 2025), monthly PVN by the 20th (NO quarterly option; EC Sales List same
// schedule), annual report approved within 6 months + filed within 1 month
// of approval (≈ 31 July), CIT on distributions only, standard kontu plāns
// chart (Ministry-of-Finance-guided).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (PVN return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (gada
//                            pārskats layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Latvian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Latvian additions
//                            are a B-milestone
//   - national e-invoicing  B2B e-invoicing requirements introduced (2025
//                            timeframe) — a domestic engine is a
//                            B-milestone; 'peppol-bis-3.0' registered for
//                            cross-border (LV is a Peppol participant,
//                            EAS 9939)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'LV',
    baseCurrency: 'EUR',
    locale: 'lv',
    legalForms: ['sia', 'as', 'ik', 'sia-maza'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Reģistrācijas numurs (unified registration number), 11 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^LV\d{11}$/i, // LV + unified registration number (11 digits)
    // 9939 = Latvia VAT number (EAS codelist; 0218 = unified reg. number)
    peppolSchemeId: '9939',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2100,
    // Small business: registration threshold €50,000
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% pievienotās vērtības nodoklis' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12% samazinātā likme' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% samazinātā likme' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% eksports / intra-EU piegādes' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Atbrīvotās piegādes' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Apgrieztā PVN piemērošana' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU iegāde' },
    ],
    accounts: {
      // Standard kontu plāns: 1510 Priekšnodoklis (input, asset) / 1520 PVN
      // budžetā (output, liability); the settlement account 1530 absorbs
      // the return balance + rounding differences
      ledger: [
        { code: '1510', name: 'Priekšnodoklis', type: 'asset', normalBalance: 'debit' },
        { code: '1520', name: 'PVN budžetā', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1530',
      differenceDefault: '1530',
      settlementAccountName: 'PVN — norēķini',
    },
    // returnLayout omitted — the PVN return engine is a B-milestone
    filingPeriodicity: 'monthly', // no quarterly option (EDS monthly)
    reverseChargeEffectiveRateBp: 2100,
  },

  reporting: {
    // format omitted — gada pārskats layout is a B-milestone
    taxonomy: null,
    // Standard kontu plāns (Ministry-of-Finance-guided) — Latvian names
    defaultChart: [
      { code: '1000', name: 'Norēķinu konti bankā', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Kase', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Pircēju parādi', type: 'asset', normalBalance: 'debit' },
      { code: '1510', name: 'Priekšnodoklis', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Pamatlīdzekļi', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Uzkrātais nolietojums', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Krājumi', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Piegādātāju parādi', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'PVN budžetā', type: 'liability', normalBalance: 'credit' },
      { code: '1400', name: 'Darba samaksas parādi', type: 'liability', normalBalance: 'credit' },
      { code: '1530', name: 'PVN — norēķini', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Pamatkapitāls', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Nesadalītā peļņa', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Pārskata gada peļņa/zaudējumi', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Ieņēmumi no preču pārdošanas', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Citi ieņēmumi', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Iepirkto preču un materiālu izmaksas', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Darbības izmaksas', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Darbaspēka izmaksas', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Pamatlīdzekļu nolietojums', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Citi saimnieciskās darbības izdevumi', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Finanšu izdevumi', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — gada pārskats layout is a B-milestone
  },

  compliance: {
    // Monthly PVN due the 20th of the following month (no quarterly
    // option); annual report approved within 6 months of FYE and filed
    // within 1 month of approval (≈ 31 July). CIT is on distributions
    // only — no annual CIT return, so the calendar carries VAT + annual
    // accounts.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'lv-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'lv-annual-accounts' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Latvia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (LV additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9939); the
    // domestic B2B e-invoicing framework (2025 timeframe) is a B-milestone
    // auditFile omitted — no Latvian SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};