/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Finland jurisdiction profile (Phase B — FI profile).
//
// Data sources: research brief at docs-research/fi-profile.md — VAT rates
// from Vero (Finnish Tax Administration, updated 1/1/2026), return
// frequencies/deadlines from Marosa/Avalara/1Office, chart codes/labels
// from the current Liikekirjuri 2026 model chart (asteri.fi PDF, HIGH),
// e-invoice status from the EU Commission 2025 country sheet.
//
// Key corrections the research made to initial assumptions: the reduced
// VAT rate is 13.5% since 1 Jan 2026 (was 14%; the old 14%/10% scheme
// ended) — 10% now covers newspapers/magazines only; sub-€30K businesses
// file VAT ANNUALLY (not quarterly); annual accounts are FILED within
// 8 months of FYE (prepared within 4) — not "1 month after adoption";
// there is NO B2B e-invoicing mandate (the 2025-04-01 assumption is
// FALSE) — only B2G is mandatory; Peppol BIS 3.0 is accepted for
// voluntary transmission (scheme 0037).
//
// Phase B scope discipline (same contract as LU/GB/FR/US/BE/DE/DK):
// register ONLY what maps to existing generic engines; everything else
// fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (OmaVero
//                            kausiveroilmoitus engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail
//                            (tilinpäätös tuloslaskelma/tase layout is a
//                            B-milestone; iXBRL mandatory from 2027)
//   - documents.auditFile   omitted → XAF export fails (no Finnish SAF-T)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (arvonlisäverolaki rule set is a B-milestone)
// Registered: e-invoicing 'peppol-bis-3.0' (voluntary Peppol — the e-
// Invoicing Act 241/2019 gives B2B buyers a right to request e-invoices;
// B2G is mandatory), SEPA, CAMT.053 (Finnish norm, MED), ECB, closing
// 2375 -> 2251.

