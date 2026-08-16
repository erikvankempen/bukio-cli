# Italy — bukio jurisdiction profile (IT)

Phase D profile. Research verified 15 August 2026. Confidence per item.

## 1. Tax system

IVA (Imposta sul Valore Aggiunto) — EU member, SEPA, EUR.

| Item | Value | Source |
|---|---|---|
| Standard rate | **22 %** | Numeral/Taxenlight 2026 guides |
| Reduced rates | **10 %**, **5 %**, **4 %** | Numeral/Taxenlight 2026 |
| Small business | **Regime forfettario** (flat-rate regime) for turnover ≤ **€85,000** | Taxenlight 2026 |
| Reverse charge | B2B domestic reverse-charge list + intra-Community (§ art. 17/19 DPR 633/72) | Agenzia Entrate |
| Registration | Partita IVA mandatory for any business activity | Taxenlight 2026 |

Confidence: high.

## 2. VAT returns (liquidazione IVA)

| Item | Value | Source |
|---|---|---|
| Return | **Liquidazione IVA** (periodic settlement) + annual **Dichiarazione IVA** | Agenzia Entrate |
| Frequency | **Monthly** (16th of the following month) when prior-year turnover > €400K; **quarterly** below (versamento by the 16th of the second month after the quarter, +1% interest) | Agenzia Entrate scadenzario; tot.money 2026 |
| Quarterly dates | Q1 → **16 May**, Q2 → **16 Aug**, Q3 → **16 Nov**, Q4 → **16 Feb** next year (16th of the 2nd month after the quarter) | tot.money 2026; Agenzia Entrate |
| Annual return | **Dichiarazione IVA** due **30 April** of the following year | turbotasse; fiscoetasse |
| Payment | Via **F24** (the settlement form — payment rail, not a return) | Agenzia Entrate |

Confidence: high (Agenzia Entrate + 2026 commercial calendars).

## 3. Identifiers

| Item | Value | Source |
|---|---|---|
| VAT number (Partita IVA) | `IT` + **11 digits** (IT12345678901) | Agenzia Entrate |
| Company register | **REA** number (Registro Imprese / Camera di Commercio) — free text | Registro Imprese |
| Individual tax id | Codice Fiscale (16 chars) — 0210 EAS; companies use the Partita IVA (0211) | Peppol EAS codelist |
| Peppol EAS | **0211** (Partita IVA) | Peppol BIS 3.0 EAS codelist |
| Bank | IBAN (IT…), SEPA | — |

Confidence: high.

## 4. Chart of accounts

Italy has **no statutory chart** — every studio uses its own. The default chart
follows the common commercialisti convention (4-digit class scheme: 1xxx
liquidità/crediti, 2xxx debiti, 3xxx patrimonio netto, 4xxx ricavi, 5xxx costi).
Account names are the standard Italian ones (Cassa contanti, Banca c/c,
Crediti v/clienti, Debiti v/fornitori, IVA a credito, IVA a debito, Erario
c/IVA, Ricavi delle vendite e delle prestazioni, …). The CNDCEC published a
recommended scheme but it is not statutory and not uniformly used — the
convention chart is documented as such (same treatment as GB/IE).

Confidence: medium-high on names (universal), medium on the numeric scheme
(convention — documented).

## 5. Legal forms & fiscal year

- Forms: SRL, SRL semplificata, SPA, SNC, SAS, ditta individuale
- Fiscal year: calendar year (12-31) for the vast majority
- Annual accounts (**bilancio**): approved within 120 days of FYE (art. 2364
  c.c.), deposited at the Registro Imprese within 30 days of approval →
  **~5 months** after FYE (30 May for calendar year). Filed by ALL SRL/SPA.
- Corporate tax: IRES 24 % + IRAP ~3.9 % (not calendarised — Modelo 22-style
  filings are IRES/IRAP returns, B-milestone)

Confidence: high.

## 6. E-invoicing / Peppol

- **FatturaPA via SdI** (Sistema di Interscambio) — domestic e-invoicing
  **mandatory** for B2B since 1 Jan 2019 (and B2C since 1 Jan 2022). The
  FatturaPA XML (fatturapa_v1.2) is NOT Peppol UBL — a separate schema. The
  existing Peppol BIS UBL builder does NOT emit it → **FatturaPA export is a
  B-milestone** (new builder).
- Italy IS a Peppol participant for cross-border B2B (EAS 0211) — the
  existing UBL pipeline applies to cross-border invoices.

Confidence: high.

## 7. B-milestones (not registered — strict dispatch fails loudly)

- `tax.returnLayout` — liquidazione IVA return engine (B-milestone)
- `reporting.format` — bilancio (civil code layout) (B-milestone)
- `documents.auditFile` — no Italian SAF-T equivalent (B-milestone)
- `documents.invoiceCompliance` — **registered: the art. 226 EU baseline**
  ('eu-invoice-vereisten'); DPR 633/72 additions are a B-milestone
- **FatturaPA/SdI export** — domestic e-invoicing format (B-milestone);
  `documents.eInvoicing: 'peppol-bis-3.0'` registered for cross-border only

## 8. it.js mapping

| bukio field | Value |
|---|---|
| meta.country / baseCurrency / locale | IT / EUR / it |
| meta.legalForms | srl, srl-semplificata, spa, snc, sas, ditta-individuale |
| identifiers.vatIdFormat | /^IT\d{11}$/ |
| identifiers.peppolSchemeId | 0211 |
| tax.standardRateBp / codes | 2200 / 22, 10, 5, 4, V, R, RE, M, P |
| tax.accounts.ledger | [1300 IVA a credito, 2100 IVA a debito] |
| tax.accounts.fileDefault / differenceDefault | 2400 Erario c/IVA / 2400 |
| reporting.defaultChart | commercialisti convention, ~27 accounts |
| reporting.debtorsAccount / bankAccountDefault | 1200 / 1100 |
| compliance.filingTypes | LIQUIDAZIONE_IVA (YYYY-Qn, it-liquidazione-quarterly), DICHIARAZIONE_IVA (YYYY, it-30-apr), BILANCIO (YYYY, it-bilancio) |
| documents.eInvoicing | peppol-bis-3.0 (cross-border; FatturaPA B-milestone) |
| closing | result 3200 → equity 3100 |
