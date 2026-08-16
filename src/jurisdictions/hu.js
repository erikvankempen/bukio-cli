/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Hungary jurisdiction profile (Phase F — HU profile).
//
// Data sources: research brief at docs-research/hu-profile.md — ÁFA 27/18/5
// (27% is the EU's highest), VAT number HU + 8 digits (adószám), company
// Cégjegyzék number, registration threshold HUF 12M, Peppol EAS 9910
// (official EAS codelist, release 8 Dec 2025), VAT return monthly by the
// 20th (quarterly/annual options by turnover), annual report within 5
// months of FYE, TAO (CIT) by 31 May, Számviteli törvény statutory chart
// (simplified skeleton; statutory VAT accounts 466/467 + settlement 468).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (ÁFA return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (beszámoló
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Hungarian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Hungarian additions
//                            are a B-milestone
//   - RTIR                  real-time invoice reporting 3.0 (NAV) is
//                            mandatory and is a B-milestone; 'peppol-bis-3.0'
//                            registered for cross-border (HU is a Peppol
//                            participant, EAS 9910)
// Registered: SEPA, CAMT.053, ECB (HUF base currency), closing 4190 -> 4130.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'HU',
    baseCurrency: 'HUF',
    locale: 'hu',
    legalForms: ['kft', 'zrt', 'nyrt', 'bt', 'egyeni'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Cégjegyzék number (11 chars)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^HU\d{8}$/i, // HU + adószám (8 digits)
    // 9910 = Hungary VAT number (EAS codelist)
    peppolSchemeId: '9910',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2700,
    // Small business: registration threshold HUF 12,000,000 (≈ €30K)
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '27', rateBp: 2700, type: 'standard', euReverse: 0, description: '27% általános forgalmi adó' },
      { code: '18', rateBp: 1800, type: 'standard', euReverse: 0, description: '18% kedvezményes kulcs' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% kedvezményes kulcs' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% export / EU-n belüli értékesítés' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Adómentes értékesítés' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Fordított adózás (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'EU-n belüli beszerzés' },
    ],
    accounts: {
      // Szt. statutory VAT accounts: 4660 Előzetesen felszámított áfa
      // (input, asset) / 4670 Fizetendő áfa (output, liability); the
      // settlement account 4680 absorbs the return balance + rounding
      ledger: [
        { code: '4660', name: 'Előzetesen felszámított áfa', type: 'asset', normalBalance: 'debit' },
        { code: '4670', name: 'Fizetendő áfa', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '4680',
      differenceDefault: '4680',
      settlementAccountName: 'ÁFA elszámolás',
    },
    // returnLayout omitted — the ÁFA return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly/annual options by turnover
    reverseChargeEffectiveRateBp: 2700,
  },

  reporting: {
    // format omitted — beszámoló layout is a B-milestone
    taxonomy: null,
    // Számviteli törvény (statutory chart) — simplified skeleton, Hungarian names
    defaultChart: [
      { code: '3810', name: 'Bank', type: 'asset', normalBalance: 'debit' },
      { code: '3840', name: 'Pénztár', type: 'asset', normalBalance: 'debit' },
      { code: '3110', name: 'Vevők', type: 'asset', normalBalance: 'debit' },
      { code: '4660', name: 'Előzetesen felszámított áfa', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Tárgyi eszközök', type: 'asset', normalBalance: 'debit' },
      { code: '1390', name: 'Tárgyi eszközök értékcsökkenése', type: 'asset', normalBalance: 'credit' },
      { code: '2100', name: 'Készletek', type: 'asset', normalBalance: 'debit' },
      { code: '4540', name: 'Szállítók', type: 'liability', normalBalance: 'credit' },
      { code: '4670', name: 'Fizetendő áfa', type: 'liability', normalBalance: 'credit' },
      { code: '4710', name: 'Személyi jellegű kötelezettségek', type: 'liability', normalBalance: 'credit' },
      { code: '4680', name: 'ÁFA elszámolás', type: 'liability', normalBalance: 'credit' },
      { code: '4110', name: 'Jegyzett tőke', type: 'equity', normalBalance: 'credit' },
      { code: '4130', name: 'Eredménytartalék', type: 'equity', normalBalance: 'credit' },
      { code: '4190', name: 'Mérleg szerinti eredmény', type: 'equity', normalBalance: 'credit' },
      // 9110 first: postingDefaults picks the first income account
      { code: '9110', name: 'Értékesítés nettó árbevétele', type: 'income', normalBalance: 'credit' },
      { code: '9210', name: 'Egyéb bevételek', type: 'income', normalBalance: 'credit' },
      { code: '5100', name: 'Anyagköltség', type: 'expense', normalBalance: 'debit' },
      { code: '5200', name: 'Igénybe vett szolgáltatások', type: 'expense', normalBalance: 'debit' },
      { code: '5400', name: 'Bérköltség', type: 'expense', normalBalance: 'debit' },
      { code: '5600', name: 'Értékcsökkenési leírás', type: 'expense', normalBalance: 'debit' },
      { code: '5700', name: 'Egyéb költségek', type: 'expense', normalBalance: 'debit' },
      { code: '5800', name: 'Pénzügyi ráfordítások', type: 'expense', normalBalance: 'debit' },
      { code: '5900', name: 'Társasági adó', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '3110',
    bankAccountDefault: '3810',
    inferTaxonomy: null,
    // statutoryAccounts omitted — beszámoló layout is a B-milestone
  },

  compliance: {
    // Monthly ÁFA return due the 20th of the following month (quarterly
    // by the 20th of the month after the quarter; annual below HUF 8M);
    // annual report (beszámoló) within 5 months of FYE (31 May); TAO
    // (CIT) annual return 31 May.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'hu-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'hu-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'hu-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Hungary is SEPA
    fxSource: 'ecb',
    baseCurrency: 'HUF',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (HU additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9910); RTIR
    // real-time reporting (NAV) is a B-milestone
    // auditFile omitted — no Hungarian SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '4190', equityAccount: '4130' },
};