# Cyprus — bukio jurisdiction profile (CY)

Phase D profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **19 %** — default rate (one of the EU's highest) | PwC CY (high) |
| Reduced rate | **9 %** — accommodation; restaurant & catering services; certain local passenger transport; supplies by old people's homes; certain electricity supplies (if not exempt) | PwC CY (high) |
| Reduced rate | **5 %** — foodstuffs and pharmaceutical products and other listed goods/services; also first 130 m² of a primary residence (value ≤ €350,000, total ≤ €475,000 / 190 m², conditions) | PwC CY (high) |
| Super-reduced rate | **3 %** — since 21 July 2023: entry to first performances of classical theatrical/musical/dance works; waste collection/treatment/cleaning; sewage disposal & tank emptying; cultural goods (books, newspapers, magazines — print & electronic); goods for citizens with special needs | PwC CY (high) |
| Zero rate | **0 %** — exports, qualifying aircraft/vessels; temporary zero-rating of basic goods (bread, milk, eggs, children's food, diapers, feminine-hygiene products; earlier also coffee, sugar, meat, vegetables) renewed to **31 Dec 2026** (R.A.A. 337/2025) | PwC CY (high) |
| Small business | **Small undertakings scheme**: exemption from VAT registration/charging where annual taxable turnover does not exceed **€15,600** in a 12-month period; new rules (2025) extend exemption to non-established EU SMEs with ≤ €15,600 Cyprus turnover | businessincyprus.gov.cy; Avalara; philenews (high for €15,600) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `CY` + **8 digits + 1 letter** — `/^CY\d{8}[A-Z]$/` (e.g. CY12345678X; last character is always a letter) | Avalara EU VAT formats (high) |
| Company number | **HE number** — `HE` + 6 digits (e.g. `HE 123456`), label "HE number", issued by the **Registrar of Companies (DRCOR)**; `EE` prefix used for branches of foreign companies; older companies may carry legacy `C` numbers. Verify live format via DRCOR e-filing | cyprusbusinessdir (label, high); format medium |
| Peppol EAS | **9928** — `CY:VAT` "Cyprus VAT number" | Official OpenPEPPOL code list, docs.peppol.eu codelists v8.5 (high; cross-checked with peppolgate.eu) |

## 3. Legal forms

- **Ltd** — private limited company (dominant form)
- **p.l.c.** — public limited company
- **Sole trader** (self-employed, registered with the Tax Department)
- **Partnership** (general / limited)
- **Branch of a foreign company** (EE number)

Confidence: medium (standard Companies Law forms; verify suffixes at DRCOR).

## 4. Chart of accounts

Cyprus has **no statutory chart of accounts**; companies apply IFRS and local accountants use a convention-based international chart, kept in **English**. Recommended bukio skeleton (4-digit codes, unique, same convention chart as MT/NL-family profiles):

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
| 2300 | Employee taxes payable (PAYE/GHS) | 5300 | Professional fees |
| 2410 | VAT input (on purchases) | 5400 | Repairs and maintenance |
| 2420 | VAT output (on sales) | 5500 | Depreciation |
| 2430 | VAT settlement (net payable/recoverable) | 5600 | Other operating expenses |
| 2500 | Loans (long-term) | 5700 | Finance costs |
| | | 5800 | Tax expense (CIT) |

VAT ledger pair 2410/2420 can be sub-divided per rate (19/9/5/3/0) — the VAT 4 return reports output VAT by rate band; 2430 is cleared when the return is filed/paid. Confidence: medium (no statutory chart; skeleton is convention-based).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (VAT 4) | Quarterly standard — due the **10th of the second month after the quarter**: Q1 → **10 May**, Q2 → **10 Aug**, Q3 → **10 Nov**, Q4 → **10 Feb**. Filed on the **Tax For All (TFA) portal** (taxforall.mof.gov.cy; VAT filing moved off TAXISnet in Mar 2023). Payment due same day. Late filing: flat **€100** per return (even nil); late payment 10 % + 1.75 % interest | Avalara; CyprusDesk; Marosa (high) |
| Monthly VAT | Mandatory for businesses with annual turnover **> €1 million**; quarterly otherwise (Tax Department assigns frequency at registration) | FZCO Accountants (medium); Avalara (high on "assigned frequency") |
| EC Sales List | 15th of the following month; Intrastat 10th of the following month | Marosa (medium) |
| Annual VAT return | **None** — quarterly (or monthly) returns only; VAT refunds applied via TD2008 | CyprusDesk (medium/high) |
| Annual accounts | File with the Registrar within **42 days of the AGM**; AGM within 18 months of incorporation, then ≤ 15 months after the last AGM — in practice accounts commonly land ~10–13 months after FYE | companies.gov.cy; savvacyprus; nexoracyprus (medium/high) |
| Corporate income tax (TD4) | Calendar tax year. Transitional 2026 dates: TY2023 → **31 Mar 2026**, TY2024 → **30 Nov 2026** (extensions per Decrees KΔΠ 358/2025 & 359/2025). **Permanent rule from TY2026: due 31 January of the second year following the tax year** (TY2026 → 31 Jan 2028). Provisional tax: 1st instalment **31 July** (with provisional return), 2nd/revised **31 December**; balance via self-assessment **1 August** | cmarkou tax calendar 2026; PwC CY (high) |
| Fiscal year end | Tax year = calendar year; companies commonly use **31 December** | PwC CY; practice (high) |

### 5b. VAT return shape
Standard shape is **quarterly (VAT 4, 9 boxes)**; **monthly** filing is required for traders above €1M annual turnover. Both shapes verified (Marosa: "10th day of the second following month"; FZCO: monthly > €1M). Note the "10th of the month after the quarter" shorthand is wrong — it is the 10th of the *second* month after the quarter.

## 6. E-invoicing

- **Peppol participant: yes** — scheme ID **9928** (`CY:VAT`); OpenPeppol member with public entities using Peppol for B2G e-invoicing (Qvalia 2026; docs.peppol.eu).
- **Domestic mandate: none** for B2G/B2B/B2C (EC 2025 eInvoicing country sheet); discussions on mandatory adoption ongoing. Confidence: high.

## 7. Payment

- **SEPA** member; **EUR** currency (eurozone since 2008). IBAN-based payments. Confidence: high (common knowledge; ECB).

## 8. Gotchas

- **19 % standard rate** — one of the EU's highest; plus 9 / 5 / 3 / 0 (3 % super-reduced only since July 2023 — don't assume 19/9/5/0 alone).
- No statutory chart of accounts; English-language books (IFRS).
- 5 % reduced VAT on the first 130 m² of a primary residence (conditions; declaration deadline 12 months from possession) — property supplies need care.
- Temporary zero-rating of basic foodstuffs is renewed periodically (now to 31 Dec 2026) — track extensions.
- Flat **€100 late-filing penalty per VAT return**, even for nil returns.
- VAT is filed on the **TFA portal**, not TAXISnet (since Mar 2023).
- Monthly VAT above €1M turnover; quarterly otherwise.
- HE numbers issued by DRCOR; annual return 42 days after AGM with late fees.
- SDC/GHS levies on dividends/interest for Cyprus tax-resident individuals (incl. deemed dividend-distribution rules) — dividend bookkeeping may need SDC/GHS tracking.
