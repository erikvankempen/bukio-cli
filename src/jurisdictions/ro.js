/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Romania jurisdiction profile (Phase F — RO profile).
//
// Data sources: research brief at docs-research/ro-profile.md — TVA 18/9
// (standard cut from 19% on 1 Jan 2026), VAT number RO + 2-10 digits
// (CUI/CIF), registration threshold RON 300K, Peppol EAS 9947 exists in the
// codelist but Romania does NOT participate in Peppol (e-Factura national
// system is the domestic route), D300 monthly by the 25th, annual accounts
// within 150 days of FYE (~30 May), D101 by 25 June, Planul de conturi
// general statutory chart (simplified skeleton; statutory VAT accounts
// 4424 recoverable / 4423 payable + settlement 4426).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (D300 return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Romanian
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Romanian SAF-T;
//                            e-Factura/e-SAF is a different schema)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Romanian additions
//                            are a B-milestone
//   - e-Factura             mandatory national e-invoicing (B2B since 2024)
//                            is a B-milestone; 'peppol-bis-3.0' registered
//                            for CROSS-BORDER invoices only — Romania has
//                            no national EAS scheme (not a Peppol
//                            participant), so the Peppol SEND path does not
//                            apply domestically
// Registered: SEPA, CAMT.053, ECB (RON base currency), closing 1211 -> 1171.
// New market: NO i18n table yet — documents render in English (languages
// ['en'], defaultLanguage 'en'), same treatment as GB/IE/US.

export default {
  meta: {
    country: 'RO',
    baseCurrency: 'RON',
    locale: 'ro',
    legalForms: ['srl', 'sa', 'snc', 'sca', 'pfa'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // CUI/CIF (cod unic de înregistrare), 2-10 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^RO\d{2,10}$/i, // RO + CUI (2-10 digits)
    // 9947 = Romania VAT number (EAS codelist) — cross-border reference;
    // Romania itself is NOT a Peppol participant
    peppolSchemeId: '9947',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 1800,
    // Small business: registration threshold RON 300,000 (≈ €60K)
    smallBusinessScheme: 'exemption-threshold',
    codes: [
      { code: '18', rateBp: 1800, type: 'standard', euReverse: 0, description: '18% taxa pe valoare adăugată' },
      { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% cotă redusă' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% export / livrări intracomunitare' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Operațiuni scutite' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'TVA inversat (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Achiziție intracomunitară' },
    ],
    accounts: {
      // Statutory VAT accounts: 4424 TVA de recuperat (input, asset) /
      // 4423 TVA de plată (output, liability); the settlement account 4426
      // absorbs the return balance + rounding differences
      ledger: [
        { code: '4424', name: 'TVA de recuperat', type: 'asset', normalBalance: 'debit' },
        { code: '4423', name: 'TVA de plată', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '4426',
      differenceDefault: '4426',
      settlementAccountName: 'TVA — decontare',
    },
    // returnLayout omitted — the D300 return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for small taxpayers
    reverseChargeEffectiveRateBp: 1800,
  },

  reporting: {
    // format omitted — Romanian financial-statements layout is a B-milestone
    taxonomy: null,
    // Planul de conturi general (statutory chart) — simplified skeleton, Romanian names
    defaultChart: [
      { code: '5121', name: 'Conturi la bănci', type: 'asset', normalBalance: 'debit' },
      { code: '5311', name: 'Casa', type: 'asset', normalBalance: 'debit' },
      { code: '4111', name: 'Clienți', type: 'asset', normalBalance: 'debit' },
      { code: '4424', name: 'TVA de recuperat', type: 'asset', normalBalance: 'debit' },
      { code: '2120', name: 'Mijloace fixe', type: 'asset', normalBalance: 'debit' },
      { code: '2810', name: 'Amortizare mijloace fixe', type: 'asset', normalBalance: 'credit' },
      { code: '3710', name: 'Mărfuri', type: 'asset', normalBalance: 'debit' },
      { code: '4011', name: 'Furnizori', type: 'liability', normalBalance: 'credit' },
      { code: '4423', name: 'TVA de plată', type: 'liability', normalBalance: 'credit' },
      { code: '4210', name: 'Personal — datorii', type: 'liability', normalBalance: 'credit' },
      { code: '4426', name: 'TVA — decontare', type: 'liability', normalBalance: 'credit' },
      { code: '1012', name: 'Capital subscris vărsat', type: 'equity', normalBalance: 'credit' },
      { code: '1171', name: 'Rezultatul reportat', type: 'equity', normalBalance: 'credit' },
      { code: '1211', name: 'Profit sau pierdere', type: 'equity', normalBalance: 'credit' },
      // 7010 first: postingDefaults picks the first income account
      { code: '7010', name: 'Venituri din vânzări', type: 'income', normalBalance: 'credit' },
      { code: '7080', name: 'Alte venituri', type: 'income', normalBalance: 'credit' },
      { code: '6010', name: 'Cheltuieli cu materiile prime', type: 'expense', normalBalance: 'debit' },
      { code: '6280', name: 'Alte cheltuieli', type: 'expense', normalBalance: 'debit' },
      { code: '6410', name: 'Cheltuieli cu salariile', type: 'expense', normalBalance: 'debit' },
      { code: '6811', name: 'Cheltuieli cu amortizarea', type: 'expense', normalBalance: 'debit' },
      { code: '6660', name: 'Cheltuieli cu dobânzile', type: 'expense', normalBalance: 'debit' },
      { code: '6910', name: 'Cheltuieli cu impozitul pe profit', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '4111',
    bankAccountDefault: '5121',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Romanian layout is a B-milestone
  },

  compliance: {
    // D300 monthly by the 25th of the following month (quarterly for small
    // taxpayers); annual accounts filed with the Ministry of Finance within
    // 150 days of FYE (~30 May); D101 annual CIT return + payment 25 June
    // of the following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'ro-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'ro-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'ro-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Romania is SEPA
    fxSource: 'ecb',
    baseCurrency: 'RON',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (RO additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border only (EAS 9947 reference);
    // e-Factura domestic e-invoicing is a B-milestone
    // auditFile omitted — e-SAF is a different schema (B-milestone)
    languages: ['en'],
    defaultLanguage: 'en',
  },

  closing: { resultAccount: '1211', equityAccount: '1171' },
};