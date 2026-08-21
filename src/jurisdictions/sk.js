/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Slovakia jurisdiction profile (Phase F — SK profile).
//
// Data sources: research brief at docs-research/sk-profile.md — DPH 23/19/5
// (standard raised to 23% on 1 Jan 2025), VAT number SK + 10 digits, IČO
// company number, registration threshold €49,790, Peppol EAS 9950 (official
// EAS codelist, release 8 Dec 2025), VAT return monthly/quarterly by the
// 25th, annual accounts within 6 months of FYE, CIT return 31 March (6-
// month extension with fee), směrná účtová osnova statutory framework
// chart (simplified skeleton; VAT account 343 split into 3431/3432/3433).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (DPH return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Slovak
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Slovak SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Slovak additions are
//                            a B-milestone
//   - national e-invoicing  2027 B2B mandate in preparation (Slovakia
//                            became a Peppol Authority in Mar 2026) —
//                            'peppol-bis-3.0' registered for cross-border
//                            (SK is a Peppol participant, EAS 9950)
// Registered: SEPA, CAMT.053, ECB, closing 4310 -> 4210.
// Documents render in Slovak (languages ['sk'], defaultLanguage
// 'sk') — full i18n table since 16 Aug 2026.

export default {
  meta: {
    country: 'SK',
    baseCurrency: 'EUR',
    locale: 'sk',
    legalForms: ['sro', 'as', 'szco', 'ks'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // IČO (identifikačné číslo organizácie), 8 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^SK\d{10}$/i, // SK + 10 digits
    // 9950 = Slovakia VAT number (EAS codelist)
    peppolSchemeId: '9950',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2300,
    // Small business: registration threshold €49,790
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '23', rateBp: 2300, type: 'standard', euReverse: 0, description: '23% daň z pridanej hodnoty' },
      { code: '19', rateBp: 1900, type: 'standard', euReverse: 0, description: '19% znížená sadzba' },
      { code: '5', rateBp: 500, type: 'standard', euReverse: 0, description: '5% znížená sadzba' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% vývoz / intra-EU dodanie' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Oslobodené dodania' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Prenesenie daňovej povinnosti (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU nadobudnutie' },
    ],
    accounts: {
      // směrná účtová osnova: the statutory VAT account 343 split into
      // 3431 (input, asset) / 3432 (output, liability) with 3433 as the
      // settlement account absorbing the return balance + rounding
      ledger: [
        { code: '3431', name: 'DPH na vstupe', type: 'asset', normalBalance: 'debit' },
        { code: '3432', name: 'DPH na výstupe', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '3433',
      differenceDefault: '3433',
      settlementAccountName: 'DPH — zúčtovanie',
    },
    // returnLayout omitted — the DPH return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for small taxpayers
    reverseChargeEffectiveRateBp: 2300,
  },

  reporting: {
    // format omitted — Slovak financial-statements layout is a B-milestone
    taxonomy: null,
    // směrná účtová osnova (statutory framework) — simplified skeleton, Slovak names
    defaultChart: [
      { code: '2210', name: 'Bankové účty', type: 'asset', normalBalance: 'debit' },
      { code: '2110', name: 'Pokladnica', type: 'asset', normalBalance: 'debit' },
      { code: '3110', name: 'Odberatelia', type: 'asset', normalBalance: 'debit' },
      { code: '3431', name: 'DPH na vstupe', type: 'asset', normalBalance: 'debit' },
      { code: '0220', name: 'Dlhodobý hmotný majetok', type: 'asset', normalBalance: 'debit' },
      { code: '0820', name: 'Oprávky k DHM', type: 'asset', normalBalance: 'credit' },
      { code: '1320', name: 'Tovar na sklade', type: 'asset', normalBalance: 'debit' },
      { code: '3210', name: 'Dodávatelia', type: 'liability', normalBalance: 'credit' },
      { code: '3432', name: 'DPH na výstupe', type: 'liability', normalBalance: 'credit' },
      { code: '3310', name: 'Zamestnanci', type: 'liability', normalBalance: 'credit' },
      { code: '3433', name: 'DPH — zúčtovanie', type: 'liability', normalBalance: 'credit' },
      { code: '4110', name: 'Základné imanie', type: 'equity', normalBalance: 'credit' },
      { code: '4210', name: 'Nerozdelený zisk', type: 'equity', normalBalance: 'credit' },
      { code: '4310', name: 'Výsledok hospodárenia bežného obdobia', type: 'equity', normalBalance: 'credit' },
      // 6010 first: postingDefaults picks the first income account
      { code: '6010', name: 'Tržby za vlastné výrobky a služby', type: 'income', normalBalance: 'credit' },
      { code: '6110', name: 'Ostatné výnosy', type: 'income', normalBalance: 'credit' },
      { code: '5010', name: 'Spotreba materiálu', type: 'expense', normalBalance: 'debit' },
      { code: '5180', name: 'Služby', type: 'expense', normalBalance: 'debit' },
      { code: '5210', name: 'Mzdové náklady', type: 'expense', normalBalance: 'debit' },
      { code: '5510', name: 'Odpisy', type: 'expense', normalBalance: 'debit' },
      { code: '5680', name: 'Finančné náklady', type: 'expense', normalBalance: 'debit' },
      { code: '5910', name: 'Daň z príjmov', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '3110',
    bankAccountDefault: '2210',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Slovak layout is a B-milestone
  },

  compliance: {
    // Monthly/quarterly DPH return due the 25th of the following month;
    // annual accounts within 6 months of FYE; CIT return 31 March of the
    // following year (6-month extension to 30 June with fee).
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'sk-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'sk-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'sk-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Slovakia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (SK additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9950); the
    // 2027 domestic B2B mandate is a B-milestone
    // auditFile omitted — no Slovak SAF-T
    languages: ['sk'],
    defaultLanguage: 'sk',
  },

  closing: { resultAccount: '4310', equityAccount: '4210' },
};