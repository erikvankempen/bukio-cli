/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Norway jurisdiction profile (Phase B — NO profile).
//
// Data sources: research brief at docs-research/no-profile.md — VAT
// rates/deadlines from skatteetaten.no + Altinn (official), chart codes
// from the NORSK STANDARD KONTOPLAN (NS 4102, verified sporo.no list),
// identifiers from Brønnøysundregistrene (org.nr), Peppol scheme 0192
// from the official EAS codelist, e-invoice status (EHF/Peppol) from
// Peppol/EHF sources.
//
// Key corrections the research made to initial assumptions: Norway is
// NOT in the EU (EEA — no EU reverse charge, VOEC for low-value imports);
// VAT filing is BI-MONTHLY (6 returns per year), NOT monthly — the
// calendar engine gained the 'YYYY-Pn' period shape for it; there is NO
// small-business VAT exemption (the NOK 50,000 figure is a registration
// threshold); the VAT number carries the MVA suffix (NO123456789MVA);
// the annual accounts are filed by 31 July (approved ≤ 6 months + filed
// ≤ 1 month), not a generic month-count.
//
// Phase B scope discipline (same contract as LU/GB/FR/US/BE/DE/DK/FI):
// register ONLY what maps to existing generic engines; everything else
// fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (mva-meldingen via
//                            Altinn is a B-milestone)
//   - reporting.format      omitted → financial statements fail (årsregnskap
//                            layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (Norway mandates
//                            SAF-T Accounting — a B-milestone, distinct
//                            from XAF/FAIA/FEC)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (mva-loven rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (EHF 3.0 = Peppol BIS Billing
// 3.0 UBL profile — B2G mandatory since 2012, B2B voluntary; the
// builder's output IS the EHF 3.0 profile; scheme 0192), SEPA, CAMT.053,
// ECB, closing 8960 -> 8960.

