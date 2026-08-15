/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Austria jurisdiction profile (Phase C — AT profile).
//
// Data sources: research brief at docs-research/at-profile.md — EKR chart
// (Einheitskontenrahmen) verified against the official BMF SAF-T chart
// (EKR_fuer_SAF-T.csv, bmf.gv.at), rates from UStG § 10 + the 2026 VAT guides,
// UVA deadlines from the BMF/usp.gv.at, Kleinunternehmer threshold from
// § 6/1 Z 27 UStG (raised to €55,000 on 1 Jan 2025), Peppol EAS codelist
// (9914 = Österreichische Umsatzsteuer-Identifikationsnummer).
//
// Key corrections research made to initial assumptions: the quarterly UVA
// option applies up to €100,000 prior-year turnover (monthly above); BOTH
// monthly and quarterly UVA are due the 15th of the SECOND following month;
// the annual USt-Erklärung is due 30 June (electronic filing is mandatory —
// 30 April is the paper deadline); the EKR has a single Umsatzsteuer output
// account 3500 and a single Vorsteuer input account 2500 (no per-rate VAT
// accounts like SKR 03); Austria has NO statutory chart law for SMEs but the
// EKR is the dominant convention and the BMF SAF-T chart; Austrian SAF-T
// (SAF-T AT, mandatory since 2021 for bookkeeping software) is an OECD-style
// XML — NOT the Dutch Auditfile Financieel 4.0 layout the XAF builder emits.
//
// Phase C scope discipline (same contract as the Phase B markets): register
// ONLY what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (UVA engine is a
//                            B-milestone)
//   - reporting.format      omitted → financial statements fail (UGB/Jahres-
//                            abschluss layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (SAF-T AT is a
//                            different XML schema — OECD-style, not the
//                            Belastingdienst Auditfile)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (§ 11 UStG full-invoice rule set is a
//                            B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (AT is a Peppol participant;
// B2G e-invoicing mandatory since 2014, B2B voluntary; scheme 9914 = UID),
// SEPA, CAMT.053, ECB, closing 9350 -> 9380.

