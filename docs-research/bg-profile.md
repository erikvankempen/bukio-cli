# Bulgaria — bukio jurisdiction profile (BG)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **20 %** | EC europa.eu VAT rates (high) |
| Reduced rate | **9 %** — hotel and restaurant accommodation | Avalara BG (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **€51,130** (BGN 100,000 fixed at the euro conversion rate 1.95583 — thresholds converted at €-adoption 1 Jan 2026) | vatcalc 2026; Bulgarian.LLC (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `BG` + **9 or 10 digits** — `/^BG\d{9,10}$/` (9-digit = the EIK/UIC; 10-digit variants exist for some taxpayers) | Avalara; bulgarian.llc (high) |
| Company number | **EIK / UIC (ЕИК)** — 9 digits, issued by the Registry Agency; label 'uic/eik' | Registry Agency (high) |
| Peppol EAS | **9926** — `BG:VAT` "Bulgaria VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **EOOD** (ЕООД) — single-member limited liability (dominant for 1-owner SMEs)
- **OOD** (ООД) — multi-member limited liability
- **AD** (АД) — joint-stock (public/private)
- **EAD** (ЕАД) — single-shareholder joint-stock
- **ET** (ЕТ) — sole trader

Confidence: high (standard Tърговски закон forms).

## 4. Chart of accounts

Bulgaria has a **statutory chart** — the Национален сметкоплан (National Chart of Accounts, approved by the Ministry of Finance). The bukio skeleton follows the NSS class structure (1 assets, 2 liabilities, 3 equity/revenues, 4 expenses) with standard Bulgarian account names:

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Каса (Cash) | 2000 | Основен капитал (Share capital) |
| 1010 | Банкови сметки (Bank accounts) | 2100 | Неразпределена печалба (Retained earnings) |
| 1200 | Клиенти (Trade receivables) | 2200 | Печалба (загуба) за годината (Result for the year) |
| 1300 | Доставчици (Trade payables) | 3000 | Приходи от продажби (Sales revenue) |
| 1410 | Задължения към персонала (Employee payables) | 3100 | Други приходи (Other income) |
| 1500 | ДДС за възстановяване — input VAT | 4000 | Разходи за материали (Materials) |
| 1510 | ДДС за внасяне — output VAT | 4100 | Разходи за услуги (Services) |
| 1520 | ДДС — разплащания с бюджета (VAT settlement) | 4200 | Разходи за персонала (Personnel costs) |
| 1800 | Дълготрайни активи (Fixed assets) | 4300 | Амортизации (Depreciation) |
| 1810 | Амортизация на ДА (Accumulated depreciation) | 4400 | Финансови разходи (Finance costs) |
| 1900 | Материални запаси (Inventory) | 4500 | Данъци (Corporate tax) |

Ledger pair 1500 (input, debit) / 1510 (output, credit); settlement 1520 cleared at filing. Closing: 2200 → 2100. Confidence: medium (skeleton is NSS-structured but simplified; NSS codes are conventionally 4-digit class-based).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | **Monthly**, due the **14th** of the following month (e-portal of NRA; payment same day). Small businesses may opt into quarterly + annual | Marosa due-date table (medium/high) |
| Annual accounts | Annual financial statements filed at the Trade Register by **30 June** of the following year | bulgarian.llc annual-return guide (high) |
| Corporate income tax | Annual CIT return (form 1010) filed between 1 March and **30 June** of the following year | innovires 2026; aidosbg (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9926** (`BG:VAT`); EU Directive 2027/903 B2B e-invoicing implementation in preparation. Confidence: high (EAS codelist).
- **Domestic mandate: none yet** (national e-invoicing discussions; 2027+ EU framework) — B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** since **1 Jan 2026** (eurozone adoption; conversion 1.95583 BGN/EUR). IBAN-based payments. Confidence: high (EC Access2Markets).

## 8. Gotchas

- Euro adoption on 1 Jan 2026 — books/amounts in EUR thereafter; thresholds converted at 1.95583.
- 9 % reduced rate applies to hotel/restaurant accommodation.
- EIK/UIC is both the company number and the base of the 9-digit VAT number.
- Annual tax + accounts both cluster on **30 June**.
- NSS chart is statutory; the skeleton is a simplified NSS-structured convention (documented deviation).