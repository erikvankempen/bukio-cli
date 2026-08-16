# Slovakia — bukio jurisdiction profile (SK)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **23 %** (raised from 20 % effective 1 Jan 2025) | vatcalc SK 2025/26; Tax Foundation 2026 (high) |
| Reduced rate | **19 %** — basic foodstuffs, accommodation (from 1 Jan 2025) | vatcalc SK (high) |
| Reduced rate | **5 %** — certain foods, medicines, books | Tax Foundation 2026 (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **€49,790** (2024+; lowered from €50K) | vatcalc SK (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `SK` + **10 digits** — `/^SK\d{10}$/` | Avalara EU VAT formats (high) |
| Company number | **IČO** (Identifikačné číslo organizácie) — 8 digits, issued by the commercial register; label 'ico' | Register právnických osôb (high) |
| Peppol EAS | **9950** — `SK:VAT` "Slovakia VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **s.r.o.** — spoločnosť s ručením obmedzeným (limited liability, dominant)
- **a.s.** — akciová spoločnosť (joint-stock)
- **živnostník (SZČO)** — sole trader
- **k.s.** — komanditná spoločnosť (limited partnership)

Confidence: high (standard Obchodný zákonník forms).

## 4. Chart of accounts

Slovakia uses the **statutory framework chart** (směrná účtová osnova per the Slovak Ministry of Finance decree) with the standard class layout. The VAT account 343 is split into 3431/3432/3433 for the engine pair (same documented convention as CZ):

| Code | Account | Code | Account |
|---|---|---|---|
| 2210 | Bankové účty (Bank) | 4110 | Základné imanie (Share capital) |
| 2110 | Pokladnica (Cash) | 4210 | Nerozdelený zisk (Retained earnings) |
| 3110 | Odberatelia (Trade receivables) | 4310 | Výsledok hospodárenia bežného obdobia (Result for the year) |
| 3210 | Dodávatelia (Trade payables) | 6010 | Tržby za vlastné výrobky a služby (Sales revenue) |
| 3310 | Zamestnanci (Employee payables) | 6110 | Ostatné výnosy (Other income) |
| 3431 | DPH na vstupe (input VAT) | 5010 | Spotreba materiálu (Materials) |
| 3432 | DPH na výstupe (output VAT) | 5180 | Služby (Services) |
| 3433 | DPH — zúčtovanie (VAT settlement) | 5210 | Mzdové náklady (Personnel costs) |
| 0220 | Dlhodobý hmotný majetok (Fixed assets) | 5510 | Odpisy (Depreciation) |
| 0820 | Oprávky k DHM (Accumulated depreciation) | 5680 | Finančné náklady (Finance costs) |
| 1320 | Tovar na sklade (Inventory) | 5910 | Daň z príjmov (Corporate tax) |

Ledger pair 3431 (input, debit) / 3432 (output, credit); settlement 3433. Closing: 4310 → 4210. Confidence: medium (framework-chart skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | **Monthly** by the **25th** of the following month (quarterly for small taxpayers — 25th of the month after the quarter) | Marosa SK due-date table (medium/high) |
| Annual accounts | Annual report within 6 months of FYE (filed with the register; ~30 June for calendar year) | Slovak practice (medium) |
| Corporate income tax | Annual return due **31 March** of the following year (6-month extension to 30 June with fee) | PwC SK (medium/high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9950** (`SK:VAT`); Slovakia became a **Peppol Authority in March 2026**, with a B2B e-invoicing mandate slated for **2027** (EN 16931 UBL 2.1/CII). Confidence: high (EAS codelist + Peppol news).
- **Domestic mandate: 2027 B2B mandate in preparation** — B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2009). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- Standard rate **23 %** since 2025 — one of the EU's highest.
- New Peppol Authority (Mar 2026) — cross-border Peppol BIS fully aligned; the 2027 domestic mandate is a B-milestone.
- VAT account 343 split (3431/3432/3433) — documented convention.