/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Bulgaria jurisdiction profile (Phase E — BG profile).
//
// Data sources: research brief at docs-research/bg-profile.md — IVA 20% + 9%
// (hotel/restaurant accommodation), VAT number BG + 9-10 digits, EIK/UIC
// company number, Peppol EAS 9926 (official EAS codelist, release 8 Dec
// 2025), monthly VAT return by the 14th of the following month, annual
// accounts + CIT return (form 1010) both due 30 June, NSS statutory chart
// (Национален сметкоплан, simplified skeleton).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (ДДС return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (ГФО
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Bulgarian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Bulgarian additions
//                            are a B-milestone
//   - national e-invoicing  no domestic XML mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (BG is a Peppol
//                            participant, EAS 9926)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'BG',
    baseCurrency: 'EUR', // eurozone since 1 Jan 2026 (conversion 1.95583 BGN/EUR)
    locale: 'bg',
    legalForms: ['eood', 'ood', 'ad', 'ead', 'et'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // EIK / UIC (ЕИК), 9 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^BG\d{9,10}$/i, // BG + EIK (9) or 10 digits
    // 9926 = Bulgaria VAT number (EAS codelist)
    peppolSchemeId: '9926',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2000,
    // Small business: registration threshold €51,130 (BGN 100,000 fixed at
    // the 1 Jan 2026 euro conversion)
    smallBusinessScheme: 'compound-scheme',
    codes: [
      { code: '20', rateBp: 2000, type: 'standard', euReverse: 0, description: '20% данък върху добавената стойност' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% настаняване (хотели/ресторанти)' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% износ / вътреобщностни доставки' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Освободени доставки' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Обратно начисляване' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Вътреобщностно придобиване' },
    ],
    accounts: {
      // NSS-structured: 1500 ДДС за възстановяване (input, asset) / 1510
      // ДДС за внасяне (output, liability); the settlement account 1520
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '1500', name: 'ДДС за възстановяване', type: 'asset', normalBalance: 'debit' },
        { code: '1510', name: 'ДДС за внасяне', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1520',
      differenceDefault: '1520',
      settlementAccountName: 'ДДС — разплащания с бюджета',
    },
    // returnLayout omitted — the monthly ДДС return engine is a B-milestone
    filingPeriodicity: 'monthly',
    reverseChargeEffectiveRateBp: 2000,
  },

  reporting: {
    // format omitted — ГФО (annual financial statements) layout is a B-milestone
    taxonomy: null,
    // NSS (statutory) chart — simplified 1xxx-4xxx skeleton, Bulgarian names
    defaultChart: [
      { code: '1010', name: 'Банкови сметки', type: 'asset', normalBalance: 'debit' },
      { code: '1000', name: 'Каса', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Клиенти', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'ДДС за възстановяване', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Дълготрайни активи', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Амортизация на ДА', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Материални запаси', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Доставчици', type: 'liability', normalBalance: 'credit' },
      { code: '1510', name: 'ДДС за внасяне', type: 'liability', normalBalance: 'credit' },
      { code: '1410', name: 'Задължения към персонала', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'ДДС — разплащания с бюджета', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Основен капитал', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Неразпределена печалба', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Печалба (загуба) за годината', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Приходи от продажби', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Други приходи', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Разходи за материали', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Разходи за услуги', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Разходи за персонала', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Амортизации', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Финансови разходи', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Данъци', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1010',
    inferTaxonomy: null,
    // statutoryAccounts omitted — ГФО layout is a B-milestone
  },

  compliance: {
    // Monthly ДДС return due the 14th of the following month; annual
    // accounts filed at the Trade Register and the CIT return (form 1010)
    // both due 30 June of the following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'bg-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'bg-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'bg-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Bulgaria is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (BG additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9926); no
    // domestic XML mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Bulgarian SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};