export default {
  meta: {
    country: 'AT',
    baseCurrency: 'EUR',
    locale: 'de-AT',
    legalForms: ['gmbh', 'ag', 'og', 'kg', 'e-u'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Firmenbuchnummer (FN; format e.g. FN 123456a — free text)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^ATU\d{8}$/i, // UID: ATU + 8 digits (ATU12345678)
    // 9914 = Österreichische Umsatzsteuer-Identifikationsnummer (EAS)
    peppolSchemeId: '9914',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2000,
    // Kleinunternehmer (§ 6 Abs. 1 Z 27 UStG): no VAT charged, exempt from
    // UVA; threshold €55,000 annual turnover since 1 Jan 2025 (was €35,000)
    smallBusinessScheme: 'kleinunternehmer',
    // AT rates (§ 10 UStG): 20% standard / 13% Zwischensteuersatz (catering,
    // cultural, accommodation) / 10% reduced (food, books, medicine, rent,
    // public transport). A 4.9% rate applies to selected basic food from
    // 1 Jul 2026 (not yet a standing bukio code). Exports and intra-Community
    // supplies are steuerfrei (§ 6) — not a general 0% category. EU reverse
    // charge (§ 19 Abs. 1 UStG) applies.
    codes: [
      { code: '20', rateBp: 2000, type: 'standard', euReverse: 0, description: '20% Normalsteuersatz' },
      { code: '13', rateBp: 1300, type: 'standard', euReverse: 0, description: '13% Zwischensteuersatz' },
      { code: '10', rateBp: 1000, type: 'standard', euReverse: 0, description: '10% ermäßigter Steuersatz' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Steuerfrei (§ 6 UStG)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Übergang der Steuerschuld (§ 19 Abs. 1)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Innergemeinschaftlicher Erwerb' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Differenzbesteuerung' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privatnutzung / unentgeltliche Wertabgabe' },
    ],
    accounts: {
      // EKR VAT accounts (BMF SAF-T chart): 2500 Vorsteuer (input, asset) /
      // 3500 Umsatzsteuer (output, liability). The EKR has single VAT
      // accounts (no per-rate accounts like SKR 03); the settlement account
      // 3500 absorbs the UVA balance + rounding differences.
      ledger: [
        { code: '2500', name: 'Vorsteuer', type: 'asset', normalBalance: 'debit' },
        { code: '3500', name: 'Umsatzsteuer', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '3500',
      differenceDefault: '3500', // UVA balance/rounding lands on the USt account
      settlementAccountName: 'Umsatzsteuer-Zahllast (Finanzamt)',
    },
    // returnLayout omitted — the UVA engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly when prior-year turnover > €100,000 (§ 21 UStG)
    reverseChargeEffectiveRateBp: 2000,
  },

  reporting: {
    // format omitted — UGB Jahresabschluss (Bilanz + GuV) is a B-milestone
    taxonomy: null,
    // EKR (Einheitskontenrahmen) curated SME subset — the dominant Austrian
    // convention and the official BMF SAF-T chart. Classes: 0 Anlagevermögen,
    // 1 Vorräte, 2 Forderungen/USt-VSt/Kassa/Bank, 3 Verbindlichkeiten,
    // 4 Erlöse, 5 Wareneinsatz, 6 Personal, 7 sonstige Aufwendungen,
    // 8 Steuern, 9 Eigenkapital/Abschluss. Account codes are 3- or 4-digit
    // (BMF SAF-T uses the short forms; bukio requires 4-digit codes, so the
    // 3-digit EKR codes are zero-padded: 0620, 0630, 0660, ...).
    defaultChart: [
      { code: '0620', name: 'Büromaschinen, EDV-Anlagen', type: 'asset', normalBalance: 'debit' },
      { code: '0630', name: 'PKW', type: 'asset', normalBalance: 'debit' },
      { code: '0660', name: 'Andere Betriebs- und Geschäftsausstattung', type: 'asset', normalBalance: 'debit' },
      { code: '0689', name: 'Kumulierte Abschreibungen Betriebs- und Geschäftsausstattung', type: 'asset', normalBalance: 'credit' },
      { code: '1600', name: 'Waren', type: 'asset', normalBalance: 'debit' },
      { code: '2000', name: 'Forderungen aus Lieferungen und Leistungen Inland', type: 'asset', normalBalance: 'debit' },
      { code: '2300', name: 'Sonstige Forderungen und Vermögensgegenstände', type: 'asset', normalBalance: 'debit' },
      { code: '2500', name: 'Vorsteuer', type: 'asset', normalBalance: 'debit' },
      { code: '2700', name: 'Kassa', type: 'asset', normalBalance: 'debit' },
      { code: '2800', name: 'Guthaben bei Kreditinstituten', type: 'asset', normalBalance: 'debit' },
      { code: '3110', name: 'Verbindlichkeiten gegenüber Kreditinstituten', type: 'liability', normalBalance: 'credit' },
      { code: '3300', name: 'Lieferverbindlichkeiten Inland', type: 'liability', normalBalance: 'credit' },
      { code: '3500', name: 'Umsatzsteuer', type: 'liability', normalBalance: 'credit' },
      { code: '3600', name: 'Verbindlichkeiten im Rahmen der sozialen Sicherheit', type: 'liability', normalBalance: 'credit' },
      { code: '3700', name: 'Übrige sonstige Verbindlichkeiten', type: 'liability', normalBalance: 'credit' },
      { code: '9010', name: 'Stammkapital', type: 'equity', normalBalance: 'credit' },
      { code: '9350', name: 'Jahresgewinn/-verlust', type: 'equity', normalBalance: 'credit' },
      { code: '9380', name: 'Gewinnvortrag aus Vorjahren', type: 'equity', normalBalance: 'credit' },
      // 4000 first: postingDefaults (src/invoice/index.js) picks the first
      // income account as the default sales account — the 20% Erlöse account
      // for a VAT-registered GmbH (not the § 6 Kleinunternehmer exemption)
      { code: '4000', name: 'Erlöse 20 %', type: 'income', normalBalance: 'credit' },
      { code: '4010', name: 'Erlöse 10 %', type: 'income', normalBalance: 'credit' },
      { code: '4100', name: 'Erlöse ig. Lieferungen (steuerfrei)', type: 'income', normalBalance: 'credit' },
      { code: '4800', name: 'Übrige betriebliche Erträge 20 %', type: 'income', normalBalance: 'credit' },
      { code: '5000', name: 'Wareneinsatz', type: 'expense', normalBalance: 'debit' },
      { code: '6000', name: 'Löhne', type: 'expense', normalBalance: 'debit' },
      { code: '7200', name: 'Instandhaltung', type: 'expense', normalBalance: 'debit' },
      { code: '7400', name: 'Mietaufwand 20 %', type: 'expense', normalBalance: 'debit' },
      { code: '7600', name: 'Büromaterial und Drucksorten', type: 'expense', normalBalance: 'debit' },
      { code: '7700', name: 'Versicherungen', type: 'expense', normalBalance: 'debit' },
      { code: '8500', name: 'Körperschaftsteuer', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (EKR 2000 Forderungen Inland)
    debtorsAccount: '2000',
    bankAccountDefault: '2800',
    inferTaxonomy: null,
    // statutoryAccounts omitted — UGB layout is a B-milestone
  },

  compliance: {
    // UVA (Umsatzsteuervoranmeldung): due the 15th of the SECOND following
    // month — monthly when prior-year turnover > €100,000, quarterly below
    // (§ 21 UStG); the annual USt-Erklärung is due 30 June of the following
    // year (mandatory electronic filing). No ANNUAL_ACCOUNTS filing for
    // small GmbH (Offenlegung applies only above the § 221 UGB size
    // thresholds — B-milestone anyway).
    filingTypes: [
      { type: 'UMSATZSTEUER_VORANMELDUNG', periodShape: 'YYYY-Qn', deadlineRule: 'at-uva-quarterly' },
      { type: 'UMSATZSTEUER_JAHRESERKLAERUNG', periodShape: 'YYYY', deadlineRule: 'at-annual-vat' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // AT is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    // invoiceCompliance omitted — § 11 UStG rule set is a B-milestone
    eInvoicing: 'peppol-bis-3.0', // Peppol participant; B2G e-invoicing
    // mandatory since 2014 (e-Rechnung.gv.at), B2B voluntary (EU mandate
    // planned from 2027/2028); scheme 9914 = UID
    // auditFile omitted — SAF-T AT is an OECD-style XML, not the Dutch
    // Auditfile Financieel 4.0 layout
    languages: ['de', 'en'],
    defaultLanguage: 'de',
  },

  closing: { resultAccount: '9350', equityAccount: '9380' },
};
