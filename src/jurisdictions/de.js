/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Germany jurisdiction profile (Phase B — DE profile).
//
// Data sources: research brief at docs-research/de-profile.md — SKR 03 chart
// verified against ECOVIS RTS + LEWO (773-account SKR 03 list), statutes
// from gesetze-im-internet.de (§§ 12/18/19 UStG, § 149 AO, §§ 264/325 HGB),
// e-invoice status from the BMF FAQ (March 2026), Peppol EAS codelist.
//
// Key corrections the research made to initial assumptions: the UStVA
// monthly threshold is €9,000 prior-year VAT (not €7,500); Kleinunternehmer
// is €25k/€100k since 2025 (Wachstumschancengesetz); small-cap GmbH files
// in 12 months (6 months is the PREPARATION deadline, § 264 vs § 325 HGB);
// SKR 03 has no 6xxx cost class (costs are 3xxx/4xxx; equity is 08xx, not
// 2900); the Peppol EAS scheme for the USt-IdNr is 9930 (0204 = Leitweg-ID
// B2G, 0210 = Italian CODICE FISCALE).
//
// Phase B scope discipline (same contract as LU/GB/FR/US/BE): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (ELSTER UStVA engine
//                            is a B-milestone)
//   - reporting.format      omitted → financial statements fail (HGB Bilanz
//                            + GuV, Gesamtkostenverfahren — B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no German SAF-T;
//                            GoBD is a documentation duty, not a file format)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (§ 14 UStG rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (BMF FAQ: Peppol BIS 3.0 UBL is
// EN 16931-conformant and accepted; scheme 9930 = USt-IdNr), SEPA, CAMT.053,
// ECB, closing 0860 -> 0860.

