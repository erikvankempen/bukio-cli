# Romania — bukio jurisdiction profile (RO)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **18 %** (reduced from 19 % effective 1 Jan 2026) | firmaromania 2026; EC 2026 (high) |
| Reduced rate | **9 %** — foodstuffs, accommodation, medicines | EC (high) |
| Reduced rate | **5 %** — certain foods (removed/limited in 2025; verify per product) | EC / firmaromania (medium) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold RON 300,000 (≈ €60K) | vatcalc RO (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `RO` + **2 to 10 digits** — `/^RO\d{2,10}$/` (CUI with RO prefix) | Avalara EU VAT formats (high) |
| Company number | **CUI / CIF** (cod unic de înregistrare) — 2-10 digits, issued by ANAF; the J-register (Registrul Comerțului) number; label 'cui' | ONRC practice (high) |
| Peppol EAS | **9947** — `RO:VAT` "Romania VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **SRL** — societate cu răspundere limitată (limited liability, dominant)
- **SA** — societate pe acțiuni (joint-stock)
- **SNC / SCA** — general / limited partnership
- **PFA / II** — sole trader (persoană fizică autorizată / întreprindere individuală)

Confidence: high (standard Legea 31/1990 forms).

## 4. Chart of accounts

Romania has a **statutory chart** — the Planul de conturi general (MF order, class 1-8). The VAT accounts 4423 (payable) / 4424 (recoverable) are statutory; the settlement is 4426:

| Code | Account | Code | Account |
|---|---|---|---|
| 5121 | Conturi la bănci (Bank) | 1012 | Capital subscris vărsat (Share capital) |
| 5311 | Casa (Cash) | 1171 | Rezultatul reportat (Retained earnings) |
| 4111 | Clienți (Trade receivables) | 1211 | Profit sau pierdere (Result for the year) |
| 4011 | Furnizori (Trade payables) | 7010 | Venituri din vânzări (Sales revenue) |
| 4210 | Personal — datorii (Employee payables) | 7080 | Alte venituri (Other income) |
| 4424 | TVA de recuperat (input VAT) | 6010 | Cheltuieli cu materiile prime (Materials) |
| 4423 | TVA de plată (output VAT) | 6280 | Alte cheltuieli (Services/other expenses) |
| 4426 | TVA — decontare (VAT settlement) | 6410 | Cheltuieli cu salariile (Personnel costs) |
| 2120 | Mijloace fixe (Fixed assets) | 6811 | Cheltuieli cu amortizarea (Depreciation) |
| 2810 | Amortizare mijloace fixe (Accumulated depreciation) | 6660 | Cheltuieli cu dobânzile (Finance costs) |
| 3710 | Mărfuri (Inventory) | 6910 | Cheltuieli cu impozitul pe profit (Corporate tax) |

Ledger pair 4424 (input, debit) / 4423 (output, credit); settlement 4426. Closing: 1211 → 1171. Confidence: medium (statutory-plan skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (D300) | **Monthly** by the **25th** of the following month (quarterly for small taxpayers); e-Factura-based filing | Avalara RO; Marosa (high) |
| Annual accounts | Financial statements filed with the Ministry of Finance within **150 days** of FYE (~30 May for calendar year) | firmaromania 2026 (high) |
| Corporate income tax (D101) | Annual return + payment due **25 June** of the following year (quarterly prepayments) | firmaromania 2026; PwC RO (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: NO** — Romania does NOT participate in Peppol; domestic e-invoicing runs on the **e-Factura** national system (mandatory B2B since 2024). Cross-border invoices still emit EN 16931 UBL via the generic builder, but there is no EAS scheme for the Peppol SEND path; e-Factura integration is a B-milestone.

## 7. Payment

- **SEPA** member; **RON** (leu) — own currency, ECB reference rates (baseCurrency 'RON'). Confidence: high (ECB).

## 8. Gotchas

- Standard rate **18 %** from 1 Jan 2026 (cut from 19 %).
- RON currency.
- **No Peppol participation** — the only Phase F market without an EAS scheme; e-Factura is the domestic route (B-milestone).
- e-Factura mandatory B2B since 2024; D300 monthly by the 25th.