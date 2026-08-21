/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Spain jurisdiction profile (Phase D — ES profile).
//
// Data sources: research brief at docs-research/es-profile.md — IVA rates
// (21/10/4, AEAT), Modelo 303 quarterly schedule (Q1 20 Apr, Q2 20 Jul,
// Q3 20 Oct, Q4 30 Jan next year — first 20 natural days after the quarter),
// Modelo 390 annual 30 Jan, Modelo 200 CIT within 25 days of the 6 months
// after FYE, cuentas anuales deposit 7 months after FYE, NIF format,
// Peppol EAS codelist (9920 = AEAT NIF). The chart is the OFFICIAL Plan
// General Contable (R.D. 1514/2007) SME subset — PGC codes are statutory.
//
// Phase D scope discipline (same contract as every market): register ONLY
// what maps to existing generic engines; everything else fails loudly:
//   - tax.returnLayout      omitted → OB readout fails (the Modelo 303
//                            engine is a B-milestone)
//   - reporting.format      omitted → financial statements fail (cuentas
//                            anuales PGC layout is a B-milestone)
//   - documents.auditFile   omitted → XAF export fails (no Spanish SAF-T;
//                            e-audit via SII is for large companies only)
//   - documents.invoiceCompliance → the art. 226 EU baseline is
//                            registered ('eu-invoice-vereisten'); art. 6-7
//                            Ley 37/1992 additions are a B-milestone
//   - Verifactu emission    invoicing-software obligation (hash-chain
//                            XML/QR, 2025+) — B-milestone; 'peppol-bis-3.0'
//                            is registered for CROSS-BORDER invoices (Spain
//                            is a Peppol participant, EAS 9920)
// Registered: SEPA, CAMT.053, ECB, closing 129 -> 121.

