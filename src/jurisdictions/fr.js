/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// France jurisdiction profile (Phase B — FR profile).
//
// Data sources: PCG (Plan Comptable Général, règlement ANC 2014-03) research
// brief at docs-research/fr-pcg.md — every account code source-verified
// (dougs.fr PCG pages, Pennylane, l-expert-comptable, service-public.gouv.fr,
// impots.gouv.fr). Classe-2 fixed-asset codes (211/213/215/2183) follow the
// standard PCG (verified at class level in the brief §9; the individual
// codes are the canonical PCG ones used across French accounting software).
//
// Phase B scope discipline (same contract as LU B1 / GB): register ONLY what
// maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (CA3 return engine
//                            is a B-milestone; RSI = semi-annual acomptes
//                            July 55% / Dec 40% + CA12 by 2nd working day
//                            after 1 May — verified, not calendarised yet)
//   - reporting.format      omitted → financial statements fail (PCG bilan
//                            / compte de résultat layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (FR uses FEC — the
//                            Fichier des Écritures Comptables, CGI art. 54
//                            septies — a B-milestone, not XAF/FAIA)
//   - documents.invoiceCompliance omitted → invoice finalization fails
//                            (CGI art. 289 mention obligatoires rule set
//                            is a B-milestone)
//   - compliance.filingTypes [] → calendar empty until the CA3/RSI
//                            deadline rules are verified and calendarised
// Registered: e-invoicing 'peppol-bis-3.0' (EN 16931 UBL — the EU standard
// accepted in the FR mandate via PDPs; scheme 0002 = SIREN), SEPA, CAMT.053,
// ECB, closing 120 -> 110.