export default {
  meta: {
    country: 'NO',
    baseCurrency: 'NOK',
    locale: 'nb-NO',
    legalForms: ['as', 'enk', 'ans', 'da', 'nuf'], // AS = SME standard
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // organisasjonsnummer: 9 digits (123 456 789)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^NO\d{9}MVA$/i, // NO + org.nr + MVA suffix (Peppol rule NO-R-001)
    peppolSchemeId: '0192', // NO:ORG
    accountNumber: { kind: 'iban' }, // NO + 15 chars; domestic: 11-digit kontonummer
  },

  tax: {
    system: 'vat',
    standardRateBp: 2500,
    // no small-business VAT exemption — the NOK 50,000 limit is only a
    // registration threshold (research §1)
    smallBusinessScheme: null,
    // NO rates 2026 (skatteetaten.no): 25% normal / 15% foodstuffs /
    // 12% passenger transport & accommodation / 0% exempt & outside
    // scope. Norway is NOT in the EU — NO EU reverse-charge code (RE);
    // domestic reverse charge exists for specific cases only.
    codes: [
      { code: '25', rateBp: 2500, type: 'standard', euReverse: 0, description: '25% høy sats' },
      { code: '15', rateBp: 1500, type: 'standard', euReverse: 0, description: '15% matvarer' },
      { code: '12', rateBp: 1200, type: 'standard', euReverse: 0, description: '12% persontransport m.v.' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% avgiftsfri' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Utenfor avgiftsområdet' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Omvendt avgiftsplikt (spesielle tilfeller)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marginalordningen' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privat bruk' },
    ],
    accounts: {
      // NS 4102 MVA accounts (research §6): 2710 Inngående MVA (input,
      // asset) / 2700 Utgående MVA (output, liability); the settlement
      // account 2740 Oppgjørskonto MVA nets the period and maps to the
      // mva-meldingen
      ledger: [
        { code: '2710', name: 'Inngående MVA, høy sats', type: 'asset', normalBalance: 'debit' },
        { code: '2700', name: 'Utgående MVA, høy sats', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2740',
      differenceDefault: '2740',
      settlementAccountName: 'Oppgjørskonto MVA',
    },
    // returnLayout omitted — the Altinn mva-meldingen engine is a
    // B-milestone
    filingPeriodicity: 'bimonthly', // 6 returns/year; annual option < NOK 1M
    reverseChargeEffectiveRateBp: 2500,
  },

  reporting: {
    // format omitted — the årsregnskap layout is a B-milestone
    taxonomy: null,
    // Norsk Standard Kontoplan (NS 4102) — verified sporo.no list
    // (research §5). 2060 Privatuttak is a debit-normal equity account
    // (contra-equity, migration 024). 8960 Overført til egenkapital is
    // the closing transfer account.
    defaultChart: [
      { code: '1200', name: 'Maskiner og anlegg', type: 'asset', normalBalance: 'debit' },
      { code: '1220', name: 'Inventar og utstyr', type: 'asset', normalBalance: 'debit' },
      { code: '1500', name: 'Kundefordringer', type: 'asset', normalBalance: 'debit' },
      { code: '1900', name: 'Kontanter', type: 'asset', normalBalance: 'debit' },
      { code: '1920', name: 'Bankinnskudd', type: 'asset', normalBalance: 'debit' },
      { code: '1921', name: 'Bankinnskudd 2', type: 'asset', normalBalance: 'debit' },
      { code: '2000', name: 'Aksjekapital', type: 'equity', normalBalance: 'credit' },
      { code: '2050', name: 'Egenkapital', type: 'equity', normalBalance: 'credit' },
      { code: '2060', name: 'Privatuttak', type: 'equity', normalBalance: 'debit' },
      { code: '2080', name: 'Privat innskudd', type: 'equity', normalBalance: 'credit' },
      { code: '2400', name: 'Leverandørgjeld', type: 'liability', normalBalance: 'credit' },
      { code: '2500', name: 'Betalbar skatt', type: 'liability', normalBalance: 'credit' },
      { code: '2600', name: 'Skattetrekk', type: 'liability', normalBalance: 'credit' },
      { code: '2700', name: 'Utgående MVA, høy sats', type: 'liability', normalBalance: 'credit' },
      { code: '2701', name: 'Utgående MVA, middels sats', type: 'liability', normalBalance: 'credit' },
      { code: '2702', name: 'Utgående MVA, lav sats', type: 'liability', normalBalance: 'credit' },
      { code: '2710', name: 'Inngående MVA, høy sats', type: 'asset', normalBalance: 'debit' },
      { code: '2711', name: 'Inngående MVA, middels sats', type: 'asset', normalBalance: 'debit' },
      { code: '2712', name: 'Inngående MVA, lav sats', type: 'asset', normalBalance: 'debit' },
      { code: '2740', name: 'Oppgjørskonto MVA', type: 'liability', normalBalance: 'credit' },
      { code: '2910', name: 'Forskudd fra kunder', type: 'liability', normalBalance: 'credit' },
      { code: '2960', name: 'Påløpte kostnader', type: 'liability', normalBalance: 'credit' },
      { code: '2990', name: 'Annen kortsiktig gjeld', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Salgsinntekt, avgiftspliktig', type: 'income', normalBalance: 'credit' },
      { code: '3100', name: 'Salgsinntekt, avgiftsfri', type: 'income', normalBalance: 'credit' },
      { code: '3200', name: 'Salgsinntekt, utenfor avgiftsområdet', type: 'income', normalBalance: 'credit' },
      { code: '3600', name: 'Leieinntekt', type: 'income', normalBalance: 'credit' },
      { code: '3900', name: 'Annen driftsinntekt', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Varekjøp', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Fremmedytelser og underentreprise', type: 'expense', normalBalance: 'debit' },
      { code: '5000', name: 'Lønn', type: 'expense', normalBalance: 'debit' },
      { code: '5090', name: 'Feriepenger', type: 'expense', normalBalance: 'debit' },
      { code: '5400', name: 'Arbeidsgiveravgift', type: 'expense', normalBalance: 'debit' },
      { code: '5800', name: 'Pensjonskostnader', type: 'expense', normalBalance: 'debit' },
      { code: '6000', name: 'Avskrivning', type: 'expense', normalBalance: 'debit' },
      { code: '6300', name: 'Leie lokale', type: 'expense', normalBalance: 'debit' },
      { code: '6400', name: 'Leie maskiner, inventar', type: 'expense', normalBalance: 'debit' },
      { code: '6550', name: 'Programvare', type: 'expense', normalBalance: 'debit' },
      { code: '6600', name: 'Reparasjon og vedlikehold', type: 'expense', normalBalance: 'debit' },
      { code: '6720', name: 'Regnskapshonorar', type: 'expense', normalBalance: 'debit' },
      { code: '6790', name: 'Andre fremmedtjenester', type: 'expense', normalBalance: 'debit' },
      { code: '6800', name: 'Kontorrekvisita', type: 'expense', normalBalance: 'debit' },
      { code: '6900', name: 'Telefon', type: 'expense', normalBalance: 'debit' },
      { code: '7140', name: 'Reisekostnad', type: 'expense', normalBalance: 'debit' },
      { code: '7300', name: 'Markedsføring', type: 'expense', normalBalance: 'debit' },
      { code: '7500', name: 'Forsikring', type: 'expense', normalBalance: 'debit' },
      { code: '7770', name: 'Bank- og kortgebyrer', type: 'expense', normalBalance: 'debit' },
      { code: '7790', name: 'Andre driftskostnader', type: 'expense', normalBalance: 'debit' },
      { code: '8140', name: 'Rentekostnad', type: 'expense', normalBalance: 'debit' },
      { code: '8300', name: 'Betalbar skatt', type: 'expense', normalBalance: 'debit' },
      { code: '8960', name: 'Overført til egenkapital', type: 'equity', normalBalance: 'credit' },
    ],
    // debtors account for invoice postings (1500 Kundefordringer)
    debtorsAccount: '1500',
    bankAccountDefault: '1920',
    inferTaxonomy: null,
    // statutoryAccounts omitted — årsregnskap layout is a B-milestone
  },

  compliance: {
    // research §10 (Altinn): bi-monthly mva-meldingen (6 periods/year,
    // 1 month + 10 days after the period end; P3 summer exception 31 Aug;
    // P6 due 10 Feb next year); annual accounts approved ≤ 6 months and
    // filed ≤ 1 month after FYE (31 July calendar-year)
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Pn', deadlineRule: 'no-bimonthly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'no-7-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'], // CAMT.053 is the Visma/Tripletex standard (MED)
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // NO is SEPA (EEA)
    fxSource: 'ecb',
    baseCurrency: 'NOK',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (mva-loven additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // EHF 3.0 = Peppol BIS 3.0 UBL profile:
    // B2G mandatory since 2012, B2B voluntary (B2B mandate proposed for
    // 2027-2030, not law)
    // auditFile omitted — SAF-T Accounting (mandatory in NO) is a
    // B-milestone
    languages: ['no', 'en'],
    defaultLanguage: 'no',
  },

  closing: { resultAccount: '8960', equityAccount: '8960' },
};
