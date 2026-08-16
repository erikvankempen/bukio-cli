# Lithuania — bukio jurisdiction profile (LT)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **21 %** | northtradehub 2026; vatcompliance (high) |
| Reduced rate | **9 %** — accommodation, passenger transport, certain services | vatcompliance LT (high) |
| Reduced rate | **5 %** — certain printed materials, pharmaceuticals | vatcompliance LT (high) |
| Zero rate | **0 %** — exports, intra-EU B2B, certain transport | vatcompliance LT (high) |
| Small business | VAT registration threshold **€45,000** (annual turnover; non-residents register from the first supply) | northtradehub 2026 (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `LT` + **9 or 12 digits** — `/^LT\d{9,12}$/` (9-digit for legal entities, 12-digit for VAT-group/history cases) | northtradehub; vatcompliance (high) |
| Company number | **Įmonės kodas** (legal entity code) — 9 digits, issued by VĮ Registrų centras; label 'imoniu-kodas' | Registrų centras (high) |
| Peppol EAS | **9937** — `LT:VAT` "Lithuania VAT number" (0200 'legal entity code (Lithuania)' also exists) | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **UAB** — uždaroji akcinė bendrovė (private limited, dominant)
- **AB** — akcinė bendrovė (public limited)
- **IĮ** — individuali įmonė (individual enterprise)
- **MB** — mažoji bendrija (small partnership)

Confidence: high (standard Civilinio kodekso forms).

## 4. Chart of accounts

Lithuania has a **statutory-standard chart** — the Įmonių sąskaitų planas approved by Ministry of Finance order (2002, updated); the skeleton follows its class layout (1 current assets, 2 liabilities & equity, 3 income, 4 expenses) in Lithuanian:

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Pinigai banko sąskaitoje (Bank) | 2000 | Įstatinis kapitalas (Share capital) |
| 1100 | Kasa (Cash) | 2100 | Nepaskirstytasis pelnas (Retained earnings) |
| 1200 | Pirkėjų skolos (Trade receivables) | 2200 | Ataskaitinių metų pelnas (nuostoliai) (Result for the year) |
| 1300 | Skolos tiekėjams (Trade payables) | 3000 | Pardavimo pajamos (Sales revenue) |
| 1400 | Skolos darbuotojams (Employee payables) | 3100 | Kitos pajamos (Other income) |
| 1500 | Pirkimo PVM (input VAT) | 4000 | Parduotų prekių savikaina (Cost of sales) |
| 1510 | Pardavimo PVM (output VAT) | 4100 | Veiklos sąnaudos (Operating expenses) |
| 1520 | PVM — atsiskaitymai su biudžetu (VAT settlement) | 4200 | Darbo užmokesčio sąnaudos (Personnel costs) |
| 1800 | Ilgalaikis materialusis turtas (Fixed assets) | 4300 | Ilgalaikio turto nusidėvėjimas (Depreciation) |
| 1810 | Sukauptas nusidėvėjimas (Accumulated depreciation) | 4400 | Kitos veiklos sąnaudos (Other expenses) |
| 1900 | Atsargos (Inventory) | 4500 | Finansinės sąnaudos (Finance costs) |

Ledger pair 1500 (input, debit) / 1510 (output, credit); settlement 1520 cleared at filing. Closing: 2200 → 2100. Confidence: medium (MF-plan skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (PVM) | **Monthly** by the **25th** of the following month (i.SAF; payment same day); quarterly option for smaller taxpayers | vatcompliance LT; northtradehub (high) |
| Annual accounts | Annual financial statements approved within 4 months of FYE and filed with VĮ Registrų centras shortly after (~30 April for calendar year) | Registrų centras practice (medium/high) |
| Corporate income tax | Annual CIT return (PNM) due **1 October** of the following year (advances during the year) | PwC LT CIT table (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9937** (`LT:VAT`); Lithuania also runs the domestic i.SAF/e-Sąskaita system. Confidence: high (EAS codelist).
- **Domestic mandate: e-Sąskaita B2B framework (2025 mandate for e-invoices between registered businesses)** — B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2015). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- Monthly PVM by the 25th; i.SAF reporting is part of the filing chain (B-milestone).
- VAT number accepts 9 or 12 digits.
- CIT annual return due **1 October** — later than most peers; annual accounts ~30 April.