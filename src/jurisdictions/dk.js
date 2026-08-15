/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Denmark jurisdiction profile (Phase B — DK profile).
//
// Data sources: research brief at docs-research/dk-profile.md — VAT
// facts/deadlines verbatim from the official SKAT deadlines page, CVR
// identifiers + Peppol scheme 0184 from OECD/Peppol sources, annual-report
// deadline (class B: 5 months) from 4+ advisory sources, the official
// Erhvervsstyrelsen Standardkontoplan (JSON on git.erst.dk, in force
// 1 Jan 2026) as the structural anchor for the chart.
//
// Key corrections the research made to initial assumptions: "monthly VAT
// mandatory since 2022 for all" is FALSE — frequency is turnover-tiered
// (half-yearly < DKK 5M, quarterly 5-50M/newly registered, monthly
// > DKK 50M; deadlines 1 Sep/1 Mar, 1st of the 3rd following month, 25th
// of the following month); Denmark has NO reduced rate band (25% only)
// and NO small-business exemption scheme; the exact e-conomic/Dinero
// chart codes are NOT publicly verifiable (login-gated CSVs) — the chart
// below is the brief's draft convention aligned with the OFFICIAL
// Standardkontoplan structure (1xxx assets / 2xxx liabilities / 3xxx
// equity / 4xxx income / 5xxx costs), MED confidence, flagged.
//
// Phase B scope discipline (same contract as LU/GB/FR/US/BE/DE): register
// ONLY what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (TastSelv
//                            momsangivelse engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (årsrapport
//                            skema 1 & 3 layout is a B-milestone; the
//                            official Standardkontoplan is schedule-aligned)
//   - documents.auditFile   omitted → XAF export fails (DK moves to SAF-T
//                            v2.0 by 1 Jan 2027 — a B-milestone)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (momsloven rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (B2G is OIOUBL/NemHandel — a
// different format, not this builder; voluntary B2B uses Peppol BIS 3.0
// UBL, MED — documented), SEPA, CAMT.053 (Nordic norm), ECB, closing
// 3990 -> 3120.