export default {
  meta: {
    country: 'DE',
    baseCurrency: 'EUR',
    locale: 'de-DE',
    legalForms: ['gmbh', 'ug', 'e-k', 'ag', 'gbr', 'gmbh-co-kg'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Handelsregister number (HRB/HRA + local court)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^DE\d{9}$/i, // USt-IdNr: DE + 9 digits
    // 9930 = Germany VAT number (EAS); 0204 is the B2G Leitweg-ID, 0210 is Italy
    peppolSchemeId: '9930',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 1900,
    // Kleinunternehmer (§ 19 UStG): no VAT charged, § 19 notice on invoices,
    // exempt from UStVA; thresholds since 2025: prev year ≤ €25,000 AND
    // current year ≤ €100,000
    smallBusinessScheme: 'kleinunternehmer',
    // DE rates (§ 12 UStG): 19% standard / 7% reduced / 0% only for solar
    // § 12(3); exports/intra-Community supplies are tax-free (§ 4(1)), not
    // a general 0% category. EU reverse charge (§ 13b) applies.
    codes: [
      { code: '19', rateBp: 1900, type: 'standard', euReverse: 0, description: '19% Regelsteuersatz' },
      { code: '7', rateBp: 700, type: 'standard', euReverse: 0, description: '7% ermäßigter Steuersatz' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% (§ 12 Abs. 3 UStG, Solaranlagen)' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Steuerfrei (§ 4 UStG)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Leistungsempfänger als Steuerschuldner (§ 13b)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Innergemeinschaftlicher Erwerb' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Differenzbesteuerung' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privatnutzung / unentgeltliche Wertabgabe' },
    ],
    accounts: {
      // SKR 03 VAT accounts (research §6): 1570 Abziehbare Vorsteuer
      // (input, asset) / 1776 Umsatzsteuer 19% (output, liability); the
      // settlement account 1780 (Zahllast) absorbs the UStVA balance
      ledger: [
        { code: '1570', name: 'Abziehbare Vorsteuer', type: 'asset', normalBalance: 'debit' },
        { code: '1776', name: 'Umsatzsteuer 19 %', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '1780',
      // year-end clearing runs via 1789/1790 (SKR 03 practice, brief §6 —
      // the 1789/1790 flow is a B-milestone); the rounding difference lands
      // on the Zahllast settlement account
      differenceDefault: '1780',
      settlementAccountName: 'Umsatzsteuer-Vorauszahlungen',
    },
    // returnLayout omitted — the ELSTER UStVA engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly when prior-year VAT > €9,000 (§ 18(2) UStG)
    reverseChargeEffectiveRateBp: 1900,
  },

  reporting: {
    // format omitted — HGB Bilanz + GuV is a B-milestone
    taxonomy: null,
    // DATEV SKR 03 curated subset (research §5) — the dominant German SME
    // convention. SKR 03's 9000 opening-balance clearing account is not a
    // bukio account type, so it is not seeded.
    defaultChart: [
      { code: '0027', name: 'EDV-Software', type: 'asset', normalBalance: 'debit' },
      { code: '0200', name: 'Technische Anlagen und Maschinen', type: 'asset', normalBalance: 'debit' },
      { code: '0300', name: 'Andere Anlagen, Betriebs- und Geschäftsausstattung', type: 'asset', normalBalance: 'debit' },
      { code: '0320', name: 'Pkw', type: 'asset', normalBalance: 'debit' },
      { code: '0410', name: 'Geschäftsausstattung', type: 'asset', normalBalance: 'debit' },
      { code: '0800', name: 'Gezeichnetes Kapital', type: 'equity', normalBalance: 'credit' },
      { code: '0840', name: 'Kapitalrücklage', type: 'equity', normalBalance: 'credit' },
      { code: '0860', name: 'Gewinnvortrag vor Verwendung', type: 'equity', normalBalance: 'credit' },
      { code: '0970', name: 'Sonstige Rückstellungen', type: 'liability', normalBalance: 'credit' },
      { code: '1000', name: 'Kasse', type: 'asset', normalBalance: 'debit' },
      { code: '1200', name: 'Bank', type: 'asset', normalBalance: 'debit' },
      { code: '1400', name: 'Forderungen aus Lieferungen und Leistungen', type: 'asset', normalBalance: 'debit' },
      { code: '1460', name: 'Zweifelhafte Forderungen', type: 'asset', normalBalance: 'debit' },
      { code: '1518', name: 'Geleistete Anzahlungen 19 % Vorsteuer', type: 'asset', normalBalance: 'debit' },
      { code: '1570', name: 'Abziehbare Vorsteuer', type: 'asset', normalBalance: 'debit' },
      { code: '1571', name: 'Abziehbare Vorsteuer 7 %', type: 'asset', normalBalance: 'debit' },
      { code: '1576', name: 'Abziehbare Vorsteuer 19 %', type: 'asset', normalBalance: 'debit' },
      { code: '1600', name: 'Verbindlichkeiten aus Lieferungen und Leistungen', type: 'liability', normalBalance: 'credit' },
      { code: '1665', name: 'Verbindlichkeiten gegenüber GmbH-Gesellschaftern', type: 'liability', normalBalance: 'credit' },
      { code: '1700', name: 'Sonstige Verbindlichkeiten', type: 'liability', normalBalance: 'credit' },
      { code: '1718', name: 'Erhaltene, versteuerte Anzahlungen 19 % USt', type: 'liability', normalBalance: 'credit' },
      { code: '1740', name: 'Verbindlichkeiten aus Lohn und Gehalt', type: 'liability', normalBalance: 'credit' },
      { code: '1741', name: 'Verbindlichkeiten aus Lohn- und Kirchensteuer', type: 'liability', normalBalance: 'credit' },
      { code: '1742', name: 'Verbindlichkeiten im Rahmen der sozialen Sicherheit', type: 'liability', normalBalance: 'credit' },
      { code: '1771', name: 'Umsatzsteuer 7 %', type: 'liability', normalBalance: 'credit' },
      { code: '1776', name: 'Umsatzsteuer 19 %', type: 'liability', normalBalance: 'credit' },
      { code: '1780', name: 'Umsatzsteuer-Vorauszahlungen (Zahllast)', type: 'liability', normalBalance: 'credit' },
      { code: '2100', name: 'Zinsen und ähnliche Aufwendungen', type: 'expense', normalBalance: 'debit' },
      { code: '2200', name: 'Körperschaftsteuer', type: 'expense', normalBalance: 'debit' },
      { code: '2400', name: 'Forderungsverluste', type: 'expense', normalBalance: 'debit' },
      { code: '3100', name: 'Fremdleistungen', type: 'expense', normalBalance: 'debit' },
      { code: '3200', name: 'Wareneingang', type: 'expense', normalBalance: 'debit' },
      { code: '3400', name: 'Wareneingang 19 % Vorsteuer', type: 'expense', normalBalance: 'debit' },
      { code: '4100', name: 'Löhne und Gehälter', type: 'expense', normalBalance: 'debit' },
      { code: '4120', name: 'Gehälter', type: 'expense', normalBalance: 'debit' },
      { code: '4127', name: 'Geschäftsführergehälter', type: 'expense', normalBalance: 'debit' },
      { code: '4130', name: 'Gesetzliche Sozialaufwendungen', type: 'expense', normalBalance: 'debit' },
      { code: '4210', name: 'Miete, unbewegliche Wirtschaftsgüter', type: 'expense', normalBalance: 'debit' },
      { code: '4360', name: 'Versicherungen', type: 'expense', normalBalance: 'debit' },
      { code: '4500', name: 'Fahrzeugkosten', type: 'expense', normalBalance: 'debit' },
      { code: '4600', name: 'Werbekosten', type: 'expense', normalBalance: 'debit' },
      { code: '4830', name: 'Abschreibungen auf Sachanlagen', type: 'expense', normalBalance: 'debit' },
      { code: '4900', name: 'Sonstige betriebliche Aufwendungen', type: 'expense', normalBalance: 'debit' },
      // 8400 first: postingDefaults (src/invoice/index.js) picks the first
      // income account as the default sales account — for a VAT-registered
      // GmbH that is the 19% sales account, NOT the §19 Kleinunternehmer
      // account (8195, a niche exemption), which is listed last
      { code: '8400', name: 'Erlöse 19 % USt', type: 'income', normalBalance: 'credit' },
      { code: '8200', name: 'Erlöse', type: 'income', normalBalance: 'credit' },
      { code: '8300', name: 'Erlöse 7 % USt', type: 'income', normalBalance: 'credit' },
      { code: '8195', name: 'Erlöse Kleinunternehmer § 19 UStG', type: 'income', normalBalance: 'credit' },
    ],
    // debtors account for invoice postings (SKR 03 1400)
    debtorsAccount: '1400',
    inferTaxonomy: null,
    // statutoryAccounts omitted — HGB layout is a B-milestone
  },

  compliance: {
    // research §10: UStVA quarterly (10th of the month after the quarter;
    // monthly when prior-year VAT > €9,000); annual VAT return 31 July
    // (§ 149 AO); annual accounts filed (Offenlegung) 12 months after the
    // balance-sheet date (§ 325 HGB)
    filingTypes: [
      { type: 'UMSATZSTEUER_VORANMELDUNG', periodShape: 'YYYY-Qn', deadlineRule: 'de-ustva-quarterly' },
      { type: 'UMSATZSTEUER_JAHRESERKLAERUNG', periodShape: 'YYYY', deadlineRule: 'de-annual-vat' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'de-12-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // DE is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    // invoiceCompliance omitted — § 14 UStG rule set is a B-milestone
    eInvoicing: 'peppol-bis-3.0', // BMF FAQ: any EN 16931-conformant format
    // accepted; Peppol BIS 3.0 UBL is one (scheme 9930 = USt-IdNr). B2B
    // issue mandate from 1 Jan 2028 (large from 1 Jan 2027).
    // auditFile omitted — no German SAF-T
    languages: ['de', 'en'],
    defaultLanguage: 'de',
  },

  closing: { resultAccount: '0860', equityAccount: '0860' },
};
