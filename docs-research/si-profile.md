# Slovenia — bukio jurisdiction profile (SI)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **22 %** (raised from 20 % effective 1 Jan 2025) | vatcalc SI; Tax Foundation 2026 (high) |
| Reduced rate | **9.5 %** — foodstuffs, accommodation, books, certain services | vatcalc SI (high) |
| Reduced rate | **5 %** — certain foods, medicines, newspapers | Tax Foundation 2026 (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **€60,000** (nil for non-residents; €10,000 for pan-EU digital OSS) | vatcalc SI (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `SI` + **8 digits** — `/^SI\d{8}$/` (the davčna številka with SI prefix) | fonoa SI; vatit (high) |
| Company number | **DŠ / davčna številka** (tax number) — 8 digits, issued by FURS; also the matična številka (registration number) 7 digits from AJPES; label 'davcna' | FURS (high) |
| Peppol EAS | **9949** — `SI:VAT` "Slovenia VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **d.o.o.** — družba z omejeno odgovornostjo (limited liability, dominant)
- **d.d.** — delniška družba (joint-stock)
- **s.p.** — samostojni podjetnik (sole trader)
- **k.d.** — komanditna družba (limited partnership)

Confidence: high (standard Zakon o gospodarskih družbah forms).

## 4. Chart of accounts

Slovenia has a **standardised kontni načrt** (chart defined in the Slovenian accounting standard SRS 30, 2024 edition) used with minor software variants. The bukio skeleton follows the standard layout (1 current assets/liquidity, 2 liabilities & equity, 3 revenues/expenses classes):

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Poslovni račun (Bank) | 2000 | Osnovni kapital (Share capital) |
| 1100 | Blagajna (Cash) | 2100 | Preneseni dobiček (Retained earnings) |
| 1200 | Kratkoročne terjatve do kupcev (Trade receivables) | 2200 | Čisti dobiček iz poslovnega izida (Result for the year) |
| 1300 | Kratkoročne obveznosti do dobaviteljev (Trade payables) | 3000 | Prihodki od prodaje (Sales revenue) |
| 1400 | Obveznosti do zaposlencev (Employee payables) | 3100 | Drugi prihodki (Other income) |
| 1500 | Vstopni DDV (input VAT) | 4000 | Stroški blaga in materiala (Materials) |
| 1510 | Izstopni DDV (output VAT) | 4100 | Stroški storitev (Services) |
| 1520 | DDV — obračun (VAT settlement) | 4200 | Stroški dela (Personnel costs) |
| 1800 | Opredmetena osnovna sredstva (Fixed assets) | 4300 | Amortizacija (Depreciation) |
| 1810 | Popravek vrednosti OOS (Accumulated depreciation) | 4400 | Finančni odhodki (Finance costs) |
| 1900 | Zaloge (Inventory) | 4500 | Davek od dohodkov pravnih oseb (Corporate tax) |

Ledger pair 1500 (input, debit) / 1510 (output, credit); settlement 1520 cleared at filing. Closing: 2200 → 2100. Confidence: medium (SRS-30-structured skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (DDV-O) | **Monthly** by the **20th** of the following month (eDavki); quarterly option for small businesses | Marosa due-date table (medium/high) |
| Annual VAT return | Annual DDV-O for quarterly filers due **31 March** of the following year | Marosa SI (medium) |
| Annual accounts | Annual report filed with **AJPES** within **8 months** of FYE (31 August for calendar year; sole traders 3 months) | AJPES (high) |
| Corporate income tax (DDPO) | Annual return due **31 March** of the following year (with payment; prepayments quarterly) | PwC SI / Marosa (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9949** (`SI:VAT`). Confidence: high (EAS codelist).
- **Domestic mandate: none yet** — EU 2027/903 framework in preparation. B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2007). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- Standard rate raised to **22 %** on 1 Jan 2025.
- DDV-O monthly by the 20th; quarterly filers file an annual DDV-O by 31 March.
- Annual accounts via AJPES within **8 months** (31 August) — later than most peers.
- DDPO by 31 March; davčna številka doubles as the VAT-number base.