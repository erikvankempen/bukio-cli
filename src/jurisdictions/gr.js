/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Greece jurisdiction profile (Phase F — GR profile).
//
// Data sources: research brief at docs-research/gr-profile.md — ΦΠΑ 24/13/6,
// VAT number EL + 9 digits (Greece uses the EL prefix in VIES), AFM tax
// number / GEMI registration, small-business threshold €10,000, Peppol EAS
// 9933 (official EAS codelist, release 8 Dec 2025), VAT return monthly by
// the 26th (quarterly by the 30th of the month after the quarter), annual
// accounts within 10 months of FYE (GEMI), CIT return 30 June, ΕΓΛΣ
// statutory chart (simplified skeleton).
//
// Phase F scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (ΦΠΑ return engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (Greek
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Greek SAF-T)
//   - documents.invoiceCompliance → the art. 226 EU baseline is registered
//                            ('eu-invoice-vereisten'); Greek additions are
//                            a B-milestone
//   - myDATA                mandatory digital books/reporting (AADE) is a
//                            B-milestone; 'peppol-bis-3.0' registered for
//                            cross-border (GR is a Peppol participant,
//                            EAS 9933)
// Registered: SEPA, CAMT.053, ECB, closing 4300 -> 4200.
// Documents render in Greek (languages ['el'], defaultLanguage
// 'el') — full i18n table since 16 Aug 2026.

export default {
  meta: {
    country: 'GR',
    baseCurrency: 'EUR',
    locale: 'el',
    legalForms: ['ike', 'epe', 'ae', 'oe', 'atomi'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // GEMI registration number (Γ.Ε.ΜΗ.)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^EL\d{9}$/i, // EL + AFM (9 digits) — EL is the VIES prefix
    // 9933 = Greece VAT number (EAS codelist)
    peppolSchemeId: '9933',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2400,
    // Small-scale exemption (art. 39b Greek VAT Code): €10,000
    smallBusinessScheme: 'small-scale-exemption',
    codes: [
      { code: '24', rateBp: 2400, type: 'standard', euReverse: 0, description: '24% ΦΠΑ' },
      { code: '13', rateBp: 1300, type: 'standard', euReverse: 0, description: '13% μειωμένος συντελεστής' },
      { code: '6', rateBp: 600, type: 'standard', euReverse: 0, description: '6% υπερμειωμένος συντελεστής' },
      { code: '0', rateBp: 0, type: 'exempt', euReverse: 0, description: '0% εξαγωγές / ενδοκοινοτικές παραδόσεις' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Απαλλασσόμενες παραδόσεις' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Αντίστροφη χρέωση (reverse charge)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Ενδοκοινοτική απόκτηση' },
    ],
    accounts: {
      // ΕΓΛΣ structure: 5400 ΦΠΑ εισροών (input, asset) / 5450 ΦΠΑ εκροών
      // (output, liability); the settlement account 5403 absorbs the
      // return balance + rounding differences
      ledger: [
        { code: '5400', name: 'ΦΠΑ εισροών', type: 'asset', normalBalance: 'debit' },
        { code: '5450', name: 'ΦΠΑ εκροών', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '5403',
      differenceDefault: '5403',
      settlementAccountName: 'ΦΠΑ — εκκαθάριση',
    },
    // returnLayout omitted — the ΦΠΑ return engine is a B-milestone
    filingPeriodicity: 'monthly', // quarterly option for small filers (30th of the month after the quarter)
    reverseChargeEffectiveRateBp: 2400,
  },

  reporting: {
    // format omitted — Greek financial-statements layout is a B-milestone
    taxonomy: null,
    // ΕΓΛΣ (statutory chart, PD 1123/1980 structure) — simplified skeleton,
    // Greek names
    defaultChart: [
      { code: '3802', name: 'Τράπεζες', type: 'asset', normalBalance: 'debit' },
      { code: '3800', name: 'Ταμείο', type: 'asset', normalBalance: 'debit' },
      { code: '3000', name: 'Πελάτες', type: 'asset', normalBalance: 'debit' },
      { code: '5400', name: 'ΦΠΑ εισροών', type: 'asset', normalBalance: 'debit' },
      { code: '1000', name: 'Ενσώματα πάγια', type: 'asset', normalBalance: 'debit' },
      { code: '1100', name: 'Αποσβεσμένα πάγια', type: 'asset', normalBalance: 'credit' },
      { code: '2000', name: 'Αποθέματα', type: 'asset', normalBalance: 'debit' },
      { code: '5000', name: 'Προμηθευτές', type: 'liability', normalBalance: 'credit' },
      { code: '5450', name: 'ΦΠΑ εκροών', type: 'liability', normalBalance: 'credit' },
      { code: '5300', name: 'Αμοιβές προσωπικού πληρωτέες', type: 'liability', normalBalance: 'credit' },
      { code: '5403', name: 'ΦΠΑ — εκκαθάριση', type: 'liability', normalBalance: 'credit' },
      { code: '4000', name: 'Κεφάλαιο', type: 'equity', normalBalance: 'credit' },
      { code: '4200', name: 'Αποτελέσματα εις νέον', type: 'equity', normalBalance: 'credit' },
      { code: '4300', name: 'Αποτέλεσμα χρήσης', type: 'equity', normalBalance: 'credit' },
      // 8000 first: postingDefaults picks the first income account
      { code: '8000', name: 'Πωλήσεις', type: 'income', normalBalance: 'credit' },
      { code: '8100', name: 'Λοιπά έσοδα', type: 'income', normalBalance: 'credit' },
      { code: '6000', name: 'Αγορές εμπορευμάτων', type: 'expense', normalBalance: 'debit' },
      { code: '6100', name: 'Αμοιβές προσωπικού', type: 'expense', normalBalance: 'debit' },
      { code: '6200', name: 'Παροχές τρίτων', type: 'expense', normalBalance: 'debit' },
      { code: '6600', name: 'Αποσβέσεις', type: 'expense', normalBalance: 'debit' },
      { code: '6500', name: 'Λοιπά έξοδα', type: 'expense', normalBalance: 'debit' },
      { code: '6700', name: 'Χρηματοοικονομικά έξοδα', type: 'expense', normalBalance: 'debit' },
      { code: '6900', name: 'Φόρος εισοδήματος', type: 'expense', normalBalance: 'debit' },
    ],
    debtorsAccount: '3000',
    bankAccountDefault: '3802',
    inferTaxonomy: null,
    // statutoryAccounts omitted — Greek layout is a B-milestone
  },

  compliance: {
    // Monthly ΦΠΑ return due the 26th of the following month (quarterly
    // filers: 30th of the month after the quarter); annual accounts
    // published/filed (GEMI) within 10 months of FYE; CIT return (ΝΠΟ)
    // 30 June of the following year.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-MM', deadlineRule: 'gr-vat-monthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'gr-annual-accounts' },
      { type: 'CIT', periodShape: 'YYYY', deadlineRule: 'gr-cit' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Greece is SEPA
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (GR additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9933); myDATA
    // digital reporting is a B-milestone
    // auditFile omitted — no Greek SAF-T
    languages: ['el'],
    defaultLanguage: 'el',
  },

  closing: { resultAccount: '4300', equityAccount: '4200' },
};