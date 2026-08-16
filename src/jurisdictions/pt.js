/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across sixteen jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Portugal jurisdiction profile (Phase D — PT profile).
//
// Data sources: research brief at docs-research/pt-profile.md — IVA rates
// (23/13/6 mainland; island rates differ), Declaração Periódica deadlines
// (20th of the SECOND month after the period — monthly for turnover
// > €650K/intra-Community, quarterly for SMEs; payment by the 25th),
// Modelo 22 (IRC) 31 May, IES 15 July, NIPC format, Peppol EAS codelist
// (9946 = Portugal VAT number). The chart is the OFFICIAL SNC (Decreto-Lei
// 158/2009 + CNC Plano de Contas Multidimensional) — 2-digit base classes
// zero-padded to bukio's 4-digit form (0011 Caixa, 0012 Depósitos à ordem,
// 0021 Clientes, 0022 Fornecedores, 0024 Estado e outros entes públicos,
// 0031 Compras, 0043 Ativos fixos tangíveis, 0051 Capital, 0056 Resultados
// transitados, 8181 Resultado líquido do período) plus the 4-digit VAT
// accounts (2432 IVA dedutível, 2433 IVA liquidado, 2434 IVA regularizações).
//
// Phase D scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (the Declaração
//                            Periódica engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail
//                            (demonstrações financeiras SNC layout is a
//                            B-milestone)
//   - documents.auditFile   omitted → XAF export fails (SAF-T PT is a
//                            different XML schema)
//   - documents.invoiceCompliance → the art. 226 EU baseline is
//                            registered ('eu-invoice-vereisten'); CIVA
//                            additions are a B-milestone
//   - ATCUD emission        mandatory invoice code since 1 Jan 2022 —
//                            B-milestone; 'peppol-bis-3.0' is registered
//                            for CROSS-BORDER invoices (Portugal is a
//                            Peppol participant, EAS 9946)
// Registered: SEPA, CAMT.053, ECB, closing 8181 -> 0056.

