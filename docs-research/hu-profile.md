# Hungary — bukio jurisdiction profile (HU)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **27 %** — the EU's highest | EC europa.eu VAT rates (high) |
| Reduced rate | **18 %** — accommodation, restaurant services, certain food | EC (high) |
| Reduced rate | **5 %** — medicines, books, certain foodstuffs (milk, bread, poultry) | EC (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold HUF 12,000,000 (≈ €30K) | vatcalc HU (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `HU` + **8 digits** — `/^HU\d{8}$/` (adószám with HU prefix) | Avalara EU VAT formats (high) |
| Company number | **Cégjegyzék** number (e.g. 01-09-XXXXXX) — 11 chars, issued by the Court of Registration; the adószám (tax number) is 11 digits (8 + suffix); label 'adoszam' | Hungarian registry practice (high) |
| Peppol EAS | **9910** — `HU:VAT` "Hungary VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **Kft.** — korlátolt felelősségű társaság (limited liability, dominant)
- **Zrt.** / **Nyrt.** — closed / public joint-stock
- **Bt.** — betéti társaság (limited partnership)
- **Egyéni vállalkozó** — sole trader

Confidence: high (standard Ctv. forms).

## 4. Chart of accounts

Hungary has a **statutory chart** — the Számviteli törvény (Act C of 2000) with the standard 1-9 class layout (1 fixed assets, 2 inventory, 3 receivables/cash, 4 liabilities, 5 equity, 8 revenue, 5/6/7 costs). The VAT accounts 466/467 are statutory; the settlement is 468 (or netted via 466/467):

| Code | Account | Code | Account |
|---|---|---|---|
| 3810 | Bank (Bank) | 4110 | Jegyzett tőke (Share capital) |
| 3840 | Pénztár (Cash) | 4130 | Eredménytartalék (Retained earnings) |
| 3110 | Vevők (Trade receivables) | 4190 | Mérleg szerinti eredmény (Result for the year) |
| 4540 | Szállítók (Trade payables) | 9110 | Értékesítés nettó árbevétele (Sales revenue) |
| 4710 | Személyi jellegű kötelezettségek (Employee payables) | 9210 | Egyéb bevételek (Other income) |
| 4660 | Előzetesen felszámított áfa (input VAT) | 5100 | Anyagköltség (Materials) |
| 4670 | Fizetendő áfa (output VAT) | 5200 | Igénybe vett szolgáltatások (Services) |
| 4680 | ÁFA elszámolás (VAT settlement) | 5400 | Bérköltség (Personnel costs) |
| 1200 | Tárgyi eszközök (Fixed assets) | 5600 | Értékcsökkenési leírás (Depreciation) |
| 1390 | Tárgyi eszközök értékcsökkenése (Accumulated depreciation) | 5700 | Egyéb költségek (Other expenses) |
| 2100 | Készletek (Inventory) | 5800 | Pénzügyi ráfordítások (Finance costs) |
| | | 5900 | Társasági adó (Corporate tax) |

Ledger pair 4660 (input, debit) / 4670 (output, credit); settlement 4680. Closing: 4190 → 4130. Confidence: medium (Szt.-structured skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | **Monthly** by the **20th** of the following month (quarterly for small taxpayers by the 20th of the month after the quarter; annual option below HUF 8M) | Marosa HU (medium/high) |
| Annual accounts | Annual report (beszámoló) within **5 months** of FYE (31 May for calendar year) | Hungarian practice (high) |
| Corporate income tax | Annual return (TAO) due **31 May** of the following year | PwC HU (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9910** (`HU:VAT`). Confidence: high (EAS codelist).
- **Domestic mandate: RTIR (real-time invoice reporting 3.0) is mandatory** — every invoice is reported to NAV in real time; a RTIR 3.0 integration is a B-milestone; Peppol BIS registered for cross-border.

## 7. Payment

- **SEPA** member; **HUF** (forint) — own currency, ECB reference rates (baseCurrency 'HUF'). Confidence: high (ECB).

## 8. Gotchas

- **27 % standard** — the EU's highest; 5 % band for basic foods/medicines.
- HUF currency.
- RTIR real-time invoice reporting mandatory (B-milestone).
- Annual accounts + TAO both due **31 May**.