export default {
  meta: {
    country: 'FR',
    baseCurrency: 'EUR',
    locale: 'fr',
    // French SME legal forms (PCG/commercial-law conventions)
    legalForms: ['entreprise-individuelle', 'eurl', 'sarl', 'sasu', 'sas', 'sa'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // SIREN (9 digits) / SIRET (14)
    vatIdLabel: 'tax_id',
    vatIdFormat: /^FR\d{11}$/i, // FR + 2-digit key + 9-digit SIREN
    peppolSchemeId: '0002', // SIREN (French Peppol registry code, BT-34/BT-49)
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2000,
    smallBusinessScheme: 'franchise', // franchise en base: €85K goods / €37.5K services
    // French TVA rates (service-public.gouv.fr, verified): 20% normal /
    // 10% intermediate / 5.5% reduced / 2.1% particular. EU reverse charge
    // applies (RE); the 2025/2026 rate structure is unchanged.
    codes: [
      { code: '20', rateBp: 2000, type: 'standard', euReverse: 0, description: '20% taux normal' },
      { code: '10', rateBp: 1000, type: 'standard', euReverse: 0, description: '10% taux intermédiaire' },
      { code: '5.5', rateBp: 550, type: 'standard', euReverse: 0, description: '5,5% taux réduit' },
      { code: '2.1', rateBp: 210, type: 'standard', euReverse: 0, description: '2,1% taux particulier' },
      { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% taux zéro' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exonéré' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Auto-liquidation (nationale)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Auto-liquidation (UE)' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marge' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Usage privé' },
    ],
    accounts: {
      // PCG classe 445 (verified): 44566 TVA sur autres biens et services
      // (input, asset) / 44571 TVA collectée (output, liability); the
      // settlement account 44551 TVA à décaisser absorbs the return balance
      // and rounding differences
      ledger: [
        { code: '44566', name: 'TVA sur autres biens et services', type: 'asset', normalBalance: 'debit' },
        { code: '44571', name: 'TVA collectée', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '44551',
      differenceDefault: '44551',
      settlementAccountName: 'TVA à décaisser',
    },
    // returnLayout omitted — CA3 monthly / RSI semi-annual return engine
    // is a B-milestone
    filingPeriodicity: 'monthly', // réel normal CA3; RSI = 2 semi-annual acomptes
    reverseChargeEffectiveRateBp: 2000,
  },

  reporting: {
    // format omitted — the PCG bilan / compte de résultat layout is a
    // B-milestone
    taxonomy: null, // no separate taxonomy system — the PCG codes ARE the chart
    defaultChart: [
      { code: '101', name: 'Capital social', type: 'equity', normalBalance: 'credit' },
      { code: '1061', name: 'Réserve légale', type: 'equity', normalBalance: 'credit' },
      { code: '1068', name: 'Autres réserves', type: 'equity', normalBalance: 'credit' },
      { code: '110', name: 'Report à nouveau – solde créditeur', type: 'equity', normalBalance: 'credit' },
      { code: '120', name: 'Résultat de l\'exercice – bénéfice', type: 'equity', normalBalance: 'credit' },
      { code: '164', name: 'Emprunts auprès des établissements de crédit', type: 'liability', normalBalance: 'credit' },
      { code: '401', name: 'Fournisseurs', type: 'liability', normalBalance: 'credit' },
      { code: '408', name: 'Fournisseurs – Factures non parvenues', type: 'liability', normalBalance: 'credit' },
      { code: '411', name: 'Clients', type: 'asset', normalBalance: 'debit' },
      { code: '418', name: 'Clients – Produits non encore facturés', type: 'asset', normalBalance: 'debit' },
      { code: '44551', name: 'TVA à décaisser', type: 'liability', normalBalance: 'credit' },
      { code: '44566', name: 'TVA sur autres biens et services', type: 'asset', normalBalance: 'debit' },
      { code: '44571', name: 'TVA collectée', type: 'liability', normalBalance: 'credit' },
      { code: '512', name: 'Banques', type: 'asset', normalBalance: 'debit' },
      { code: '530', name: 'Caisse', type: 'asset', normalBalance: 'debit' },
      { code: '211', name: 'Terrains', type: 'asset', normalBalance: 'debit' },
      { code: '213', name: 'Constructions', type: 'asset', normalBalance: 'debit' },
      { code: '215', name: 'Installations techniques, matériel et outillage', type: 'asset', normalBalance: 'debit' },
      { code: '2183', name: 'Matériel de bureau et informatique', type: 'asset', normalBalance: 'debit' },
      { code: '281', name: 'Amortissements des immobilisations corporelles', type: 'asset', normalBalance: 'credit' },
      { code: '601', name: 'Achats stockés – Matières premières', type: 'expense', normalBalance: 'debit' },
      { code: '606', name: 'Achats non stockés de matières et fournitures', type: 'expense', normalBalance: 'debit' },
      { code: '6061', name: 'Fournitures non stockables (eau, énergie)', type: 'expense', normalBalance: 'debit' },
      { code: '607', name: 'Achats de marchandises', type: 'expense', normalBalance: 'debit' },
      { code: '613', name: 'Locations', type: 'expense', normalBalance: 'debit' },
      { code: '6226', name: 'Honoraires', type: 'expense', normalBalance: 'debit' },
      { code: '624', name: 'Transports de biens', type: 'expense', normalBalance: 'debit' },
      { code: '626', name: 'Frais postaux et de télécommunications', type: 'expense', normalBalance: 'debit' },
      { code: '631', name: 'Impôts, taxes sur rémunérations', type: 'expense', normalBalance: 'debit' },
      { code: '633', name: 'Autres impôts sur rémunérations', type: 'expense', normalBalance: 'debit' },
      { code: '635', name: 'Autres impôts, taxes et versements assimilés', type: 'expense', normalBalance: 'debit' },
      { code: '641', name: 'Rémunérations du personnel', type: 'expense', normalBalance: 'debit' },
      { code: '645', name: 'Cotisations de sécurité sociale et de prévoyance', type: 'expense', normalBalance: 'debit' },
      { code: '661', name: 'Charges d\'intérêts', type: 'expense', normalBalance: 'debit' },
      { code: '681', name: 'Dotations aux amortissements', type: 'expense', normalBalance: 'debit' },
      { code: '695', name: 'Impôts sur les bénéfices', type: 'expense', normalBalance: 'debit' },
      // 706 first: postingDefaults picks the first income account as the
      // default sales account — for the service-focused SME market that is
      // 'Prestations de services', not the product-sales account (701)
      { code: '706', name: 'Prestations de services', type: 'income', normalBalance: 'credit' },
      { code: '701', name: 'Ventes de produits finis', type: 'income', normalBalance: 'credit' },
      { code: '707', name: 'Ventes de marchandises', type: 'income', normalBalance: 'credit' },
      { code: '708', name: 'Produits des activités annexes', type: 'income', normalBalance: 'credit' },
    ],
    // debtors account for invoice postings (PCG 411 Clients)
    debtorsAccount: '411',
    bankAccountDefault: '512',
    inferTaxonomy: null,
    // statutoryAccounts omitted — PCG bilan layout is a B-milestone
  },

  compliance: {
    // B-milestone: CA3 monthly (24th of the following month) and RSI
    // semi-annual acomptes (July 55% / Dec 40% + CA12 by the 2nd working
    // day after 1 May) deadline rules need verification + calendar
    // support; the FR calendar is intentionally empty until then
    filingTypes: [],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // FR is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    // invoiceCompliance omitted — CGI art. 289 mention obligatoires rule
    // set is a B-milestone (invoice finalization fails loudly until then)
    eInvoicing: 'peppol-bis-3.0', // EN 16931 UBL; FR mandate (Factur-X/UBL
    // via PDPs) accepts it; scheme 0002 = SIREN from identifiers
    // auditFile omitted — FEC (CGI art. 54 septies) is a B-milestone
    languages: ['fr', 'en'],
    defaultLanguage: 'fr',
  },

  closing: { resultAccount: '120', equityAccount: '110' },
};
