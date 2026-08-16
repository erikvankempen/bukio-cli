# Malta — bukio jurisdiction profile (MT)

Phase D profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **18 %** — default on any taxable supply unless a reduced rate/exemption applies | MTCA VAT Rates (high) |
| Reduced rate | **12 %** — custody & management of securities; management of credit/credit guarantees; hiring of pleasure boats (≤5 weeks cumulative per 12 months); body-care services under the Health Care Professions Act (Cap. 464) incl. health studios | MTCA VAT Rates & Exemptions FAQ 12/06/2025 (high) |
| Reduced rate | **7 %** — licensed tourist accommodation (Malta Travel & Tourism Services Act premises, holiday camps, camping sites); use of sporting facilities | MTCA FAQ (high) |
| Reduced rate | **5 %** — electricity; confectionery & similar items; plus (per secondary source) books/periodicals, medical devices, cultural admission | MTCA FAQ (high); Numeral 2026 (medium) |
| Zero rate | **0 %** — exports, intra-EU B2B, certain essentials (Fifth Schedule, exemption with credit) | MTCA FAQ (high) |
| Small business | **Small undertaking exemption** (Art. 11 VAT Act): annual domestic turnover ≤ **€35,000** (uniform for goods & services, from 1 Jan 2025 via Act XXXVIII of 2024 + LNs 345–353/2024); exempt traders don't charge VAT and cannot reclaim input VAT; annual declaration due **15 February** (traders < €7,000 may opt out). Art. 11A: cross-border exemption for Union turnover ≤ €100,000 | PwC MT; Quazar; ncmb (high for €35k; medium for details) |

Note: the "18/7/5/0" shorthand is incomplete — MTCA officially lists **12 %, 7 %, 5 %, 0 %** as the reduced rates.

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `MT` + **8 digits** — `/^MT\d{8}$/` (e.g. MT12345678) | Avalara EU VAT formats; EUIPO list (high) |
| Company number | Issued by **Malta Business Registry (MBR)**; label "Company Registration Number". Classic format `C` + digits (e.g. `C 12345`); newer registrations carry longer numeric strings — **verify live format against the MBR company search before coding validation** | mbr.mt (issuer); format low/medium |
| Peppol EAS | **9943** — `MT:VAT` "Malta VAT number" | Official OpenPEPPOL code list, docs.peppol.eu codelists v8.5 (high) |

## 3. Legal forms

- **Ltd** — private limited liability company (dominant form)
- **p.l.c.** — public limited company
- **Partnerships** — general ("en nom collectif"), limited ("en commandite"), including the limited partnership
- **Sole trader / self-employed** — VAT-registered under Art. 10 or exempt under Art. 11

Confidence: medium (standard Companies Act forms; verify per-entity suffixes at MBR).

## 4. Chart of accounts

Malta has **no statutory chart of accounts**; companies apply IFRS (as adopted by the EU) and local accountants use a convention-based international chart, kept in **English**. Recommended bukio skeleton (4-digit codes, unique, based on the common international convention chart used locally):

| Code | Account | Code | Account |
|---|---|---|---|
| 1000 | Bank — current account | 3000 | Share capital |
| 1100 | Trade debtors | 3100 | Share premium |
| 1200 | Other receivables | 3200 | Retained earnings |
| 1300 | Prepayments | 3300 | Profit/(loss) for the year (closing result) |
| 1400 | Inventory | 4000 | Sales revenue |
| 1500 | Fixed assets | 4100 | Other income |
| 1510 | Accumulated depreciation | 4200 | Purchases / cost of sales |
| 2000 | Trade creditors | 5000 | Wages and salaries |
| 2100 | Other payables | 5100 | Rent and rates |
| 2200 | Accruals | 5200 | Utilities |
| 2300 | Employee taxes payable (PAYE/SS) | 5300 | Professional fees |
| 2410 | VAT input (on purchases) | 5400 | Repairs and maintenance |
| 2420 | VAT output (on sales) | 5500 | Depreciation |
| 2430 | VAT settlement (net payable/recoverable) | 5600 | Other operating expenses |
| 2500 | Loans (long-term) | 5700 | Finance costs |
| | | 5800 | Tax expense (CIT) |

VAT ledger pair 2410/2420 can be sub-divided per rate (12/7/5/0); 2430 is cleared when the quarterly return is filed/paid. Confidence: medium (no statutory chart; skeleton is convention-based).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | Quarterly (calendar quarters), e-filed via myTax portal. Due by the **15th of the second month after quarter end (≈45 days)**: Q1 → 15 May, Q2 → 15 Aug, Q3 → 15 Nov, Q4 → 15 Feb; **+7 days (22nd) when filed online** | Marosa due-dates table; Numeral 2026 (medium/high); MTCA cycle page states "15th of the month following period end, +7 days online" — verify exact day against myTax |
| Annual VAT declaration | Small undertakings (Art. 11): annual declaration due **15 February** | ncmb (medium) |
| EC Sales List | 15th of the following month | Marosa (medium) |
| Annual accounts | File with MBR within **10 months after FYE** (e.g. 31 October for a 31 Dec year-end) | PwC MT; ncmb; CSB (high) |
| Corporate income tax | Form C + self-assessment due **9 months after FYE** (e.g. 30 September for 31 Dec); concessionary e-filing extensions granted in recent years (FYE 31 Dec 2024 → 28 Nov 2025). Provisional tax: 3 instalments (20 % / 30 % / 50 %) every 4 months | MTCA; PwC (high); VitalLaw (medium) |
| Fiscal year end | **31 December** is the common default; any date may be adopted | Practice (medium/high) |

### 5b. VAT return shape
Quarterly is the standard cycle; monthly cycles also exist for some traders (secondary source). Due 15th/22nd of the month following the period end per MTCA wording; for quarterly periods commercial sources cite the 15th/22nd of the *second* month after the quarter. Confidence: medium — verify against the myTax portal.

## 6. E-invoicing

- **Peppol participant: yes** — scheme ID **9943** (`MT:VAT`); Malta leverages Peppol for public-sector e-invoicing as part of digital government reforms; B2B enablement being expanded (Qvalia 2026; docs.peppol.eu).
- **Domestic mandate: none** for B2B (note only). Government-approved Peppol providers exist (e.g. Pagero). Confidence: medium.

## 7. Payment

- **SEPA** member; **EUR** currency (eurozone since 2008). IBAN-based payments. Confidence: high (common knowledge; ECB).

## 8. Gotchas

- No statutory chart of accounts — IFRS + convention chart, English-language books.
- **Four reduced rates incl. 12 %** (securities custody, credit management, pleasure-boat hire, body-care) — do not assume 18/7/5/0.
- Small-undertaking exemption is **€35,000 uniform** since 1 Jan 2025 (previously €37k goods / €24k services; one 2026 source still cites €30k for services — treat €35k as current per PwC/Quazar/ncmb).
- Tourist accommodation is at the **7 %** rate (licensed tourist premises).
- Autumn deadline cluster for a 31 Dec FYE: CIT 30 Sep, accounts 31 Oct, Q3 VAT mid-Nov.
- MBR company-number format in flux (classic `C` prefix vs newer numeric strings) — validate leniently.
- Annual CIT e-filing extensions are routine — check MTCA announcements each year.