export default {
  meta: {
    country: 'ES',
    baseCurrency: 'EUR',
    locale: 'es',
    legalForms: ['sl', 'slu', 'sa', 'autonomo'],
    defaultFiscalYearEnd: '12-31',
  },

  identifiers: {
    companyIdLabel: 'registration_id', // Registro Mercantil — the NIF is the primary identifier
    vatIdLabel: 'tax_id',
    vatIdFormat: /^ES[A-Z0-9]\d{7}[A-Z0-9]$/i, // NIF: ES + letter/N + 7 digits + check char
    // 9920 = Agencia Española de Administración Tributaria (EAS)
    peppolSchemeId: '9920',
    accountNumber: { kind: 'iban' },
  },

  tax: {
    system: 'vat',
    standardRateBp: 2100,
    // Recargo de equivalencia: retailers ≤ €1M (VAT charged by the supplier)
    smallBusinessScheme: 'recargo-equivalencia',
    // ES rates (Ley 37/1992): 21% standard / 10% reduced / 4% super-reduced.
    // Intra-Community B2B reverse charge (art. 84) applies; domestic RE for
    // construction etc. No general 0% category (exports are no sujeto/exento).
    codes: [
      { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% tipo general' },
      { code: '10', rateBp: 1000, type: 'standard', euReverse: 0, description: '10% tipo reducido' },
      { code: '4', rateBp: 400, type: 'standard', euReverse: 0, description: '4% tipo superreducido' },
      { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Exento (art. 20-25 Ley 37/1992)' },
      { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Inversión del sujeto pasivo (art. 84)' },
      { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Adquisición intracomunitaria' },
      { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Régimen especial de bienes usados' },
      { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Uso privado / autoconsumo' },
    ],
    accounts: {
      // PGC official codes: 472 HP IVA soportado (input, asset) / 477 HP IVA
      // repercutido (output, liability); the settlement account 475 HP
      // acreedora por conceptos fiscales absorbs the 303 balance
      ledger: [
        { code: '472', name: 'H.P. IVA soportado', type: 'asset', normalBalance: 'debit' },
        { code: '477', name: 'H.P. IVA repercutido', type: 'liability', normalBalance: 'credit' },
      ],
      fileDefault: '475',
      differenceDefault: '475', // 303 balance/rounding lands on 475 H.P. acreedora
      settlementAccountName: 'H.P. acreedora por IVA (Modelo 303)',
    },
    // returnLayout omitted — the Modelo 303 engine is a B-milestone
    filingPeriodicity: 'quarterly', // monthly for gran empresa > €6M
    reverseChargeEffectiveRateBp: 2100,
  },

  reporting: {
    // format omitted — cuentas anuales (PGC layout) is a B-milestone
    taxonomy: null,
    // PGC (R.D. 1514/2007) SME subset — official statutory codes
    defaultChart: [
      { code: '570', name: 'Caja, euros', type: 'asset', normalBalance: 'debit' },
      { code: '572', name: 'Bancos e instituciones de crédito c/c', type: 'asset', normalBalance: 'debit' },
      { code: '430', name: 'Clientes', type: 'asset', normalBalance: 'debit' },
      { code: '440', name: 'Deudores', type: 'asset', normalBalance: 'debit' },
      { code: '472', name: 'H.P. IVA soportado', type: 'asset', normalBalance: 'debit' },
      { code: '216', name: 'Mobiliario', type: 'asset', normalBalance: 'debit' },
      { code: '217', name: 'Equipos para procesos de información', type: 'asset', normalBalance: 'debit' },
      { code: '281', name: 'Amortización acumulada del inmovilizado material', type: 'asset', normalBalance: 'credit' },
      { code: '400', name: 'Proveedores', type: 'liability', normalBalance: 'credit' },
      { code: '410', name: 'Acreedores por prestaciones de servicios', type: 'liability', normalBalance: 'credit' },
      { code: '475', name: 'H.P. acreedora por conceptos fiscales', type: 'liability', normalBalance: 'credit' },
      { code: '476', name: 'Organismos de la Seguridad Social acreedores', type: 'liability', normalBalance: 'credit' },
      { code: '477', name: 'H.P. IVA repercutido', type: 'liability', normalBalance: 'credit' },
      { code: '100', name: 'Capital social', type: 'equity', normalBalance: 'credit' },
      { code: '112', name: 'Reserva legal', type: 'equity', normalBalance: 'credit' },
      { code: '121', name: 'Resultados negativos de ejercicios anteriores', type: 'equity', normalBalance: 'credit' },
      { code: '129', name: 'Resultado del ejercicio', type: 'equity', normalBalance: 'credit' },
      // 700 first: postingDefaults picks the first income account — Ventas
      // de mercaderías (21% goods); 705 Prestaciones de servicios follows
      { code: '700', name: 'Ventas de mercaderías', type: 'income', normalBalance: 'credit' },
      { code: '705', name: 'Prestaciones de servicios', type: 'income', normalBalance: 'credit' },
      { code: '778', name: 'Ingresos excepcionales', type: 'income', normalBalance: 'credit' },
      { code: '600', name: 'Compras de mercaderías', type: 'expense', normalBalance: 'debit' },
      { code: '621', name: 'Arrendamientos y cánones', type: 'expense', normalBalance: 'debit' },
      { code: '622', name: 'Reparaciones y conservación', type: 'expense', normalBalance: 'debit' },
      { code: '623', name: 'Servicios de profesionales independientes', type: 'expense', normalBalance: 'debit' },
      { code: '624', name: 'Transportes', type: 'expense', normalBalance: 'debit' },
      { code: '625', name: 'Primas de seguros', type: 'expense', normalBalance: 'debit' },
      { code: '626', name: 'Servicios bancarios y similares', type: 'expense', normalBalance: 'debit' },
      { code: '627', name: 'Publicidad, propaganda y relaciones públicas', type: 'expense', normalBalance: 'debit' },
      { code: '628', name: 'Suministros', type: 'expense', normalBalance: 'debit' },
      { code: '629', name: 'Otros servicios', type: 'expense', normalBalance: 'debit' },
      { code: '640', name: 'Sueldos y salarios', type: 'expense', normalBalance: 'debit' },
      { code: '642', name: 'Seguridad social a cargo de la empresa', type: 'expense', normalBalance: 'debit' },
      { code: '662', name: 'Intereses de deudas', type: 'expense', normalBalance: 'debit' },
      { code: '681', name: 'Amortización del inmovilizado material', type: 'expense', normalBalance: 'debit' },
      { code: '630', name: 'Impuesto sobre beneficios', type: 'expense', normalBalance: 'debit' },
    ],
    // debtors account for invoice postings (PGC 430 Clientes)
    debtorsAccount: '430',
    bankAccountDefault: '572',
    inferTaxonomy: null,
    // statutoryAccounts omitted — cuentas anuales layout is a B-milestone
  },

  compliance: {
    // Modelo 303 quarterly (first 20 days after the quarter; Q4 until
    // 30 Jan next year) + Modelo 390 annual summary (30 Jan). Modelo 200
    // (Impuesto sobre Sociedades, 25%): within 25 days of the 6 months
    // after FYE. Cuentas anuales: approved within 6 months, deposited at
    // the Registro Mercantil within 1 month of approval (7 months after
    // FYE).
    filingTypes: [
      { type: 'IVA_TRIMESTRAL', periodShape: 'YYYY-Qn', deadlineRule: 'es-303-quarterly' },
      { type: 'IVA_ANUAL', periodShape: 'YYYY', deadlineRule: 'es-390' },
      { type: 'IMPUESTO_SOCIEDADES', periodShape: 'YYYY', deadlineRule: 'es-200' },
      { type: 'CUENTAS_ANUALES', periodShape: 'YYYY', deadlineRule: 'es-7-months' },
    ],
  },

  exchange: {
    bankStatementFormats: ['camt.053', 'csv'],
    paymentFormats: ['sepa-pain.001', 'sepa-pain.008'], // Spain is SEPA core
    fxSource: 'ecb',
    baseCurrency: 'EUR',
  },

  documents: {
    invoiceCompliance: 'eu-invoice-vereisten', // art. 226 baseline (art. 6-7 Ley 37/1992 additions are a B-milestone)
    eInvoicing: 'peppol-bis-3.0', // cross-border Peppol (EAS 9920); Verifactu
    // software obligation (2025+) is a B-milestone
    // auditFile omitted — no Spanish SAF-T
    languages: ['es', 'en'],
    defaultLanguage: 'es',
  },

  closing: { resultAccount: '129', equityAccount: '121' },
};
