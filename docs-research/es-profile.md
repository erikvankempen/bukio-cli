# Spain — bukio jurisdiction profile (ES)

Phase D profile. Research verified 15 August 2026. Confidence per item.

## 1. Tax system

IVA (Impuesto sobre el Valor Añadido) — EU member, SEPA, EUR.

| Item | Value | Source |
|---|---|---|
| Standard rate | **21 %** | AEAT; standard since 2012 |
| Reduced rates | **10 %**, **4 %** (super-reduced) | AEAT |
| Small business | **Recargo de equivalencia** (retailers ≤ €1M — VAT is charged by the supplier); régimen simplificado for agriculture/ganadería | AEAT |
| Reverse charge | Intra-Community B2B (art. 84 Ley 37/1992); domestic RE for construction etc. | AEAT |
| Registration | NIF required for any activity; IVA registration from first supply | AEAT |

Confidence: high.

## 2. VAT returns (Modelo 303 / 390)

| Item | Value | Source |
|---|---|---|
| Return | **Modelo 303** (quarterly; monthly for gran empresa > €6M) + **Modelo 390** (annual summary) | AEAT |
| 303 quarterly dates | Q1 → **20 April**, Q2 → **20 July**, Q3 → **20 October**, Q4 → **30 January** next year (first 20 natural days after the quarter; Q4 until 30 Jan) | Radar Fiscal; Avalara; Holded 2026 |
| 390 annual | **30 January** of the following year | lextax 2026 |
| CIT | **Modelo 200** (Impuesto sobre Sociedades, 25 %): within **25 days of the 6 months** after FYE (calendar year → 25 July) | AEAT |

Confidence: high (multiple 2026 sources agree on the 303 schedule).

## 3. Identifiers

| Item | Value | Source |
|---|---|---|
| VAT number / NIF | `ES` + letter + 7 digits + check letter (companies: B12345678; general NIF `[A-Z0-9]` + 7 digits + `[A-Z0-9]`) | AEAT |
| Company register | Registro Mercantil number — free text (NIF is the primary identifier) | Registro Mercantil |
| Peppol EAS | **9920** (AEAT NIF) | Peppol BIS 3.0 EAS codelist |
| Bank | IBAN (ES…), SEPA | — |

Confidence: high.

## 4. Chart of accounts

**Plan General Contable (PGC)** — official statutory chart (R.D. 1514/2007,
subgroups 1-9; PGC PYMES for SMEs). The default chart is the SME subset:
570 Caja, 572 Bancos c/c, 430 Clientes, 440 Deudores, 472 HP IVA soportado
(input), 477 HP IVA repercutido (output), 400 Proveedores, 475 HP acreedora
por conceptos fiscales, 476 OOSS acreedores, 100 Capital social, 112 Reserva
legal, 129 Resultado del ejercicio, 700 Ventas de mercaderías, 705 Prestaciones
de servicios, 600 Compras de mercaderías, 621-629 servicios exteriores,
640 Sueldos y salarios, 642 Seguridad social, 681 Amortización inmovilizado,
662 Intereses de deudas, 630 Impuesto sobre beneficios.

Confidence: high (official PGC codes, cross-checked in multiple texts).

## 5. Legal forms & fiscal year

- Forms: SL (sociedad limitada), SLU (unipersonal), SA, autónomo
- Fiscal year: calendar year (12-31) for the vast majority
- Annual accounts (**cuentas anuales**): approved within 6 months of FYE,
  deposited at the Registro Mercantil within 1 month of approval → **7 months**
  after FYE (31 July for calendar year)
- CIT 25 % (Modelo 200, above)

Confidence: high.

## 6. E-invoicing / Peppol

- **Verifactu** — invoicing-software obligation (hash-chain XML/QR) from 1 Jan
  2025; **Ley Crea y Crece** B2B e-invoicing platform mandate (phased, delayed
  to 2026/2027). Both are software/format features — **B-milestones**.
- Spain IS a Peppol participant (EAS 9920) — the existing UBL pipeline applies
  to cross-border B2B.

Confidence: high.

## 7. B-milestones (not registered — strict dispatch fails loudly)

- `tax.returnLayout` — Modelo 303 return engine (B-milestone)
- `reporting.format` — cuentas anuales (PGC layout) (B-milestone)
- `documents.auditFile` — no Spanish SAF-T (e-audit via AEAT SII for large
  companies only) (B-milestone)
- `documents.invoiceCompliance` — **registered: the art. 226 EU baseline**
  ('eu-invoice-vereisten'); art. 6-7 Ley 37/1992 additions are a B-milestone
- Verifactu emission — B-milestone

## 8. es.js mapping

| bukio field | Value |
|---|---|
| meta.country / baseCurrency / locale | ES / EUR / es |
| meta.legalForms | sl, slu, sa, autonomo |
| identifiers.vatIdFormat | /^ES[A-Z0-9]\d{7}[A-Z0-9]$/i |
| identifiers.peppolSchemeId | 9920 |
| tax.standardRateBp / codes | 2100 / 21, 10, 4, V, R, RE, M, P |
| tax.accounts.ledger | [472 HP IVA soportado, 477 HP IVA repercutido] |
| tax.accounts.fileDefault / differenceDefault | 475 HP acreedora / 475 |
| reporting.defaultChart | PGC SME subset, ~30 accounts |
| reporting.debtorsAccount / bankAccountDefault | 430 / 572 |
| compliance.filingTypes | IVA_TRIMESTRAL (YYYY-Qn, es-303-quarterly), IVA_ANUAL (YYYY, es-390), IMPUESTO_SOCIEDADES (YYYY, es-200), CUENTAS_ANUALES (YYYY, es-7-months) |
| documents.eInvoicing | peppol-bis-3.0 (cross-border; Verifactu B-milestone) |
| closing | result 129 → equity 121 |
