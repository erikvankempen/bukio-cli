# Latvia — bukio jurisdiction profile (LV)

Phase E profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **21 %** | globalvatcompliance LV; Tax Foundation 2026 (high) |
| Reduced rate | **12 %** — accommodation, certain foods, books, utilities (broad band) | globalvatcompliance LV (high) |
| Reduced rate | **5 %** — certain foods, medicines, medical devices | Tax Foundation 2026 (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **€50,000** (annual taxable supplies) | vatcalc LV 2025 (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `LV` + **11 digits** — `/^LV\d{11}$/` (the unified registration number with LV prefix) | Lappa LV guide (high) |
| Company number | **Reģistrācijas numurs** (unified registration number) — 11 digits, issued by the Latvian Commercial Register (Uzņēmumu reģistrs); label 'reg-nr' | Lappa LV (high) |
| Peppol EAS | **9939** — `LV:VAT` "Latvia VAT number" (0218 'unified registration number (Latvia)' also exists) | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **SIA** — sabiedrība ar ierobežotu atbildību (limited liability, dominant)
- **AS** — akciju sabiedrība (joint-stock)
- **IK** — individuālais komersants (sole trader)
- **SIA mazā** — small SIA (reduced capital)

Confidence: high (standard Komerclikums forms).

## 4. Chart of accounts

Latvia has a **standardised kontu plāns** (chart per the LR Finance Ministry's guidance, used by Latvian software). The skeleton follows the standard class layout (1 current assets, 2 liabilities & equity, 3 revenues, 4 expenses) in Latvian:

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Norēķinu konti bankā (Bank) | 2000 | Pamatkapitāls (Share capital) |
| 1100 | Kase (Cash) | 2100 | Nesadalītā peļņa (Retained earnings) |
| 1200 | Pircēju parādi (Trade receivables) | 2200 | Pārskata gada peļņa/zaudējumi (Result for the year) |
| 1300 | Piegādātāju parādi (Trade payables) | 3000 | Ieņēmumi no preču pārdošanas (Sales revenue) |
| 1400 | Darba samaksas parādi (Employee payables) | 3100 | Citi ieņēmumi (Other income) |
| 1510 | Priekšnodoklis (input VAT) | 4000 | Iepirkto preču un materiālu izmaksas (Materials) |
| 1520 | PVN budžetā (output VAT) | 4100 | Darbības izmaksas (Operating expenses) |
| 1530 | PVN — norēķini (VAT settlement) | 4200 | Darbaspēka izmaksas (Personnel costs) |
| 1800 | Pamatlīdzekļi (Fixed assets) | 4300 | Pamatlīdzekļu nolietojums (Depreciation) |
| 1810 | Uzkrātais nolietojums (Accumulated depreciation) | 4400 | Citi saimnieciskās darbības izdevumi (Other expenses) |
| 1900 | Krājumi (Inventory) | 4500 | Finanšu izdevumi (Finance costs) |

Ledger pair 1510 (input, debit) / 1520 (output, credit); settlement 1530 cleared at filing. Closing: 2200 → 2100. Confidence: medium (standard-plan skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (PVN) | **Monthly**, due the **20th** of the following month via EDS (no quarterly option for established businesses; EC Sales List same schedule) | Lappa LV; Avalara (high) |
| Annual accounts | Annual report (gada pārskats) approved within 6 months of FYE and filed with VID within 1 month of approval (≈ 31 July for calendar year; most companies file Apr–Jun) | Latvian company-law practice (medium) |
| Corporate income tax | **CIT on distributions** — 20 % on distributed profits; declaration accompanies each distribution/quarter; no classic annual CIT return | PwC CIT table; LR VID (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9939** (`LV:VAT`; 0218 unified-registration alternative). Confidence: high (EAS codelist).
- **Domestic mandate: B2B e-invoicing requirements introduced (2025 timeframe); 2027 EU framework** — B-milestone note only.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2014). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- **Monthly VAT only** — no quarterly option; PVN + EC Sales List both due the 20th.
- **CIT on distributions** — retained profit untaxed until distributed (like EE); calendar carries VAT + annual accounts.
- Annual report filed via VID EDS within 1 month of approval; approval ≤ 6 months after FYE.