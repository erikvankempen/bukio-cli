/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Poland jurisdiction profile (Phase F — PL profile).
//
// Data sources: research brief at docs-research/pl-profile.md — VAT 23/8/5,
// VAT number PL + 10 digits (NIP), REGON/KRS company numbers, registration
// threshold PLN 200K, Peppol EAS 9945 (official EAS codelist, release 8 Dec
// 2025), JPK_V7M monthly by the 25th (quarterly JPK_V7K for small
// taxpayers), annual accounts ~6 months (approval) + 15 days (KRS filing),
// CIT-8 by 31 March, Rozporządzenie MF statutory framework chart
// (simplified skeleton).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (JPK_V7 return
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Polish
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Polish SAF-T;
//                            JPK is a different schema)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Polish additions are
//                            a B-milestone
//   - KSeF                  mandatory domestic e-invoicing (live Feb/Apr
//                            2026) is a separate XML schema — B-milestone
//                            (like FatturaPA); 'peppol-bis-3.0' registered
//                            for cross-border (PL is a Peppol participant,
//                            EAS 9945)
// Registered: SEPA, CAMT.053, ECB (PLN base currency), closing 4030 -> 4020.
// Documents render in Polish (languages ['pl'], defaultLanguage
// 'pl') — full i18n table since 16 Aug 2026.

export default {
  meta: {
    country: 'PL',
    baseCurrency: 'PLN',
    locale: 'pl',
    legalForms: ['sp-zoo', 'sa', 'spj', 'spk', 'jdg'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // NIP (tax id) 10 digits; REGON 9 / KRS
    vatIdLabel: 'tax_id',
    vatIdFormat: /^PL\d{10}$/i, // PL + NIP (10 digits)
    // 9945 = Poland VAT number (EAS codelist)
    peppolSchemeId: '9945',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2300,
    // Small business: registration threshold PLN 200,000 (≈ €47K)
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '23', rateBp: 2300, type: 'standard', euReverse: 0, description: '23% podatek od towarów i usług' },
      { code: '8', rateBp: 800, type: 'standard', euReverse: 0, description: '8% stawka obniżona' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% stawka obniżona' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% eksport / dostawy wewnątrzwspólnotowe' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Zwolnienia' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Odwrotne obciążenie (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Wewnątrzwspólnotowe nabycie' },
    ],
    accounts: {
      // MF framework: 2210 VAT naliczony (input, asset) / 2220 VAT należny
      // (output, liability); the settlement account 2230 absorbs the
      // return balance + rounding differences
      ledger: [
        { code: '2210', name: 'VAT naliczony', type: 'asset', normalBalance: 'debit' },
        { code: '2220', name: 'VAT należny', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2230',
      differenceDefault: '2230',
      settlementAccountName: 'Rozliczenie VAT',
    },
    // returnLayout omitted — the JPK_V7 return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly JPK_V7K for small taxpayers
    reverseChargeEffectiveRateBp: 2300,
  },

  reporting: {
    // format omitted — Polish financial-statements layout is a B-milestone
    taxonomy: null,
    // Rozporządzenie MF (statutory framework) — simplified skeleton, Polish names
    defaultChart: [
      { code: '1310', name: 'Rachunek bankowy', type: 'asset', normalBalance: 'debit' },
      { code: '1010', name: 'Kasa', type: 'asset', normalBalance: 'debit' },
      { code: '2010', name: 'Rozrachunki z odbiorcami', type: 'asset', normalBalance: 'debit' },
      { code: '2210', name: 'VAT naliczony', type: 'asset', normalBalance: 'debit' },
      { code: '0100', name: 'Środki trwałe', type: 'asset', normalBalance: 'debit' },
      { code: '0710', name: 'Umorzenie środków trwałych', type: 'asset', normalBalance: 'credit' },
      { code: '3300', name: 'Materiały i towary', type: 'asset', normalBalance: 'debit' },
      { code: '2020', name: 'Rozrachunki z dostawcami', type: 'liability', normalBalance: 'credit' },
      { code: '2220', name: 'VAT należny', type: 'liability', normalBalance: 'credit' },
      { code: '2310', name: 'Rozrachunki z pracownikami', type: 'liability', normalBalance: 'credit' },
      { code: '2230', name: 'Rozliczenie VAT', type: 'liability', normalBalance: 'credit' },
      { code: '4000', name: 'Kapitał podstawowy', type: 'equity', normalBalance: 'credit' },
      { code: '4010', name: 'Kapitał zapasowy', type: 'equity', normalBalance: 'credit' },
      { code: '4020', name: 'Wynik finansowy lat ubiegłych', type: 'equity', normalBalance: 'credit' },
      { code: '4030', name: 'Wynik finansowy netto', type: 'equity', normalBalance: 'credit' },
      // 7000 first: postingDefaults picks the first income account
      { code: '7000', name: 'Przychody ze sprzedaży', type: 'income', normalBalance: 'credit' },
      { code: '7010', name: 'Pozostałe przychody', type: 'income', normalBalance: 'credit' },
      { code: '5000', name: 'Zużycie materiałów i energii', type: 'expense', normalBalance: 'debit' },
      { code: '5010', name: 'Wynagrodzenia', type: 'expense', normalBalance: 'debit' },
      { code: '5020', name: 'Amortyzacja', type: 'expense', normalBalance: 'debit' },
      { code: '5030', name: 'Usługi obce', type: 'expense', normalBalance: 'debit' },
      { code: '5040', name: 'Pozostałe koszty', type: 'expense', normalBalance: 'debit' },
      { code: '5050', name: 'Koszty finansowe', type: 'expense', normalBalance: 'debit' },
      { code: '5060', name: 'Podatek dochodowy', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '2010',
    bankAccountDefault: '1310',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Polish layout is a B-milestone
  },

  compliance: {
    // JPK_V7M monthly by the 25th of the following month (quarterly
    // JPK_V7K for small taxpayers); annual accounts approved within 6
    // months of FYE + filed with KRS within 15 days of approval; CIT-8
    // annual return 31 March of the following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'pl-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'pl-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'pl-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Poland is SEPA
    fxSource: 'ecb',
    baseCurrency: 'PLN',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (PL additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9945); KSeF
    // is a separate XML schema — B-milestone
    // auditFile omitted — JPK is a different schema (B-milestone)
    languages: ['pl'],
    defaultLanguage: 'pl',
  },

  closing: { resultAccount: '4030', equityAccount: '4020' },
};