export default {
  meta: {
    country: 'PT',
    baseCurrency: 'EUR',
    locale: 'pt',
    legalForms: ['lda', 'sa', 'uni', 'eni'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Registo Comercial — the NIPC is the primary identifier
    vatIdLabel: 'tax_id',
    vatIdFormat: /^PT\d{9}$/i, // NIPC: PT + 9 digits
    // 9946 = Portugal VAT number (EAS)
    peppolSchemeId: '9946',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2300,
    // Regime de isenção (art. 53 CIVA): exemption below ~€15,000 turnover
    smallBusinessScheme: 'threshold',
    // PT rates (CIVA): 23% standard / 13% reduced / 6% intermediate
    // (mainland; Madeira 22/12/5, Azores 16/9/4). Intra-Community B2B
    // reverse charge applies; no general 0% category (exports are
    // isento/não sujeito).
    codes: [
      { code: '23', rateBp: 2300, type: 'standard', euReverse: 0, description: '23% taxa normal' },
      { code: '13', rateBp: 1300, type: 'standard', euReverse: 0, description: '13% taxa intermédia' },
      { code: '6', rateBp: 600, type: 'standard', euReverse: 0, description: '6% taxa reduzida' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Isento (art. 9 CIVA)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Autoliquidação (art. 2 CIVA)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Aquisição intracomunitária' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Regime da margem' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Uso privado / autoconsumo' },
    ],
    accounts: {
      // SNC official codes: 2432 IVA dedutível (input, asset) / 2433 IVA
      // liquidado (output, liability); the settlement account 2434 IVA
      // regularizações absorbs the DP return balance
      ledger: [
        { code: '2432', name: 'IVA dedutível', type: 'asset', normalBalance: 'debit' },
        { code: '2433', name: 'IVA liquidado', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '2434',
      differenceDefault: '2434', // DP balance/rounding lands on IVA regularizações
      settlementAccountName: 'IVA a pagar ao Estado (DP)',
    },
    // returnLayout omitted — the Declaração Periódica engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly for turnover > €650K / intra-Community
    reverseChargeEffectiveRateBp: 2300,
  },

  reporting: {
    // format omitted — demonstrações financeiras (SNC layout) is a
    // B-milestone
    taxonomy: null,
    // SNC official chart (DL 158/2009 + CNC) — 2-digit base classes
    // zero-padded to 4 digits (same treatment as the AT EKR short forms);
    // the VAT accounts and the result account are naturally 4-digit
    defaultChart: [
      { code: '0012', name: 'Depósitos à ordem', type: 'asset', normalBalance: 'debit' },
      { code: '0011', name: 'Caixa', type: 'asset', normalBalance: 'debit' },
      { code: '0021', name: 'Clientes', type: 'asset', normalBalance: 'debit' },
      { code: '0022', name: 'Fornecedores', type: 'liability', normalBalance: 'credit' },
      { code: '0024', name: 'Estado e outros entes públicos', type: 'liability', normalBalance: 'credit' },
      { code: '2432', name: 'IVA dedutível', type: 'asset', normalBalance: 'debit' },
      { code: '2433', name: 'IVA liquidado', type: 'liability', normalBalance: 'credit' },
      { code: '2434', name: 'IVA regularizações', type: 'liability', normalBalance: 'credit' },
      { code: '0043', name: 'Ativos fixos tangíveis', type: 'asset', normalBalance: 'debit' },
      { code: '0438', name: 'Amortizações acumuladas', type: 'asset', normalBalance: 'credit' },
      { code: '0051', name: 'Capital', type: 'equity', normalBalance: 'credit' },
      { code: '0056', name: 'Resultados transitados', type: 'equity', normalBalance: 'credit' },
      { code: '8181', name: 'Resultado líquido do período', type: 'equity', normalBalance: 'credit' },
      // 0071 first: postingDefaults picks the first income account — Vendas
      // (23% goods); 0072 Prestações de serviços follows
      { code: '0071', name: 'Vendas', type: 'income', normalBalance: 'credit' },
      { code: '0072', name: 'Prestações de serviços', type: 'income', normalBalance: 'credit' },
      { code: '0078', name: 'Outros rendimentos e ganhos', type: 'income', normalBalance: 'credit' },
      { code: '0061', name: 'CMVMC', type: 'expense', normalBalance: 'debit' },
      { code: '0062', name: 'Fornecimentos e serviços externos', type: 'expense', normalBalance: 'debit' },
      { code: '0063', name: 'Gastos com o pessoal', type: 'expense', normalBalance: 'debit' },
      { code: '0064', name: 'Gastos de depreciação e de amortização', type: 'expense', normalBalance: 'debit' },
      { code: '0065', name: 'Gastos de financiamento', type: 'expense', normalBalance: 'debit' },
      { code: '0069', name: 'Outros gastos e perdas', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (SNC 21 Clientes)
    debtorsAccount: '0021',
    bankAccountDefault: '0012',
    inferTaxonomy: null,
    // statutoryAccounts omitted — SNC layout is a B-milestone
  },

  compliance: {
    // Declaração Periódica: due the 20th of the SECOND month after the
    // period — quarterly for SMEs (Q1 -> 20 May, Q2 -> 20 Aug, Q3 -> 20
    // Nov, Q4 -> 20 Feb next year), monthly above €650K / intra-Community.
    // Modelo 22 (IRC, 21%): 31 May. Annual accounts via IES: 15 July.
    filingTypes: [
      { type: 'IVA_DP', periodShape: 'YYYY-Qn', deadlineRule: 'pt-dp-quarterly' },
      { type: 'IRC', periodShape: 'YYYY', deadlineRule: 'pt-irc' },
      { type: 'CONTAS_ANUAIS', periodShape: 'YYYY', deadlineRule: 'pt-ies' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Portugal is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (CIVA additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9946); ATCUD
    // invoice code (2022+) is a B-milestone
    // auditFile omitted — SAF-T PT is a different XML schema
    languages: ['pt', 'en'],
    defaultLanguage: 'pt',
  },

  closing: { resultAccount: '8181', equityAccount: '0056' },
};
