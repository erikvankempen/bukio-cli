/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Sweden jurisdiction profile (Phase B — SE profile).
//
// Data sources: research brief at docs-research/se-profile.md — VAT
// rates/exemption from Skatteverket (official), momsredovisning
// frequencies/deadlines from Skatteverket, chart codes verified against
// BAS 2023 (bas.se + Visma/Spirís hb_bas2023.pdf + eDeklarera K2),
// identifiers from Commenda/EUIPO, e-invoice status from the EU
// Commission country sheet.
//
// Key corrections the research made to initial assumptions: Sweden DOES
// have a small-business VAT exemption — SEK 120,000 turnover (the
// SEK 80,000 figure is superseded; raised in 2025 per EU 2020/285);
// quarterly momsredovisning is due the 12th of the SECOND month after
// the period (not the 26th; August shifts to the 17th); the BAS 2023
// output-VAT accounts are 2611/2621/2631 by rate (not 2611/2612/2613)
// with a SINGLE input account 2641; the annual report is FILED within
// 7 months of FYE (prepared ≤ 6 + filed ≤ 1 month after adoption).
//
// Phase B scope discipline (same contract as LU/GB/FR/US/BE/DE/DK/FI/NO):
// register ONLY what maps to existing generic engines; everything else
// fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (Skatteverket
//                            momsredovisning engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail
//                            (årsredovisning K2/K3 layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Swedish SAF-T;
//                            the standardkonton/SIE export is a B-milestone)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (mervärdesskattelagen rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (B2G mandatory via Peppol —
// Peppol BIS Billing 3.0 EN 16931; B2B voluntary; Svefaktura is the
// legacy standard; scheme 0007), SEPA, CAMT.053, ECB, closing
// 2099 -> 2098.

