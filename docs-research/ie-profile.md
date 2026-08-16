# Ireland — bukio jurisdiction profile (IE)

Phase C profile. Research verified 15 August 2026. Every item source-verified;
confidence per item.

## 1. Tax system

VAT (Value-Added Tax) — EU member, SEPA, EUR.

| Item | Value | Source |
|---|---|---|
| Standard rate | **23 %** (since 1 Jan 2020) | Revenue current-VAT-rates table |
| Reduced rate | **13.5 %** | Revenue (same table) |
| Second reduced rate | **9 %** (tourism & hospitality, reintroduced from 1 Nov 2024) | Revenue (same table); Taxology 2026 |
| Livestock rate | **4.8 %** | Revenue (same table) |
| Zero rate | **0 %** (food, books, exports) | Taxology 2026 |
| Flat-rate farmer compensation | 4.5 % (2026) | Revenue (same table) |
| Registration thresholds | **€85,000 goods / €42,500 services** (since 1 Jan 2025, Finance Act 2024; previously €75,000/€37,500) | Revenue VAT-thresholds page; karr.pro; ybcase |
| Reverse charge | VATCA 2010 s. 108 (intra-Community B2B) | Revenue |

Confidence: high (Revenue primary source; thresholds cross-checked).

## 2. VAT returns (VAT3)

| Item | Value | Source |
|---|---|---|
| Return | **VAT3** — six bi-monthly periods per year (Jan/Feb … Nov/Dec) | Revenue; Marosa VAT manual |
| Deadline | **23rd of the month after the period end** (P1 Jan/Feb → 23 Mar … P6 Nov/Dec → 23 Jan next year) | Taxenlight 2026 (Sep: file 19 / pay 23) |
| Annual | Corporation tax Form **CT1** due 9 months after the accounting period end (calendar year → 30 Sep) | Revenue; outbooks 2026 calendar |
| Annual accounts | Filed with the CRO within **9 months of the FYE** (private limited companies, Companies Act 2014) | Companies Act 2014; outbooks 2026 |

Confidence: high (Revenue + 2026 compliance calendars).

## 3. Identifiers

| Item | Value | Source |
|---|---|---|
| VAT number | `IE` + **7 digits + 1-2 letters** (IE1234567T, IE1234567TW) | Revenue format guidance |
| Company register | **CRO** company number (6-7 digits) | CRO |
| Peppol EAS | **9935** — Ireland VAT number | Peppol BIS 3.0 EAS codelist |
| Bank | IBAN (IE…), SEPA core, EUR | — |

Confidence: high.

## 4. Chart of accounts

Ireland has **no statutory chart** — the default chart follows the dominant
UK-style QuickBooks/Xero 4-digit convention (1000s assets, 2000s liabilities,
3000s equity, 4000s income, 5000s+ expenses), adapted for Irish VAT control
accounts (2100 VAT on sales / 2110 VAT on purchases / 2120 VAT settlement —
Revenue) and EUR. Mirrors the GB profile's structure (the IE bookkeeping
convention family).

Confidence: high (convention; no statutory source exists).

## 5. Legal forms & fiscal year

- Forms: LTD (private limited), DAC, PLC, unlimited company, sole trader,
  partnership
- Fiscal year: calendar year default (12-31); any date allowed via the ARD
- Small business: VAT-registration thresholds above (below = not registered)

Confidence: high.

## 6. E-invoicing / Peppol

- Peppol participant; no B2B mandate yet (EU e-invoicing directive rolling out
  2027+); Peppol BIS 3.0 UBL accepted (scheme 9935 = VAT number)
- Revenue e-audit / ROS is API-based (no Irish SAF-T)

Confidence: high.

## 7. B-milestones (not registered — strict dispatch fails loudly)

- `tax.returnLayout` — VAT3 9-box return engine (B-milestone)
- `reporting.format` — Companies Act 2014 / FRS 102 accounts (B-milestone)
- `documents.auditFile` — no Irish SAF-T (B-milestone)
- `documents.invoiceCompliance` — **registered: the art. 226 EU baseline**
  ('eu-invoice-vereisten'); s. 108B/113 additions are a B-milestone

## 8. ie.js mapping

| bukio field | Value |
|---|---|
| meta.country / baseCurrency / locale | IE / EUR / en |
| meta.legalForms | ltd, dac, plc, unlimited-company, sole-trader, partnership |
| identifiers.vatIdFormat | /^IE\d{7}[A-Z]{1,2}$/i |
| identifiers.peppolSchemeId | 9935 |
| tax.standardRateBp / codes | 2300 / 23, 13.5, 9, 4.8, 0, V, R, RE, M, P |
| tax.accounts.ledger | [2110 VAT on purchases, 2100 VAT on sales] |
| tax.accounts.fileDefault / differenceDefault | 2120 / 2120 |
| reporting.defaultChart | UK-style, 36 accounts |
| reporting.debtorsAccount / bankAccountDefault | 1100 / 1000 |
| compliance.filingTypes | VAT3 (YYYY-Pn, ie-bimonthly), ANNUAL_ACCOUNTS (YYYY, ie-9-months), CT1 (YYYY, ie-9-months) |
| documents.eInvoicing | peppol-bis-3.0 |
| closing | result 3300 → equity 3200 |
