/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across twenty-four jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Slovenia jurisdiction profile (Phase E — SI profile).
//
// Data sources: research brief at docs-research/si-profile.md — DDV 22/9.5/5
// (standard raised to 22% on 1 Jan 2025), VAT number SI + 8 digits (davčna
// številka), registration threshold €60,000, Peppol EAS 9949 (official EAS
// codelist, release 8 Dec 2025), monthly DDV-O by the 20th (quarterly for
// small filers + annual DDV-O 31 March), annual report to AJPES within 8
// months of FYE (31 August), DDPO (CIT) by 31 March, SRS 30 standardised
// kontni načrt chart (simplified skeleton).
//
// Phase E scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (DDV-O return
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (AJPES
//                            letno poročilo is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Slovenian SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Slovenian additions
//                            are a B-milestone
//   - national e-invoicing  no domestic XML mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (SI is a Peppol
//                            participant, EAS 9949)
// Registered: SEPA, CAMT.053, ECB, closing 2200 -> 2100.
// Documents render in Slovenian (languages ['sl'], defaultLanguage
// 'sl') — full i18n table since 16 Aug 2026.

export default {
  meta: {
    country: 'SI',
    baseCurrency: 'EUR',
    locale: 'sl',
    legalForms: ['doo', 'dd', 'sp', 'kd'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // DŠ / davčna številka (tax number), 8 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^SI\d{8}$/i, // SI + davčna številka (8 digits)
    // 9949 = Slovenia VAT number (EAS codelist)
    peppolSchemeId: '9949',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2200,
    // Small business: registration threshold €60,000
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '22', rateBp: 2200, type: 'standard', euReverse: 0, description: '22% davek na dodano vrednost' },
      { code: '9.5', rateBp: 950, type: 'standard', euReverse: 0, description: '9,5% znižana stopnja' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% znižana stopnja' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% izvoz / intra-EU dobave' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Oproščene dobave' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Obrnjeno davčno breme (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU pridobitev' },
    ],
    accounts: {
      // SRS 30 standardised kontni načrt: 1500 Vstopni DDV (input, asset) /
      // 1510 Izstopni DDV (output, liability); the settlement account 1520
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '1500', name: 'Vstopni DDV', type: 'asset', normalBalance: 'debit' },
        { code: '1510', name: 'Izstopni DDV', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1520',
      differenceDefault: '1520',
      settlementAccountName: 'DDV — obračun',
    },
    // returnLayout omitted — the DDV-O return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly for small filers (annual DDV-O 31 Mar)
    reverseChargeEffectiveRateBp: 2200,
  },

  reporting: {
    // format omitted — AJPES letno poročilo is a B-milestone
    taxonomy: null,
    // SRS 30 standardised kontni načrt (simplified skeleton), Slovenian names
    defaultChart: [
      { code: '1000', name: 'Poslovni račun', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Blagajna', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Kratkoročne terjatve do kupcev', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Vstopni DDV', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Opredmetena osnovna sredstva', type: 'asset', normalBalance: 'debit' },
      { code: '1810', name: 'Popravek vrednosti OOS', type: 'asset', normalBalance: 'credit' },
      { code: '1900', name: 'Zaloge', type: 'asset', normalBalance: 'debit' },
      { code: '1300', name: 'Kratkoročne obveznosti do dobaviteljev', type: 'liability', normalBalance: 'credit' },
      { code: '1510', name: 'Izstopni DDV', type: 'liability', normalBalance: 'credit' },
      { code: '1400', name: 'Obveznosti do zaposlencev', type: 'liability', normalBalance: 'credit' },
      { code: '1520', name: 'DDV — obračun', type: 'liability', normalBalance: 'credit' },
      { code: '2000', name: 'Osnovni kapital', type: 'equity', normalBalance: 'credit' },
      { code: '2100', name: 'Preneseni dobiček', type: 'equity', normalBalance: 'credit' },
      { code: '2200', name: 'Čisti dobiček iz poslovnega izida', type: 'equity', normalBalance: 'credit' },
      // 3000 first: postingDefaults picks the first income account —
      // the standard sales account
      { code: '3000', name: 'Prihodki od prodaje', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Drugi prihodki', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Stroški blaga in materiala', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Stroški storitev', type: 'expense', normalBalance: 'debit' },
      { code: '4200', name: 'Stroški dela', type: 'expense', normalBalance: 'debit' },
      { code: '4300', name: 'Amortizacija', type: 'expense', normalBalance: 'debit' },
      { code: '4400', name: 'Finančni odhodki', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Davek od dohodkov pravnih oseb', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '1200',
    bankAccountDefault: '1000',
    inferTaxonomy: null,
    // statutoryAccounts omitted — AJPES layout is a B-milestone
  },

  compliance: {
    // Monthly DDV-O due the 20th of the following month (quarterly for
    // small filers with an annual DDV-O 31 March); annual report to AJPES
    // within 8 months of FYE (31 August); DDPO (CIT) by 31 March.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'si-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'si-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'si-ddpo' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Slovenia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (SI additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9949); no
    // domestic XML mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Slovenian SAF-T
    languages: ['sl'],
    defaultLanguage: 'sl',
  },

  closing: { resultAccount: '2200', equityAccount: '2100' },
};