export default {
  meta: {
    country: 'SE',
    baseCurrency: 'SEK',
    locale: 'sv-SE',
    legalForms: ['ab', 'enskild-firma', 'hb', 'kb', 'ekonomisk-forening'], // AB = SME standard
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // organisationsnummer: 10 digits (556677-8899)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^SE\d{12}$/i, // SE + org.nr without hyphen + '01' suffix
    peppolSchemeId: '0007', // Organisationsnummer (org.nr without hyphen)
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2500,
    // SME VAT exemption (EU 2020/285, raised 2025): annual turnover
    // ≤ SEK 120,000 -> automatically exempt from VAT registration
    // (the SEK 80,000 figure is superseded)
    smallBusinessScheme: 'franchise',
    // SE rates (Skatteverket): 25% standard / 12% (hotels, restaurants,
    // repairs) / 6% (books, culture, passenger transport) / 0% exempt.
    // From 1 Apr 2026 food sales (incl. takeaway) drop 12% -> 6%,
    // restaurant meals stay 12%. EU reverse charge applies.
    codes: [
      { code: '25', rateBp: 2500, type: 'standard', euReverse: 0, description: '25 % normalskattesats' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12 % reducerad skattesats' },
      { code: '6', rateBp: 600, type: 'standard', euReverse: 0, description: '6 % reducerad skattesats' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0 % momsfri' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Undantagen från moms' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Omvänd skattskyldighet (inrikes)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Omvänd skattskyldighet (EU)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Vinstmarginalbeskattning' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privat användning' },
    ],
    accounts: {
      // BAS 2023 VAT accounts (research §6): 2641 Debiterad ingående moms
      // (input — a debit-balance VAT position, engine type asset) /
      // 2611 Utgående moms 25 % (output, liability); the settlement
      // account 2650 Redovisningskonto för moms nets the return (box 49)
      ledger: [
        { code: '2641', name: 'Debiterad ingående moms', type: 'asset', normalBalance: 'debit' },
        { code: '2611', name: 'Utgående moms på försäljning inom Sverige, 25 %', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2650',
      differenceDefault: '2650',
      settlementAccountName: 'Redovisningskonto för moms',
    },
    // returnLayout omitted — the Skatteverket momsredovisning engine is a
    // B-milestone
    filingPeriodicity: 'quarterly', // monthly when taxable base > SEK 40M
    reverseChargeEffectiveRateBp: 2500,
  },

  reporting: {
    // format omitted — the årsredovisning K2/K3 layout is a B-milestone
    taxonomy: null,
    // BAS 2023 chart (research §5, verified against hb_bas2023.pdf + the
    // K2 eDeklarera chart). The 8999 P&L summary account is not a bukio
    // account type and is not seeded; the balance-sheet result account is
    // 2099 (closing -> 2098).
    defaultChart: [
      { code: '1210', name: 'Maskiner och andra tekniska anläggningar', type: 'asset', normalBalance: 'debit' },
      { code: '1220', name: 'Inventarier och verktyg', type: 'asset', normalBalance: 'debit' },
      { code: '1510', name: 'Kundfordringar', type: 'asset', normalBalance: 'debit' },
      { code: '1630', name: 'Avräkning för skatter och avgifter – skattekonto', type: 'asset', normalBalance: 'debit' },
      { code: '1650', name: 'Momsfordran', type: 'asset', normalBalance: 'debit' },
      { code: '1910', name: 'Kassa', type: 'asset', normalBalance: 'debit' },
      { code: '1930', name: 'Företagskonto/checkkonto/affärskonto', type: 'asset', normalBalance: 'debit' },
      { code: '2081', name: 'Aktiekapital', type: 'equity', normalBalance: 'credit' },
      { code: '2091', name: 'Balanserad vinst eller förlust', type: 'equity', normalBalance: 'credit' },
      { code: '2098', name: 'Vinst eller förlust från föregående år', type: 'equity', normalBalance: 'credit' },
      { code: '2099', name: 'Årets resultat', type: 'equity', normalBalance: 'credit' },
      { code: '2420', name: 'Förskott från kunder', type: 'liability', normalBalance: 'credit' },
      { code: '2440', name: 'Leverantörsskulder', type: 'liability', normalBalance: 'credit' },
      { code: '2510', name: 'Skatteskulder', type: 'liability', normalBalance: 'credit' },
      { code: '2611', name: 'Utgående moms på försäljning inom Sverige, 25 %', type: 'liability', normalBalance: 'credit' },
      { code: '2621', name: 'Utgående moms på försäljning inom Sverige, 12 %', type: 'liability', normalBalance: 'credit' },
      { code: '2631', name: 'Utgående moms på försäljning inom Sverige, 6 %', type: 'liability', normalBalance: 'credit' },
      { code: '2641', name: 'Debiterad ingående moms', type: 'asset', normalBalance: 'debit' },
      { code: '2650', name: 'Redovisningskonto för moms', type: 'liability', normalBalance: 'credit' },
      { code: '2710', name: 'Personalskatt', type: 'liability', normalBalance: 'credit' },
      { code: '2730', name: 'Lagstadgade sociala avgifter o särskild löneskatt', type: 'liability', normalBalance: 'credit' },
      { code: '2850', name: 'Avräkning för skatter och avgifter – skattekonto', type: 'liability', normalBalance: 'credit' },
      { code: '2990', name: 'Övriga upplupna kostnader o förutbetalda intäkter', type: 'liability', normalBalance: 'credit' },
      { code: '3001', name: 'Försäljning inom Sverige, 25 % moms', type: 'income', normalBalance: 'credit' },
      { code: '3002', name: 'Försäljning inom Sverige, 12 % moms', type: 'income', normalBalance: 'credit' },
      { code: '3003', name: 'Försäljning inom Sverige, 6 % moms', type: 'income', normalBalance: 'credit' },
      { code: '3041', name: 'Försäljning tjänster 25 % moms Sv', type: 'income', normalBalance: 'credit' },
      { code: '3042', name: 'Försäljning tjänster 12 % moms Sv', type: 'income', normalBalance: 'credit' },
      { code: '3043', name: 'Försäljning tjänster 6 % moms Sv', type: 'income', normalBalance: 'credit' },
      { code: '4010', name: 'Inköp av varor och material', type: 'expense', normalBalance: 'debit' },
      { code: '5010', name: 'Lokalhyra', type: 'expense', normalBalance: 'debit' },
      { code: '5060', name: 'Städning och renhållning', type: 'expense', normalBalance: 'debit' },
      { code: '5410', name: 'Förbrukningsinventarier', type: 'expense', normalBalance: 'debit' },
      { code: '6100', name: 'Kontorsmateriel och trycksaker', type: 'expense', normalBalance: 'debit' },
      { code: '6200', name: 'Tele och post', type: 'expense', normalBalance: 'debit' },
      { code: '6310', name: 'Företagsförsäkringar', type: 'expense', normalBalance: 'debit' },
      { code: '6540', name: 'IT-tjänster', type: 'expense', normalBalance: 'debit' },
      { code: '6570', name: 'Bankkostnader', type: 'expense', normalBalance: 'debit' },
      { code: '6590', name: 'Övriga externa tjänster', type: 'expense', normalBalance: 'debit' },
      { code: '6990', name: 'Övriga externa kostnader', type: 'expense', normalBalance: 'debit' },
      { code: '7010', name: 'Löner till kollektivanställda', type: 'expense', normalBalance: 'debit' },
      { code: '7210', name: 'Löner till tjänstemän', type: 'expense', normalBalance: 'debit' },
      { code: '7511', name: 'Lagstadgade sociala avgifter för löner och ersättningar', type: 'expense', normalBalance: 'debit' },
      { code: '8400', name: 'Räntekostnader', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (1510 Kundfordringar)
    debtorsAccount: '1510',
    bankAccountDefault: '1930',
    inferTaxonomy: null,
    // statutoryAccounts omitted — årsredovisning layout is a B-milestone
  },

  compliance: {
    // research §10 (Skatteverket): momsredovisning quarterly for the
    // SME band (≤ SEK 40M taxable base), due the 12th of the SECOND month
    // after the period (August shifts to the 17th); annual report filed
    // with Bolagsverket within 7 months of FYE (prepared ≤ 6 + filed
    // ≤ 1 month after adoption)
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Qn', deadlineRule: 'se-quarterly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'se-7-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'], // CAMT.053 confirmed for Handelsbanken, standard for the majors
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // SE is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'SEK',
  },

  documents: {
    // invoiceCompliance omitted — mervärdesskattelagen rule set is a
    // B-milestone
    eInvoicing: 'peppol-bis-3.0', // B2G mandatory via Peppol (Peppol BIS
    // Billing 3.0 EN 16931; SFS 2018:1277); B2B voluntary; Svefaktura is
    // the legacy standard
    // auditFile omitted — no Swedish SAF-T; SIE export is a B-milestone
    languages: ['sv', 'en'],
    defaultLanguage: 'sv',
  },

  closing: { resultAccount: '2099', equityAccount: '2098' },
};
