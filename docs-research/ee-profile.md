# Estonia — bukio jurisdiction profile (EE)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **24 %** (raised from 22 % effective 1 Jul 2025) | Avalara; Tax Foundation 2026 (high) |
| Reduced rate | **9 %** — hotel and accommodation services | Avalara; numeral 2026 (high) |
| Zero rate | **0 %** — books, exports, intra-EU B2B | EMTA (high) |
| Small business | VAT registration threshold **€40,000** (annual turnover) | e-resident knowledge base (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `EE` + **9 digits** — `/^EE\d{9}$/` (registrikood + EE prefix; visible on VIES) | companyforbusiness.ee; EMTA (high) |
| Company number | **Registrikood** — 8 digits, e-Äriregister commercial registry code; label 'registrikood' | e-Äriregister (high) |
| Peppol EAS | **9931** — `EE:VAT` "Estonia VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **OÜ** — osaühing (private limited, dominant; €2,500 min capital)
- **AS** — aktsiaselts (public limited)
- **FIE** — füüsilisest isikust ettevõtja (sole trader)
- **TÜ** — tulundusühistu (cooperative)

Confidence: high (standard Äriregistri forms).

## 4. Chart of accounts

Estonia has **no statutory chart**; the standard convention follows the RMP (accounting-policy-driven) layout used by Estonian software: 1 cash/receivables, 2 payables/equity, 3 revenues, 4 expenses — kept in Estonian. Convention skeleton:

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Arvelduskonto (Bank) | 2000 | Osakapital (Share capital) |
| 1100 | Kassa (Cash) | 2100 | Eelmiste perioodide jaotamata kasum (Retained earnings) |
| 1200 | Ostjate laekumata arved (Trade receivables) | 2200 | Aruandeaasta kasum/kahjum (Result for the year) |
| 1300 | Hankijatele tasumata arved (Trade payables) | 3000 | Müügitulu (Sales revenue) |
| 1400 | Võlad töötajatele (Employee payables) | 3100 | Muud äritulud (Other income) |
| 1510 | Sisendkäibemaks (input VAT) | 4000 | Kaubad, toore, materjal ja teenused (Materials & services) |
| 1520 | Väljundkäibemaks (output VAT) | 4100 | Mitmesugused tegevuskulud (Other operating expenses) |
| 1530 | Käibemaks — kohustus (VAT settlement) | 4200 | Tööjõukulud (Personnel costs) |
| 1800 | Põhivara (Fixed assets) | 4300 | Põhivara kulum ja väärtuse langus (Depreciation) |
| 1810 | Akumuleeritud kulum (Accumulated depreciation) | 4400 | Muud ärikulud (Other business expenses) |
| 1900 | Varud (Inventory) | 4500 | Intressikulud (Finance costs) |

Ledger pair 1510 (input, debit) / 1520 (output, credit); settlement 1530 cleared at filing. Closing: 2200 → 2100. Confidence: medium (RMP-convention skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (KMD) | **Monthly** by the **20th** of the following month (e-MTA; quarterly option for smaller taxpayers) | EMTA; e-resident KB (high) |
| Annual accounts | Annual report filed with the e-Äriregister within **6 months** of FYE (30 June for calendar year) | 1office 2026; e-Äriregister (high) |
| Corporate income tax | **CIT on distributions** — 20 %/22 % on distributed profits, payable by the **10th of the month following the distribution** (no classic annual CIT) | PwC CIT table (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9931** (`EE:VAT`); Estonia is an e-invoicing pioneer (e-arve ecosystem, strong Peppol B2B uptake). Confidence: high (EAS codelist).
- **Domestic mandate: B2B e-invoicing expectations mainstreaming (2027 EU framework)** — B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2011). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- Standard rate **24 %** since Jul 2025 — one of the EU's highest.
- **CIT on distributions only** — retained profits are untaxed until distributed; no annual CIT filing (the compliance calendar carries VAT + annual accounts).
- Monthly VAT by the 20th; registrikood vs VAT number differ in length (8 vs 9 digits).