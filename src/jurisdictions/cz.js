/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Czechia jurisdiction profile (Phase F — CZ profile).
//
// Data sources: research brief at docs-research/cz-profile.md — DPH 21/12
// (single reduced rate since 2024), VAT number CZ + 8-10 digits, IČO
// company number, Peppol EAS 9929 (official EAS codelist, release 8 Dec
// 2025), VAT return monthly/quarterly by the 25th, annual accounts ~6
// months (approval) + 30 days (filing), CIT return within 3 months (31
// March; 1 July for audited companies), směrná účtová osnova statutory
// framework chart (simplified skeleton; the VAT account 343 split into
// 3431/3432/3433 for the engine pair).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (DPH return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Czech
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Czech SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Czech additions are
//                            a B-milestone
//   - national e-invoicing  no domestic XML mandate yet — 'peppol-bis-3.0'
//                            registered for cross-border (CZ is a Peppol
//                            participant, EAS 9929)
// Registered: SEPA, CAMT.053, ECB (CZK base currency), closing 4310 -> 4210.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'CZ',
    baseCurrency: 'CZK',
    locale: 'cz',
    legalForms: ['sro', 'as', 'osvc', 'ks'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // IČO (identifikační číslo osoby), 8 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^CZ\d{8,10}$/i, // CZ + 8 (legal entities) / 9-10 (entrepreneurs)
    // 9929 = Czech Republic VAT number (EAS codelist)
    peppolSchemeId: '9929',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2100,
    // Small business: registration threshold CZK 2,000,000 (≈ €80K)
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% daň z přidané hodnoty' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12% snížená sazba' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% vývoz / intra-EU dodání' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Osvobozená plnění' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Přenesení daňové povinnosti (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Intra-EU pořízení' },
    ],
    accounts: {
      // směrná účtová osnova: the statutory VAT account 343 split into
      // 3431 (input, asset) / 3432 (output, liability) with 3433 as the
      // settlement account absorbing the return balance + rounding
      ledger: [
        { code: '3431', name: 'DPH na vstupu', type: 'asset', normalBalance: 'debit' },
        { code: '3432', name: 'DPH na výstupu', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '3433',
      differenceDefault: '3433',
      settlementAccountName: 'DPH — zúčtování',
    },
    // returnLayout omitted — the DPH return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for small taxpayers
    reverseChargeEffectiveRateBp: 2100,
  },

  reporting: {
    // format omitted — Czech financial-statements layout is a B-milestone
    taxonomy: null,
    // směrná účtová osnova (statutory framework) — simplified skeleton, Czech names
    defaultChart: [
      { code: '2210', name: 'Bankovní účty', type: 'asset', normalBalance: 'debit' },
      { code: '2110', name: 'Pokladna', type: 'asset', normalBalance: 'debit' },
      { code: '3110', name: 'Odběratelé', type: 'asset', normalBalance: 'debit' },
      { code: '3431', name: 'DPH na vstupu', type: 'asset', normalBalance: 'debit' },
      { code: '0220', name: 'Dlouhodobý hmotný majetek', type: 'asset', normalBalance: 'debit' },
      { code: '0820', name: 'Oprávky k DHM', type: 'asset', normalBalance: 'credit' },
      { code: '1320', name: 'Zboží na skladě', type: 'asset', normalBalance: 'debit' },
      { code: '3210', name: 'Dodavatelé', type: 'liability', normalBalance: 'credit' },
      { code: '3432', name: 'DPH na výstupu', type: 'liability', normalBalance: 'credit' },
      { code: '3310', name: 'Zaměstnanci', type: 'liability', normalBalance: 'credit' },
      { code: '3433', name: 'DPH — zúčtování', type: 'liability', normalBalance: 'credit' },
      { code: '4110', name: 'Základní kapitál', type: 'equity', normalBalance: 'credit' },
      { code: '4210', name: 'Nerozdělený zisk', type: 'equity', normalBalance: 'credit' },
      { code: '4310', name: 'Výsledek hospodaření běžného období', type: 'equity', normalBalance: 'credit' },
      // 6010 first: postingDefaults picks the first income account
      { code: '6010', name: 'Tržby za vlastní výrobky a služby', type: 'income', normalBalance: 'credit' },
      { code: '6110', name: 'Ostatní výnosy', type: 'income', normalBalance: 'credit' },
      { code: '5010', name: 'Spotřeba materiálu', type: 'expense', normalBalance: 'debit' },
      { code: '5180', name: 'Služby', type: 'expense', normalBalance: 'debit' },
      { code: '5210', name: 'Mzdové náklady', type: 'expense', normalBalance: 'debit' },
      { code: '5510', name: 'Odpisy', type: 'expense', normalBalance: 'debit' },
      { code: '5680', name: 'Finanční náklady', type: 'expense', normalBalance: 'debit' },
      { code: '5910', name: 'Daň z příjmů', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '3110',
    bankAccountDefault: '2210',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Czech layout is a B-milestone
  },

  compliance: {
    // Monthly/quarterly DPH return due the 25th of the following month;
    // annual accounts approved within 6 months of FYE + filed within 30
    // days of approval; CIT return within 3 months of FYE (31 March;
    // 1 July for audited companies).
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'cz-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'cz-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'cz-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Czechia is SEPA
    fxSource: 'ecb',
    baseCurrency: 'CZK',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (CZ additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9929); no
    // domestic XML mandate yet — national e-invoicing is a B-milestone
    // auditFile omitted — no Czech SAF-T
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '4310', equityAccount: '4210' },
};