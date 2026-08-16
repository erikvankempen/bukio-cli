# Croatia — bukio jurisdiction profile (HR)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **25 %** | EC europa.eu VAT rates (high) |
| Reduced rate | **13 %** — accommodation, newspapers, certain foods, utilities (broad reduced band) | PwC HR / Avalara (high) |
| Reduced rate | **5 %** — bread, milk, some foodstuffs, books, certain services | Avalara HR (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **~€40,000** (HRK 300,000 at the 2023 euro conversion 7.53450) | Marosa HR (medium/high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `HR` + **11 digits** — `/^HR\d{11}$/` (HR prefix + the OIB with ISO 7064 Mod 11,10 check digit) | Avalara; vatupdate 2026 (high) |
| Company number | **OIB** (Osobni identifikacijski broj) — 11 digits, issued by the Ministry of Finance; label 'oib' | LookupTax OIB guide (high) |
| Peppol EAS | **9934** — `HR:VAT` "Croatia VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **d.o.o.** — društvo s ograničenom odgovornošću (limited liability, dominant)
- **j.d.o.o.** — jednostavno d.o.o. (simple LLC, €1 capital)
- **d.d.** — dioničko društvo (joint-stock)
- **obrt** — sole trader (registered craft)

Confidence: high (standard Trgovački zakon forms).

## 4. Chart of accounts

No single statutory chart; the standard **Računski plan** used by Croatian accountants follows the 4-class structure (0 fixed/current assets, 1/2 liabilities & equity, 3 expenses, 4 revenues — with 7xxx/4xxx variants per software). Convention skeleton:

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Banka — žiro račun (Bank) | 2000 | Temeljni kapital (Share capital) |
| 1100 | Blagajna (Cash) | 2100 | Zadržana dobit (Retained earnings) |
| 1200 | Kupci (Trade receivables) | 2200 | Dobit/gubitak tekuće godine (Result for the year) |
| 1300 | Dobavljači (Trade payables) | 3000 | Prihodi od prodaje (Sales revenue) |
| 1400 | Obveze za plaće (Employee payables) | 3100 | Ostali prihodi (Other income) |
| 1500 | Potraživanja za PDV (input VAT) | 4000 | Troškovi sirovina i materijala (Materials) |
| 1510 | Obveze za PDV (output VAT) | 4100 | Troškovi usluga (Services) |
| 1520 | PDV — obračun s državom (VAT settlement) | 4200 | Troškovi zaposlenih (Personnel costs) |
| 1800 | Dugotrajna imovina (Fixed assets) | 4300 | Amortizacija (Depreciation) |
| 1810 | Ispravak vrijednosti DIM (Accumulated depreciation) | 4400 | Financijski rashodi (Finance costs) |
| 1900 | Zalihe (Inventory) | 4500 | Porez na dobit (Corporate tax) |

Ledger pair 1500 (input, debit) / 1510 (output, credit); settlement 1520 cleared at filing. Closing: 2200 → 2100. Confidence: medium (convention skeleton in the standard Croatian class layout).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (PDV) | **Monthly** by the **20th** of the following month (ePorezna); quarterly option for small taxpayers | Marosa due-date table (medium/high) |
| Annual accounts | Financial statements (RGFI) filed with FINA by **30 April** of the following year | RGFI/FINA practice (high) |
| Corporate income tax | Annual PD return by **30 April** of the following year (quarterly prepayments) | Marosa HR (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9934** (`HR:VAT`); Croatia participates in Peppol B2G. Confidence: high (EAS codelist).
- **Domestic mandate: none yet** — EU 2027/903 framework in preparation. B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** since **1 Jan 2023** (eurozone). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- 25 % standard — one of the EU's highest; 13 % is the broad reduced band.
- OIB check digit uses ISO 7064 Mod 11,10 — the format regex alone does not validate.
- Annual accounts + CIT both due **30 April**.
- Accounts are commonly kept in the standard Računski plan layout; no statutory chart (documented convention).