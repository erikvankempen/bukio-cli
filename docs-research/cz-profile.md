# Czechia — bukio jurisdiction profile (CZ)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **21 %** | EC europa.eu VAT rates (high) |
| Reduced rate | **12 %** (single reduced rate since 1 Jan 2024; previously 15/10) | hellotax; EC (high) |
| Zero rate | **0 %** — exports, intra-EU B2B, books | standard EU scheme (high) |
| Small business | VAT registration threshold CZK 2,000,000 (≈ €80K) | globalvatcompliance CZ (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `CZ` + **8, 9 or 10 digits** — `/^CZ\d{8,10}$/` (8 for legal entities, 9-10 for entrepreneurs) | globalvatcompliance; hellotax; Avalara (high) |
| Company number | **IČO** (Identifikační číslo osoby) — 8 digits, issued by the commercial registry (ARES); label 'ico' | ARES (high) |
| Peppol EAS | **9929** — `CZ:VAT` "Czech Republic VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **s.r.o.** — společnost s ručením omezeným (limited liability, dominant)
- **a.s.** — akciová společnost (joint-stock)
- **OSVČ / živnost** — sole trader (trade licence)
- **k.s.** — komanditní společnost (limited partnership)

Confidence: high (standard Obchodní zákoník forms).

## 4. Chart of accounts

Czechia has a **statutory framework chart** — the směrná účtová osnova (classes 0-8, MF Decree 500/2002). The bukio skeleton follows the standard layout; the VAT account 343 is split into 3431/3432/3433 for the input/output/settlement pair (engine contract — many practices use subaccounts of 343):

| Code | Account | Code | Account |
|---|---|---|---|
| 2210 | Bankovní účty (Bank) | 4110 | Základní kapitál (Share capital) |
| 2110 | Pokladna (Cash) | 4210 | Nerozdělený zisk (Retained earnings) |
| 3110 | Odběratelé (Trade receivables) | 4310 | Výsledek hospodaření běžného období (Result for the year) |
| 3210 | Dodavatelé (Trade payables) | 6010 | Tržby za vlastní výrobky a služby (Sales revenue) |
| 3310 | Zaměstnanci (Employee payables) | 6110 | Ostatní výnosy (Other income) |
| 3431 | DPH na vstupu (input VAT) | 5010 | Spotřeba materiálu (Materials) |
| 3432 | DPH na výstupu (output VAT) | 5180 | Služby (Services) |
| 3433 | DPH — zúčtování (VAT settlement) | 5210 | Mzdové náklady (Personnel costs) |
| 0220 | Dlouhodobý hmotný majetek (Fixed assets) | 5510 | Odpisy (Depreciation) |
| 0820 | Oprávky k DHM (Accumulated depreciation) | 5680 | Finanční náklady (Finance costs) |
| 1320 | Zboží na skladě (Inventory) | 5910 | Daň z příjmů (Corporate tax) |

Ledger pair 3431 (input, debit) / 3432 (output, credit); settlement 3433. Closing: 4310 → 4210. Confidence: medium (framework-chart skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | **Monthly or quarterly** by the **25th** of the month following the period | Avalara; Marosa CZ (high) |
| Annual accounts | Financial statements approved at the AGM within 6 months of FYE; filed with the registry within 30 days of approval (≈ 30 June + 30 days) | Czech company-law practice (medium/high) |
| Corporate income tax | Annual return due within 3 months of FYE (31 March); 6 months (1 July) for audited companies | PwC CZ (medium/high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9929** (`CZ:VAT`). Confidence: high (EAS codelist).
- **Domestic mandate: none yet** — EU 2027/903 framework in preparation. B-milestone note only.

## 7. Payment

- **SEPA** member; **CZK** (koruna) — own currency, ECB reference rates (baseCurrency 'CZK'). Confidence: high (ECB).

## 8. Gotchas

- CZK currency — amounts in koruna; the FX engine's per-profile baseCurrency applies (same treatment as DK/SE/NO/GB/US).
- Single 12 % reduced rate since 2024.
- VAT account 343 split into 3431/3432/3433 for the engine pair (documented convention).