export default {
  meta: {
    country: 'DK',
    baseCurrency: 'DKK',
    locale: 'da-DK',
    legalForms: ['aps', 'as', 'enkeltmandsvirksomhed', 'is'], // ApS = SME standard; IVS abolished
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // CVR number, 8 digits
    vatIdLabel: 'tax_id',
    vatIdFormat: /^DK\d{8}$/i, // DK + 8-digit CVR
    peppolSchemeId: '0184', // DK:CVR (DIGSTORG); legacy 9902 exists
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2500,
    // no small-business exemption scheme in Denmark (research §1)
    smallBusinessScheme: null,
    // 25% standard; NO reduced band — a few items are 0% (exports etc.)
    // rather than reduced-rated. EU reverse charge applies.
    codes: [
      { code: '25', rateBp: 2500, type: 'standard', euReverse: 0, description: '25% standard' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% (eksport m.v.)' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Fritaget' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Omvendt betalingspligt (national)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Omvendt betalingspligt (EU)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Momsordning (brugte varer)' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privat brug' },
    ],
    accounts: {
      // Danish VAT control accounts (research §6 — naming statutory,
      // codes MED): 2720 Købsmoms (input, asset) / 2710 Salgsmoms (output,
      // liability); 2730 Afregning af moms settles via the skattekonto
      ledger: [
        { code: '2720', name: 'Købsmoms', type: 'asset', normalBalance: 'debit' },
        { code: '2710', name: 'Salgsmoms', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2730',
      differenceDefault: '2730',
      afTeDragenName: 'Afregning af moms',
    },
    // returnLayout omitted — the TastSelv momsangivelse engine is a
    // B-milestone
    filingPeriodicity: 'quarterly', // SME band: quarterly (5-50M DKK); see compliance
    reverseChargeEffectiveRateBp: 2500,
  },

  reporting: {
    // format omitted — the årsrapport skema layout is a B-milestone
    taxonomy: null,
    // draft convention aligned with the official Erhvervsstyrelsen
    // Standardkontoplan structure (research §5; codes MED — vendor charts
    // are login-gated, flagged in the brief)
    defaultChart: [
      { code: '1010', name: 'Kontantkasse', type: 'asset', normalBalance: 'debit' },
      { code: '1110', name: 'Bank', type: 'asset', normalBalance: 'debit' },
      { code: '1210', name: 'Tilgodehavender fra salg (Debitorer)', type: 'asset', normalBalance: 'debit' },
      { code: '1410', name: 'Varebeholdning', type: 'asset', normalBalance: 'debit' },
      { code: '1510', name: 'Materielle anlægsaktiver', type: 'asset', normalBalance: 'debit' },
      { code: '1520', name: 'Indretning lejede lokaler', type: 'asset', normalBalance: 'debit' },
      { code: '1620', name: 'Akkumulerede afskrivninger', type: 'asset', normalBalance: 'credit' },
      { code: '2110', name: 'Kreditorer', type: 'liability', normalBalance: 'credit' },
      { code: '2210', name: 'Skyldig A-skat og AM-bidrag', type: 'liability', normalBalance: 'credit' },
      { code: '2330', name: 'Skyldige lønninger', type: 'liability', normalBalance: 'credit' },
      { code: '2380', name: 'Mellemregning ejer', type: 'equity', normalBalance: 'credit' },
      { code: '2710', name: 'Salgsmoms', type: 'liability', normalBalance: 'credit' },
      { code: '2720', name: 'Købsmoms', type: 'asset', normalBalance: 'debit' },
      { code: '2730', name: 'Afregning af moms', type: 'liability', normalBalance: 'credit' },
      { code: '2810', name: 'Skyldig selskabsskat', type: 'liability', normalBalance: 'credit' },
      { code: '2910', name: 'Hensatte forpligtelser', type: 'liability', normalBalance: 'credit' },
      { code: '3100', name: 'Anpartskapital', type: 'equity', normalBalance: 'credit' },
      { code: '3120', name: 'Overført resultat', type: 'equity', normalBalance: 'credit' },
      { code: '3990', name: 'Årets resultat', type: 'equity', normalBalance: 'credit' },
      { code: '4100', name: 'Salg af varer', type: 'income', normalBalance: 'credit' },
      { code: '4200', name: 'Salg af ydelser', type: 'income', normalBalance: 'credit' },
      { code: '4400', name: 'Salg u/moms', type: 'income', normalBalance: 'credit' },
      { code: '5110', name: 'Varekøb', type: 'expense', normalBalance: 'debit' },
      { code: '5210', name: 'Lønninger', type: 'expense', normalBalance: 'debit' },
      { code: '5220', name: 'ATP og øvrige personaleomk.', type: 'expense', normalBalance: 'debit' },
      { code: '5310', name: 'Pensionsbidrag', type: 'expense', normalBalance: 'debit' },
      { code: '5510', name: 'Husleje og lokaleomkostninger', type: 'expense', normalBalance: 'debit' },
      { code: '5610', name: 'Vedligeholdelse', type: 'expense', normalBalance: 'debit' },
      { code: '5710', name: 'El, vand og varme', type: 'expense', normalBalance: 'debit' },
      { code: '5810', name: 'Kontorhold', type: 'expense', normalBalance: 'debit' },
      { code: '5820', name: 'Telefoni og internet', type: 'expense', normalBalance: 'debit' },
      { code: '5910', name: 'Porto og fragt', type: 'expense', normalBalance: 'debit' },
      { code: '6110', name: 'Brændstof og bilomkostninger', type: 'expense', normalBalance: 'debit' },
      { code: '6210', name: 'Rejseomkostninger', type: 'expense', normalBalance: 'debit' },
      { code: '6310', name: 'Repræsentation', type: 'expense', normalBalance: 'debit' },
      { code: '6410', name: 'Forsikringer', type: 'expense', normalBalance: 'debit' },
      { code: '6510', name: 'IT og software', type: 'expense', normalBalance: 'debit' },
      { code: '6610', name: 'Revisor og advokat', type: 'expense', normalBalance: 'debit' },
      { code: '6710', name: 'Markedsføring og annoncer', type: 'expense', normalBalance: 'debit' },
      { code: '6810', name: 'Afskrivninger', type: 'expense', normalBalance: 'debit' },
      { code: '6910', name: 'Renteudgifter og gebyrer', type: 'expense', normalBalance: 'debit' },
      { code: '6990', name: 'Øvrige finansielle omkostninger', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (1210 Debitorer)
    debtorsAccount: '1210',
    inferTaxonomy: null,
    // statutoryAccounts omitted — skema layout is a B-milestone
  },

  compliance: {
    // research §10 (SKAT): VAT is turnover-tiered — half-yearly
    // (< DKK 5M: 1 Sep / 1 Mar), quarterly (5-50M or newly registered:
    // 1st of the 3rd following month), monthly (> DKK 50M: 25th of the
    // following month). The calendar registers the quarterly SME band;
    // annual report (class B) within 5 months of FYE. The official
    // digital-bookkeeping dates (Standardkontoplan 1 Jan 2026, SAF-T v2.0
    // 1 Jan 2027) are noted, not calendarised.
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Qn', deadlineRule: 'dk-quarterly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'dk-5-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'], // CAMT.053 is the Nordic norm (per-bank availability MED)
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // DK is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'DKK',
  },

  documents: {
    // invoiceCompliance omitted — momsloven rule set is a B-milestone
    eInvoicing: 'peppol-bis-3.0', // voluntary B2B via Peppol; B2G is
    // OIOUBL/NemHandel (a different format — the mandate does not apply
    // to this builder's output)
    // auditFile omitted — DK SAF-T v2.0 (1 Jan 2027) is a B-milestone
    languages: ['da', 'en'],
    defaultLanguage: 'da',
  },

  closing: { resultAccount: '3990', equityAccount: '3120' },
};