export default {
  meta: {
    country: 'FI',
    baseCurrency: 'EUR',
    locale: 'fi-FI',
    legalForms: ['oy', 'toiminimi', 'ky', 'oyj'], // Oy = SME standard
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Y-tunnus (Business ID): 7 digits + check
    vatIdLabel: 'tax_id',
    vatIdFormat: /^FI\d{8}$/i, // FI + 8 digits (Y-tunnus without the dash)
    peppolSchemeId: '0037', // Finnish tax administration organisation code (LY-tunnus)
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2550,
    // arvonlisäveroton vähäinen toiminta: exemption when turnover of the
    // current AND previous calendar year each ≤ €20,000 (in force 1 Jan
    // 2025, raised from €15,000); exempt sellers cannot deduct input VAT
    smallBusinessScheme: 'franchise',
    // FI rates 2026 (Vero): 25.5% general (since 1 Sep 2024) / 13.5%
    // reduced (since 1 Jan 2026, was 14%) / 10% newspapers & magazines /
    // 0% exports & intra-Community. EU reverse charge applies.
    codes: [
      { code: '25.5', rateBp: 2550, type: 'standard', euReverse: 0, description: '25,5 % yleinen' },
      { code: '13.5', rateBp: 1350, type: 'standard', euReverse: 0, description: '13,5 % alennettu' },
      { code: '10', rateBp: 1000, type: 'standard', euReverse: 0, description: '10 % sanoma- ja aikakauslehdet' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0 % vienti ja yhteisömyynti' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Veroton' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Käännetty verovelvollisuus (kotimainen)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Käännetty verovelvollisuus (EU)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marginaaliverotus' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Yksityiskäyttö' },
    ],
    accounts: {
      // Finnish convention (Liikekirjuri family, research §6): 1763
      // Arvonlisäverosaamiset (input, asset) / 2939 AV Verovelka (output,
      // liability); the net settles via the verotili tax account
      ledger: [
        { code: '1763', name: 'Arvonlisäverosaamiset', type: 'asset', normalBalance: 'debit' },
        { code: '2939', name: 'AV Verovelka', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2939',
      differenceDefault: '2939',
      settlementAccountName: 'AV Verovelka',
    },
    // returnLayout omitted — the OmaVero kausiveroilmoitus engine is a
    // B-milestone
    filingPeriodicity: 'quarterly', // monthly > €100K; quarterly €30-100K; annual < €30K
    reverseChargeEffectiveRateBp: 2550,
  },

  reporting: {
    // format omitted — tilinpäätös layout is a B-milestone
    taxonomy: null,
    // Liikekirjuri 2026 model chart (research §5, HIGH for codes/labels
    // from the current Finnish software chart). 2350 Yksityistili is a
    // debit-normal equity account (contra-equity, migration 024).
    // 2375 Tilikauden voitto is the result account — the exact code was
    // not printed in the source PDF (heading only); 2375 is from the
    // Liikekirjuri variant list (MED, flagged in the brief).
    defaultChart: [
      { code: '1021', name: 'Kehittämismenot', type: 'asset', normalBalance: 'debit' },
      { code: '1041', name: 'ATK-ohjelmat', type: 'asset', normalBalance: 'debit' },
      { code: '1051', name: 'Liikearvo', type: 'asset', normalBalance: 'debit' },
      { code: '1121', name: 'Rakennukset', type: 'asset', normalBalance: 'debit' },
      { code: '1161', name: 'Koneet ja laitteet', type: 'asset', normalBalance: 'debit' },
      { code: '1440', name: 'Osakkeet ja osuudet', type: 'asset', normalBalance: 'debit' },
      { code: '1501', name: 'Aineet ja tarvikkeet', type: 'asset', normalBalance: 'debit' },
      { code: '1521', name: 'Valmiit tuotteet', type: 'asset', normalBalance: 'debit' },
      { code: '1701', name: 'Myyntisaamiset', type: 'asset', normalBalance: 'debit' },
      { code: '1761', name: 'Verosaamiset', type: 'asset', normalBalance: 'debit' },
      { code: '1762', name: 'Verotilisaamiset', type: 'asset', normalBalance: 'debit' },
      { code: '1763', name: 'Arvonlisäverosaamiset', type: 'asset', normalBalance: 'debit' },
      { code: '1800', name: 'Siirtosaamiset', type: 'asset', normalBalance: 'debit' },
      { code: '1900', name: 'Kassa', type: 'asset', normalBalance: 'debit' },
      { code: '1910', name: 'Pankkitili (Nordea)', type: 'asset', normalBalance: 'debit' },
      { code: '1970', name: 'Pankkitili', type: 'asset', normalBalance: 'debit' },
      { code: '1990', name: 'Pankkitilien väliset siirrot', type: 'asset', normalBalance: 'debit' },
      { code: '2001', name: 'Osakepääoma', type: 'equity', normalBalance: 'credit' },
      { code: '2061', name: 'SVOP-rahasto', type: 'equity', normalBalance: 'credit' },
      { code: '2201', name: 'Peruspääoma', type: 'equity', normalBalance: 'credit' },
      { code: '2251', name: 'Ed. tilikausien voitto/tappio', type: 'equity', normalBalance: 'credit' },
      { code: '2350', name: 'Yksityistili', type: 'equity', normalBalance: 'debit' },
      { code: '2375', name: 'Tilikauden voitto (tappio)', type: 'equity', normalBalance: 'credit' },
      { code: '2381', name: 'Pääomalaina', type: 'equity', normalBalance: 'credit' },
      { code: '2621', name: 'Lainat rahoituslaitoksilta', type: 'liability', normalBalance: 'credit' },
      { code: '2821', name: 'Lainat rahoituslaitoksilta, lyhytaik.', type: 'liability', normalBalance: 'credit' },
      { code: '2864', name: 'Saadut ennakot', type: 'liability', normalBalance: 'credit' },
      { code: '2871', name: 'Ostovelat', type: 'liability', normalBalance: 'credit' },
      { code: '2939', name: 'AV Verovelka', type: 'liability', normalBalance: 'credit' },
      { code: '2963', name: 'TyEL-velka', type: 'liability', normalBalance: 'credit' },
      { code: '2979', name: 'Siirtovelat', type: 'liability', normalBalance: 'credit' },
      { code: '3000', name: 'Myynti ALV 25,5%', type: 'income', normalBalance: 'credit' },
      { code: '3001', name: 'Myynti ALV 13,5%', type: 'income', normalBalance: 'credit' },
      { code: '3002', name: 'Myynti ALV 10%', type: 'income', normalBalance: 'credit' },
      { code: '3003', name: 'Myynti 0%', type: 'income', normalBalance: 'credit' },
      { code: '3004', name: 'Myynti', type: 'income', normalBalance: 'credit' },
      { code: '3454', name: 'Vuokratuotot', type: 'income', normalBalance: 'credit' },
      { code: '3994', name: 'Liiketoiminnan muut tuotot', type: 'income', normalBalance: 'credit' },
      { code: '4000', name: 'Ostot ALV 25,5%', type: 'expense', normalBalance: 'debit' },
      { code: '4004', name: 'Ostot', type: 'expense', normalBalance: 'debit' },
      { code: '4454', name: 'Ulkopuoliset palvelut', type: 'expense', normalBalance: 'debit' },
      { code: '5000', name: 'Palkat ja palkkiot', type: 'expense', normalBalance: 'debit' },
      { code: '6100', name: 'YEL-maksut', type: 'expense', normalBalance: 'debit' },
      { code: '6130', name: 'TyEL-maksut', type: 'expense', normalBalance: 'debit' },
      { code: '6300', name: 'Sosiaaliturvamaksut', type: 'expense', normalBalance: 'debit' },
      { code: '6870', name: 'Poisto koneista ja kalustosta', type: 'expense', normalBalance: 'debit' },
      { code: '7214', name: 'Vuokrat', type: 'expense', normalBalance: 'debit' },
      { code: '7394', name: 'Sähkö', type: 'expense', normalBalance: 'debit' },
      { code: '7864', name: 'Matka- ja majoituskulut', type: 'expense', normalBalance: 'debit' },
      { code: '8054', name: 'Mainoskulut', type: 'expense', normalBalance: 'debit' },
      { code: '8384', name: 'Taloushallintopalvelut', type: 'expense', normalBalance: 'debit' },
      { code: '8504', name: 'Puhelin- ja tietoliikenne', type: 'expense', normalBalance: 'debit' },
      { code: '8564', name: 'Rahaliikenteen kulut', type: 'expense', normalBalance: 'debit' },
      { code: '8624', name: 'Toimistokulut', type: 'expense', normalBalance: 'debit' },
      { code: '8704', name: 'Luottotappiot', type: 'expense', normalBalance: 'debit' },
      { code: '8764', name: 'Muut kulut', type: 'expense', normalBalance: 'debit' },
      { code: '8804', name: 'Vähennyskelvottomat kulut', type: 'expense', normalBalance: 'debit' },
      { code: '9460', name: 'Korkokulut lainoista', type: 'expense', normalBalance: 'debit' },
      { code: '9900', name: 'Ennakkoverot', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (1701 Myyntisaamiset)
    debtorsAccount: '1701',
    inferTaxonomy: null,
    // statutoryAccounts omitted — tilinpäätös layout is a B-milestone
  },

  compliance: {
    // research §10: VAT monthly (> €100K) / quarterly (€30-100K) / annual
    // (< €30K, due end of February) — the calendar registers the quarterly
    // SME band, due the 12th of the second month after the period; annual
    // accounts prepared within 4 months, FILED within 8 months of FYE (PRH)
    filingTypes: [
      { type: 'VAT', periodShape: 'YYYY-Qn', deadlineRule: 'fi-quarterly' },
      { type: 'ANNUAL_ACCOUNTS', periodShape: 'YYYY', deadlineRule: 'fi-8-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'], // CAMT.053 is the Finnish standard (MED)
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // FI is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    // invoiceCompliance omitted — arvonlisäverolaki rule set is a
    // B-milestone
    eInvoicing: 'peppol-bis-3.0', // B2B is NOT mandated (research §11 —
    // only B2G is); Peppol BIS 3.0 is accepted for voluntary transmission
    // and the e-Invoicing Act 241/2019 gives buyers a right to request it
    // auditFile omitted — no Finnish SAF-T
    languages: ['fi', 'en'],
    defaultLanguage: 'fi',
  },

  closing: { resultAccount: '2375', equityAccount: '2251' },
};
