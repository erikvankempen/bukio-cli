# Portugal — bukio jurisdiction profile (PT)

Phase D profile. Research verified 15 August 2026. Confidence per item.

## 1. Tax system

IVA (Imposto sobre o Valor Acrescentado) — EU member, SEPA, EUR.

| Item | Value | Source |
|---|---|---|
| Standard rate | **23 %** (mainland) | CIVA; Avalara/PwC 2026 |
| Reduced rates | **13 %**, **6 %** (mainland; Madeira 22/12/5, Azores 16/9/4) | CIVA |
| Small business | **Regime de isenção** (art. 53 CIVA): exemption below the ~€15,000 annual turnover threshold | CIVA |
| Reverse charge | Intra-Community B2B + domestic RE list (art. 2 CIVA) | AT (Autoridade Tributária) |
| Registration | NIPC required; IVA registration above the exemption threshold | AT |

Confidence: high.

## 2. VAT returns (Declaração Periódica)

| Item | Value | Source |
|---|---|---|
| Return | **Declaração Periódica (DP)** — monthly (turnover > €650K or intra-Community) or **quarterly** (SMEs) | AT; PwC 2026 |
| Monthly DP | Due by the **20th of the second month** following the period (Jan → 20 Mar) | Avalara; PwC 2026 |
| Quarterly DP | Due by the **20th of the second month** after the quarter: Q1 → **20 May**, Q2 → **20 Aug**, Q3 → **20 Nov**, Q4 → **20 Feb** next year (Q2 and the June monthly return share the 20 Aug date) | PwC 2026; taxclara |
| Payment | Due by the **25th** of the month after the filing deadline | Avalara |
| Annual | **Modelo 22** (IRC, 21 % CIT): due **31 May**; **IES** (annual accounts + tax info): due **15 July** | EA Contabilidade 2026; Portutax |

Confidence: high.

## 3. Identifiers

| Item | Value | Source |
|---|---|---|
| VAT number / NIPC | `PT` + **9 digits** (PT501234567) | AT |
| Company register | Registo Comercial / NIPC is the primary identifier | Registo Comercial |
| Peppol EAS | **9946** (Portugal VAT number) | Peppol BIS 3.0 EAS codelist |
| Bank | IBAN (PT…), SEPA | — |

Confidence: high.

## 4. Chart of accounts

**SNC (Sistema de Normalização Contabilística)** — official chart (Decreto-Lei
158/2009 + CNC Plano de Contas Multidimensional). The base accounts are 2-digit
classes with hierarchical sub-accounts; the default chart uses the official
codes with 2-digit bases zero-padded to bukio's 4-digit form (0011 Caixa,
0012 Depósitos à ordem, 0021 Clientes, 0022 Fornecedores, 0024 Estado e outros
entes públicos, 0031 Compras, 0043 Ativos fixos tangíveis, 0051 Capital, 0056
Resultados transitados, 8181 Resultado líquido do período) plus the 4-digit
VAT accounts (2432 IVA dedutível, 2433 IVA liquidado, 2434 IVA regularizações)
and the classic P&L accounts (61 CMVMC, 62 FSE, 63 Gastos com o pessoal, 64
Gastos de depreciação, 65 Gastos de financiamento, 71 Vendas, 72 Prestações de
serviços, 78 Outros rendimentos).

Confidence: high (official CNC plan; codes cross-checked against the CNC
Plano de Contas Multidimensional PDF).

## 5. Legal forms & fiscal year

- Forms: Lda (sociedade por quotas), SA, Unipessoal (uni), Empresário em Nome
  Individual (eni)
- Fiscal year: calendar year (12-31) for the vast majority
- Annual accounts: submitted via **IES** by 15 July (calendar year)
- IRC 21 % (Modelo 22, 31 May)

Confidence: high.

## 6. E-invoicing / Peppol

- **e-faturação / ATCUD** — mandatory invoice code (ATCUD) + QR on invoices
  since 1 Jan 2022; SAF-T PT filing for large companies. Both are
  format/compliance features — **B-milestones**.
- Portugal IS a Peppol participant (EAS 9946) — the existing UBL pipeline
  applies to cross-border B2B.

Confidence: high.

## 7. B-milestones (not registered — strict dispatch fails loudly)

- `tax.returnLayout` — Declaração Periódica return engine (B-milestone)
- `reporting.format` — demonstrações financeiras (SNC layout) (B-milestone)
- `documents.auditFile` — SAF-T PT is a different XML schema (B-milestone)
- `documents.invoiceCompliance` — **registered: the art. 226 EU baseline**
  ('eu-invoice-vereisten'); CIVA additions are a B-milestone
- ATCUD emission — B-milestone

## 8. pt.js mapping

| bukio field | Value |
|---|---|
| meta.country / baseCurrency / locale | PT / EUR / pt |
| meta.legalForms | lda, sa, uni, eni |
| identifiers.vatIdFormat | /^PT\d{9}$/ |
| identifiers.peppolSchemeId | 9946 |
| tax.standardRateBp / codes | 2300 / 23, 13, 6, V, R, RE, M, P |
| tax.accounts.ledger | [2432 IVA dedutível, 2433 IVA liquidado] |
| tax.accounts.fileDefault / differenceDefault | 2434 IVA regularizações / 2434 |
| reporting.defaultChart | SNC subset, ~28 accounts (2-digit bases zero-padded) |
| reporting.debtorsAccount / bankAccountDefault | 0021 / 0012 |
| compliance.filingTypes | IVA_DP (YYYY-Qn, pt-dp-quarterly), IRC (YYYY, pt-irc), CONTAS_ANUAIS (YYYY, pt-ies) |
| documents.eInvoicing | peppol-bis-3.0 (cross-border; ATCUD B-milestone) |
| closing | result 8181 → equity 0056 |
